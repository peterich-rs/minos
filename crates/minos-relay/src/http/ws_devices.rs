//! `GET /devices` — WebSocket upgrade with header-based auth.
//!
//! Implements the handshake defined in plan §9 (in turn derived from spec
//! §7.1/§9.4). The flow is:
//!
//! 1. Parse headers (`X-Device-Id`, optional `X-Device-Role`, optional
//!    `X-Device-Secret`, optional `X-Device-Name`).
//! 2. Look up the device row. Two cases:
//!    - **No row**: insert a fresh row with `secret_hash = NULL`; the
//!      session goes live in Unpaired mode; first server frame is
//!      `Event::Unpaired`.
//!    - **Row exists with `secret_hash`**: require a matching
//!      `X-Device-Secret`; on mismatch reject pre-upgrade with `401`. On
//!      match, look up the paired peer via `pairings::get_pair`, seed the
//!      handle's `paired_with`, and push `Event::PeerOnline` as the first
//!      server frame.
//!    - **Row exists with `secret_hash = NULL`** (e.g. previous Unpaired
//!      handshake by the same device): ignore any provided secret and
//!      re-enter Unpaired mode — the next `Pair` RPC will upsert the hash.
//! 3. `WebSocketUpgrade::on_upgrade(|ws| run_session(...))`.
//!
//! # Auth failure: HTTP 401 pre-upgrade
//!
//! Spec §10.3 reserves close code `4401` for auth failure. This handler
//! rejects missing / bad credentials **before** the upgrade with an HTTP
//! 401 response — the WS is never opened. Close code 4401 is reserved for
//! a future mid-session re-auth path; MVP does not need it, and upgrading
//! just to close immediately wastes a round trip.
//!
//! # Role default
//!
//! The plan pins `IosClient` as the MVP default when `X-Device-Role` is
//! absent. In prod, clients always set the header; the default only
//! accommodates local dev where curl / test scripts may omit it.
//!
//! # Cloudflare Access
//!
//! `CF-Access-Client-Id` / `CF-Access-Client-Secret` are validated at the
//! edge. The relay does not re-verify them; we emit a debug-level log
//! when they are observed so dev builds can confirm header plumbing.
//!
//! # Unpaired-mode gating
//!
//! Step 9 does not introduce any new LocalRpc gating — step 8's dispatcher
//! already enforces the correct behaviour:
//! - `Ping` is always allowed.
//! - `RequestPairingToken` rejects non-`MacHost` callers.
//! - `Pair` rejects if already paired.
//! - `ForgetPeer` rejects if Unpaired.
//! - `Forward` synthesises a JSON-RPC "peer offline" response when
//!   `session.paired_with` is `None`, which is the plan's "reject
//!   `Forward` entirely" in spirit.

use std::sync::Arc;

use axum::{
    extract::{ws::WebSocketUpgrade, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::{Envelope, EventKind};
use std::str::FromStr;
use uuid::Uuid;

use super::RelayState;
use crate::{
    envelope::run_session,
    pairing::secret::verify_secret,
    session::SessionHandle,
    store::{self, devices::DeviceRow},
};

/// `X-Device-Id` — required, UUID v4.
const HDR_DEVICE_ID: &str = "x-device-id";
/// `X-Device-Role` — optional; defaults to [`DeviceRole::IosClient`].
const HDR_DEVICE_ROLE: &str = "x-device-role";
/// `X-Device-Secret` — optional; required when the device row has a hash.
const HDR_DEVICE_SECRET: &str = "x-device-secret";
/// `X-Device-Name` — optional; used for first-connect `display_name`.
const HDR_DEVICE_NAME: &str = "x-device-name";
/// Cloudflare Access client id; logged only.
const HDR_CF_ACCESS_ID: &str = "cf-access-client-id";
/// Cloudflare Access client secret; logged only.
const HDR_CF_ACCESS_SECRET: &str = "cf-access-client-secret";

/// Default `display_name` for devices whose handshake does not send
/// `X-Device-Name`. Spec §8.1 requires NOT NULL; the client can rename on
/// first `Pair` via the `device_name` param.
const DEFAULT_DISPLAY_NAME: &str = "unnamed";

/// `GET /devices` handler: classify auth, then WS upgrade.
///
/// Returns either:
/// - `Err((StatusCode, String))` for pre-upgrade auth failures (the plan's
///   "401" path).
/// - `Ok(Response)` where the response is the WS upgrade carrying
///   [`run_session`] as its post-upgrade callback.
pub async fn upgrade(
    State(state): State<RelayState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    // 1. Device id: required. Plan §9 step 1.
    let device_id = extract_device_id(&headers)?;

    // 2. Role: optional, default IosClient (plan decision; see module docs).
    let role = extract_device_role(&headers)?;

    // 3. Secret: optional; needed iff the stored row carries a hash.
    let device_secret = extract_device_secret(&headers);

    // 4. Display name: optional; defaults to `DEFAULT_DISPLAY_NAME`.
    let display_name = extract_device_name(&headers).unwrap_or(DEFAULT_DISPLAY_NAME.to_string());

    // CF-Access sanity log (no validation — edge handles it).
    log_cf_access_presence(&headers);

    // Look up the device row. Absent row → first connect (Unpaired mode).
    let existing = store::devices::get_device(&state.store, device_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let classification = classify(existing, device_secret.as_deref())?;

    // On first-connect, insert the device row so step-8 handlers can find
    // it. The row carries `secret_hash = NULL` until `Pair` fires.
    if classification.is_first_connect() {
        let now = chrono::Utc::now().timestamp_millis();
        if let Err(e) =
            store::devices::insert_device(&state.store, device_id, &display_name, role, now).await
        {
            // A race here (another handshake inserted in the gap) would
            // manifest as a PK conflict; we log and continue because by
            // the time we upgrade, a row is present either way.
            tracing::warn!(
                target: "minos_relay::http",
                error = %e,
                device_id = %device_id,
                "first-connect insert_device failed (possibly a race)"
            );
        }
    }

    // Determine the initial pairing state for the session handle.
    let paired_with =
        match &classification {
            Classification::FirstConnect | Classification::UnpairedExisting => None,
            Classification::Authenticated => store::pairings::get_pair(&state.store, device_id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        };

    // Build the session handle + receiver; seed paired_with before registering.
    let (handle, outbox_rx) = SessionHandle::new(device_id, role);
    *handle.paired_with.write().await = paired_with;

    // Register; if a prior session existed (reconnect), log the eviction.
    // We don't close the old socket here — the old writer task will observe
    // the receiver's closed state on its next send and exit cleanly.
    if let Some(_prev) = state.registry.insert(handle.clone()) {
        tracing::info!(
            target: "minos_relay::http",
            device_id = %device_id,
            "replaced previous session for device (reconnect)"
        );
    }

    // Push the initial server-side event onto the outbox BEFORE upgrade so
    // the very first frame the client observes after the upgrade reflects
    // state. `try_send` must succeed on a freshly-constructed outbox
    // (capacity 256 > 1), but we log if something goes awry.
    let init_event = match paired_with {
        Some(peer) => EventKind::PeerOnline {
            peer_device_id: peer,
        },
        None => EventKind::Unpaired,
    };
    let init_frame = Envelope::Event {
        version: 1,
        event: init_event,
    };
    if let Err(e) = handle.outbox.try_send(init_frame) {
        tracing::warn!(
            target: "minos_relay::http",
            error = ?e,
            device_id = %device_id,
            "failed to push initial Event onto outbox"
        );
    }

    // Perform the upgrade; `run_session` owns the socket for its lifetime.
    let registry = Arc::clone(&state.registry);
    let pairing = Arc::clone(&state.pairing);
    let store = state.store.clone();
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(e) = run_session(socket, handle, outbox_rx, registry, pairing, store).await {
            tracing::warn!(
                target: "minos_relay::http",
                error = %e,
                device_id = %device_id,
                "run_session exited with error"
            );
        }
    }))
}

// ── classification ───────────────────────────────────────────────────────

/// Three auth outcomes feeding the handshake flow.
#[derive(Debug)]
enum Classification {
    /// No row in `devices` for this id. Insert-then-Unpaired.
    FirstConnect,
    /// Row exists but `secret_hash` is NULL. Re-enter Unpaired without
    /// checking the provided secret.
    UnpairedExisting,
    /// Row exists with a non-null `secret_hash` that we successfully
    /// verified against the provided `X-Device-Secret`.
    Authenticated,
}

impl Classification {
    fn is_first_connect(&self) -> bool {
        matches!(self, Self::FirstConnect)
    }
}

/// Pure decision function: row-state × provided-secret → `Classification`
/// or a 401 tuple. Split out from [`upgrade`] for testability.
fn classify(
    row: Option<DeviceRow>,
    provided_secret: Option<&str>,
) -> Result<Classification, (StatusCode, String)> {
    match row {
        None => Ok(Classification::FirstConnect),
        Some(r) => match r.secret_hash {
            None => Ok(Classification::UnpairedExisting),
            Some(hash) => {
                let Some(secret) = provided_secret else {
                    return Err((
                        StatusCode::UNAUTHORIZED,
                        "X-Device-Secret required for authenticated device".to_string(),
                    ));
                };
                match verify_secret(secret, &hash) {
                    Ok(true) => Ok(Classification::Authenticated),
                    Ok(false) => Err((
                        StatusCode::UNAUTHORIZED,
                        "X-Device-Secret does not match stored hash".to_string(),
                    )),
                    Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
                }
            }
        },
    }
}

// ── header helpers ───────────────────────────────────────────────────────

fn extract_device_id(headers: &HeaderMap) -> Result<DeviceId, (StatusCode, String)> {
    let raw = headers
        .get(HDR_DEVICE_ID)
        .ok_or((StatusCode::UNAUTHORIZED, "X-Device-Id required".to_string()))?;
    let s = raw.to_str().map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            "X-Device-Id not UTF-8".to_string(),
        )
    })?;
    Uuid::parse_str(s).map(DeviceId).map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            format!("X-Device-Id not a valid UUID: {e}"),
        )
    })
}

fn extract_device_role(headers: &HeaderMap) -> Result<DeviceRole, (StatusCode, String)> {
    let Some(raw) = headers.get(HDR_DEVICE_ROLE) else {
        return Ok(DeviceRole::IosClient); // plan's MVP default
    };
    let s = raw.to_str().map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            "X-Device-Role not UTF-8".to_string(),
        )
    })?;
    DeviceRole::from_str(s).map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            format!("X-Device-Role invalid: {e}"),
        )
    })
}

fn extract_device_secret(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HDR_DEVICE_SECRET)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn extract_device_name(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HDR_DEVICE_NAME)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn log_cf_access_presence(headers: &HeaderMap) {
    let cf_id = headers.contains_key(HDR_CF_ACCESS_ID);
    let cf_sec = headers.contains_key(HDR_CF_ACCESS_SECRET);
    if cf_id || cf_sec {
        tracing::debug!(
            target: "minos_relay::http",
            cf_access_client_id_present = cf_id,
            cf_access_client_secret_present = cf_sec,
            "CF-Access headers observed (edge-validated; relay does not re-check)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{devices::insert_device, test_support::memory_pool};
    use pretty_assertions::assert_eq;

    // ── header extraction ─────────────────────────────────────────────

    #[test]
    fn extract_device_id_missing_returns_401() {
        let headers = HeaderMap::new();
        let err = extract_device_id(&headers).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        assert!(err.1.contains("X-Device-Id"));
    }

    #[test]
    fn extract_device_id_non_uuid_returns_401() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ID, "not-a-uuid".parse().unwrap());
        let err = extract_device_id(&headers).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        assert!(err.1.contains("valid UUID"));
    }

    #[test]
    fn extract_device_id_valid_round_trips() {
        let id = DeviceId::new();
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ID, id.to_string().parse().unwrap());
        let got = extract_device_id(&headers).unwrap();
        assert_eq!(got, id);
    }

    #[test]
    fn extract_device_role_absent_defaults_to_ios_client() {
        let headers = HeaderMap::new();
        assert_eq!(
            extract_device_role(&headers).unwrap(),
            DeviceRole::IosClient
        );
    }

    #[test]
    fn extract_device_role_kebab_case_parses() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ROLE, "mac-host".parse().unwrap());
        assert_eq!(extract_device_role(&headers).unwrap(), DeviceRole::MacHost);
    }

    #[test]
    fn extract_device_role_unknown_value_returns_401() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ROLE, "gizmo".parse().unwrap());
        let err = extract_device_role(&headers).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn extract_device_secret_absent_is_none() {
        assert_eq!(extract_device_secret(&HeaderMap::new()), None);
    }

    #[test]
    fn extract_device_secret_present_returns_string() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_SECRET, "sek".parse().unwrap());
        assert_eq!(extract_device_secret(&headers), Some("sek".to_string()));
    }

    // ── classify: pure decision function ──────────────────────────────

    #[test]
    fn classify_no_row_is_first_connect() {
        let out = classify(None, None).unwrap();
        assert!(matches!(out, Classification::FirstConnect));
    }

    #[test]
    fn classify_row_without_hash_is_unpaired_existing() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: None,
            created_at: 0,
            last_seen_at: 0,
        };
        let out = classify(Some(row), None).unwrap();
        assert!(matches!(out, Classification::UnpairedExisting));
    }

    #[test]
    fn classify_row_with_hash_missing_secret_is_401() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$abc$def".to_string()),
            created_at: 0,
            last_seen_at: 0,
        };
        let err = classify(Some(row), None).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        assert!(err.1.contains("X-Device-Secret required"));
    }

    #[tokio::test]
    async fn classify_row_with_hash_matching_secret_is_authenticated() {
        // Produce a real argon2id hash so verify_secret returns Ok(true).
        let plain = minos_domain::DeviceSecret::generate();
        let hash = crate::pairing::secret::hash_secret(&plain).unwrap();
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: Some(hash),
            created_at: 0,
            last_seen_at: 0,
        };
        let out = classify(Some(row), Some(plain.as_str())).unwrap();
        assert!(matches!(out, Classification::Authenticated));
    }

    #[tokio::test]
    async fn classify_row_with_hash_wrong_secret_is_401() {
        let plain = minos_domain::DeviceSecret::generate();
        let hash = crate::pairing::secret::hash_secret(&plain).unwrap();
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "x".to_string(),
            role: DeviceRole::IosClient,
            secret_hash: Some(hash),
            created_at: 0,
            last_seen_at: 0,
        };
        let err = classify(Some(row), Some("wrong-secret")).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        assert!(err.1.contains("does not match"));
    }

    #[tokio::test]
    async fn classify_smoke_via_real_store_row() {
        // End-to-end smoke with a real pool: insert a row, fetch it, feed
        // through `classify` — exercises the DeviceRow shape from sqlx.
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "smoke", DeviceRole::MacHost, 0)
            .await
            .unwrap();
        let row = store::devices::get_device(&pool, id)
            .await
            .unwrap()
            .unwrap();
        let out = classify(Some(row), None).unwrap();
        assert!(matches!(out, Classification::UnpairedExisting));
    }
}
