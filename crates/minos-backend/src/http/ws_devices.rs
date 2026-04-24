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
//!      handle's `paired_with`, and push `Event::PeerOnline` or
//!      `Event::PeerOffline` as the first server frame based on live
//!      registry membership.
//!    - **Row exists with `secret_hash = NULL`** (e.g. previous Unpaired
//!      handshake by the same device): ignore any provided secret and
//!      re-enter Unpaired mode — the next `Pair` RPC will upsert the hash.
//! 3. `WebSocketUpgrade::on_upgrade(|ws| activate_live_session(...);
//!    run_session(...))`.
//!
//! # Auth failure: HTTP 401 pre-upgrade, 4401 on stale post-upgrade auth
//!
//! This handler still rejects missing / bad credentials **before** the
//! upgrade with an HTTP 401 response — the WS is never opened. However, the
//! `on_upgrade` callback now revalidates the current device row / role /
//! secret before publishing the socket in the live registry. If auth became
//! stale in the gap between the HTTP 101 response and the callback running,
//! the relay closes the already-opened socket with close code `4401`
//! instead of activating it.
//!
//! # Role default
//!
//! The plan pins `IosClient` as the MVP default when `X-Device-Role` is
//! absent on first registration. For existing rows, the relay trusts the
//! stored role and rejects any mismatching header instead of reclassifying
//! the device from client input.
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
    extract::{
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        State,
    },
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
    session::{SessionHandle, SessionRegistry},
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

/// WS close code used when auth changes after the HTTP upgrade succeeds but
/// before the socket is published in the live registry.
const CLOSE_CODE_AUTH_FAILURE: u16 = 4401;

/// WS close code used when activation-time revalidation itself fails.
const CLOSE_CODE_INTERNAL_ERROR: u16 = 1011;

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

    // 2. Role header: parsed if present, resolved against the stored row below.
    let requested_role = extract_device_role(&headers)?;

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

    let role = resolve_device_role(existing.as_ref(), requested_role)?;
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
                target: "minos_backend::http",
                error = %e,
                device_id = %device_id,
                "first-connect insert_device failed (possibly a race)"
            );
        }
    }

    // Build the session handle + receiver now, but defer any live-session
    // side effects until the upgrade callback is actually running. Pairing
    // state is also refreshed there so reconnects use the latest DB view.
    let (handle, outbox_rx) = SessionHandle::new(device_id, role);

    // Perform the upgrade; `run_session` owns the socket for its lifetime.
    let registry = Arc::clone(&state.registry);
    let pairing = Arc::clone(&state.pairing);
    let store = state.store.clone();
    let token_ttl = state.token_ttl;
    Ok(ws.on_upgrade(move |mut socket| async move {
        match revalidate_live_session_auth(
            &store,
            device_id,
            role,
            requested_role,
            device_secret.as_deref(),
        )
        .await
        {
            Ok(paired_with) => {
                *handle.paired_with.write().await = paired_with;
            }
            Err(ActivationAuthError::Unauthorized(message)) => {
                tracing::info!(
                    target: "minos_backend::http",
                    device_id = %device_id,
                    reason = %message,
                    "device auth changed before websocket activation; closing 4401"
                );
                close_socket(&mut socket, CLOSE_CODE_AUTH_FAILURE, "auth_revoked").await;
                return;
            }
            Err(ActivationAuthError::Internal(message)) => {
                tracing::warn!(
                    target: "minos_backend::http",
                    device_id = %device_id,
                    error = %message,
                    "failed to revalidate websocket auth during activation"
                );
                close_socket(
                    &mut socket,
                    CLOSE_CODE_INTERNAL_ERROR,
                    "activation_revalidate_failed",
                )
                .await;
                return;
            }
        }

        activate_live_session(registry.as_ref(), &handle).await;

        if let Err(e) = run_session(
            socket, handle, outbox_rx, registry, pairing, store, token_ttl,
        )
        .await
        {
            tracing::warn!(
                target: "minos_backend::http",
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

#[derive(Debug)]
enum ActivationAuthError {
    Unauthorized(String),
    Internal(String),
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

async fn revalidate_live_session_auth(
    store: &sqlx::SqlitePool,
    device_id: DeviceId,
    expected_role: DeviceRole,
    requested_role: Option<DeviceRole>,
    provided_secret: Option<&str>,
) -> Result<Option<DeviceId>, ActivationAuthError> {
    let row = store::devices::get_device(store, device_id)
        .await
        .map_err(|e| ActivationAuthError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ActivationAuthError::Unauthorized(
                "device row missing during websocket activation".to_string(),
            )
        })?;

    let resolved_role = resolve_device_role(Some(&row), requested_role)
        .map_err(|(_status, message)| ActivationAuthError::Unauthorized(message))?;
    if resolved_role != expected_role {
        return Err(ActivationAuthError::Unauthorized(format!(
            "device role changed during websocket activation: expected {expected_role}, got {resolved_role}"
        )));
    }

    match classify(Some(row), provided_secret).map_err(|(status, message)| {
        if status == StatusCode::UNAUTHORIZED {
            ActivationAuthError::Unauthorized(message)
        } else {
            ActivationAuthError::Internal(message)
        }
    })? {
        Classification::FirstConnect => Err(ActivationAuthError::Unauthorized(
            "device row missing during websocket activation".to_string(),
        )),
        Classification::UnpairedExisting => Ok(None),
        Classification::Authenticated => store::pairings::get_pair(store, device_id)
            .await
            .map_err(|e| ActivationAuthError::Internal(e.to_string())),
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

fn extract_device_role(headers: &HeaderMap) -> Result<Option<DeviceRole>, (StatusCode, String)> {
    let Some(raw) = headers.get(HDR_DEVICE_ROLE) else {
        return Ok(None);
    };
    let s = raw.to_str().map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            "X-Device-Role not UTF-8".to_string(),
        )
    })?;
    DeviceRole::from_str(s).map(Some).map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            format!("X-Device-Role invalid: {e}"),
        )
    })
}

fn resolve_device_role(
    existing: Option<&DeviceRow>,
    requested_role: Option<DeviceRole>,
) -> Result<DeviceRole, (StatusCode, String)> {
    match existing {
        Some(row) => {
            if let Some(role) = requested_role {
                if role != row.role {
                    return Err((
                        StatusCode::UNAUTHORIZED,
                        format!(
                            "X-Device-Role mismatch for existing device: expected {}, got {}",
                            row.role, role
                        ),
                    ));
                }
            }
            Ok(row.role)
        }
        None => Ok(requested_role.unwrap_or(DeviceRole::IosClient)),
    }
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
            target: "minos_backend::http",
            cf_access_client_id_present = cf_id,
            cf_access_client_secret_present = cf_sec,
            "CF-Access headers observed (edge-validated; relay does not re-check)"
        );
    }
}

async fn close_socket(ws: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn activate_live_session(registry: &SessionRegistry, handle: &SessionHandle) {
    let paired_with = *handle.paired_with.read().await;

    // Queue the initial event before publishing this handle in the live
    // registry so it remains the first frame for the new socket.
    let init_frame = Envelope::Event {
        version: 1,
        event: initial_presence_event(paired_with, registry),
    };
    if let Err(e) = handle.outbox.try_send(init_frame) {
        tracing::warn!(
            target: "minos_backend::http",
            error = ?e,
            device_id = %handle.device_id,
            "failed to push initial Event onto outbox"
        );
    }

    // Register only once the upgrade callback is running with the live
    // socket. Reconnects still revoke the prior live session.
    let replaced_existing = if let Some(prev) = registry.insert(handle.clone()) {
        prev.revoke();
        tracing::info!(
            target: "minos_backend::http",
            device_id = %handle.device_id,
            "replaced previous session for device (reconnect)"
        );
        true
    } else {
        false
    };

    notify_live_peer_connected(registry, handle.device_id, paired_with, replaced_existing).await;
}

fn initial_presence_event(paired_with: Option<DeviceId>, registry: &SessionRegistry) -> EventKind {
    match paired_with {
        Some(peer)
            if registry
                .get(peer)
                .is_some_and(|handle| !handle.outbox.is_closed()) =>
        {
            EventKind::PeerOnline {
                peer_device_id: peer,
            }
        }
        Some(peer) => EventKind::PeerOffline {
            peer_device_id: peer,
        },
        None => EventKind::Unpaired,
    }
}

async fn notify_live_peer_connected(
    registry: &SessionRegistry,
    device_id: DeviceId,
    paired_with: Option<DeviceId>,
    replaced_existing: bool,
) {
    if replaced_existing {
        return;
    }
    let Some(peer) = paired_with else {
        return;
    };
    let Some(peer_handle) = registry
        .get(peer)
        .filter(|handle| !handle.outbox.is_closed())
    else {
        return;
    };
    if *peer_handle.paired_with.read().await != Some(device_id) {
        return;
    }

    let frame = Envelope::Event {
        version: 1,
        event: EventKind::PeerOnline {
            peer_device_id: device_id,
        },
    };
    if let Err(e) = peer_handle.outbox.try_send(frame) {
        tracing::warn!(
            target: "minos_backend::http",
            error = ?e,
            device_id = %device_id,
            peer = %peer,
            "failed to push Event::PeerOnline to live peer"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{devices::insert_device, test_support::memory_pool};
    use pretty_assertions::assert_eq;
    use tokio::sync::mpsc::error::TryRecvError;

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
        assert_eq!(extract_device_role(&headers).unwrap(), None);
    }

    #[test]
    fn extract_device_role_kebab_case_parses() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ROLE, "mac-host".parse().unwrap());
        assert_eq!(
            extract_device_role(&headers).unwrap(),
            Some(DeviceRole::MacHost)
        );
    }

    #[test]
    fn extract_device_role_unknown_value_returns_401() {
        let mut headers = HeaderMap::new();
        headers.insert(HDR_DEVICE_ROLE, "gizmo".parse().unwrap());
        let err = extract_device_role(&headers).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn resolve_device_role_first_connect_defaults_to_ios_client() {
        let role = resolve_device_role(None, None).unwrap();
        assert_eq!(role, DeviceRole::IosClient);
    }

    #[test]
    fn resolve_device_role_existing_row_rejects_mismatched_header() {
        let row = DeviceRow {
            device_id: DeviceId::new(),
            display_name: "mac".to_string(),
            role: DeviceRole::MacHost,
            secret_hash: None,
            created_at: 0,
            last_seen_at: 0,
        };

        let err = resolve_device_role(Some(&row), Some(DeviceRole::IosClient)).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        assert!(err.1.contains("mismatch"));
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

    #[tokio::test]
    async fn revalidate_live_session_auth_allows_unpaired_existing_row() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "ios", DeviceRole::IosClient, 0)
            .await
            .unwrap();

        let paired_with =
            revalidate_live_session_auth(&pool, id, DeviceRole::IosClient, None, None)
                .await
                .unwrap();

        assert_eq!(paired_with, None);
    }

    #[tokio::test]
    async fn revalidate_live_session_auth_returns_current_pairing_for_authenticated_reconnect() {
        let pool = memory_pool().await;
        let mac_id = DeviceId::new();
        let ios_id = DeviceId::new();
        let secret = minos_domain::DeviceSecret::generate();
        let hash = crate::pairing::secret::hash_secret(&secret).unwrap();

        insert_device(&pool, mac_id, "mac", DeviceRole::MacHost, 0)
            .await
            .unwrap();
        insert_device(&pool, ios_id, "ios", DeviceRole::IosClient, 0)
            .await
            .unwrap();
        store::devices::upsert_secret_hash(&pool, mac_id, &hash)
            .await
            .unwrap();
        store::pairings::insert_pairing(&pool, mac_id, ios_id, 0)
            .await
            .unwrap();

        let paired_with = revalidate_live_session_auth(
            &pool,
            mac_id,
            DeviceRole::MacHost,
            Some(DeviceRole::MacHost),
            Some(secret.as_str()),
        )
        .await
        .unwrap();

        assert_eq!(paired_with, Some(ios_id));
    }

    #[tokio::test]
    async fn revalidate_live_session_auth_rejects_stale_secret_change() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        let secret = minos_domain::DeviceSecret::generate();
        let original_hash = crate::pairing::secret::hash_secret(&secret).unwrap();
        let replacement = minos_domain::DeviceSecret::generate();
        let replacement_hash = crate::pairing::secret::hash_secret(&replacement).unwrap();

        insert_device(&pool, id, "mac", DeviceRole::MacHost, 0)
            .await
            .unwrap();
        store::devices::upsert_secret_hash(&pool, id, &original_hash)
            .await
            .unwrap();

        let existing = store::devices::get_device(&pool, id).await.unwrap();
        let role = resolve_device_role(existing.as_ref(), Some(DeviceRole::MacHost)).unwrap();
        let classification = classify(existing, Some(secret.as_str())).unwrap();
        assert!(matches!(classification, Classification::Authenticated));

        store::devices::upsert_secret_hash(&pool, id, &replacement_hash)
            .await
            .unwrap();

        let err = revalidate_live_session_auth(
            &pool,
            id,
            role,
            Some(DeviceRole::MacHost),
            Some(secret.as_str()),
        )
        .await
        .unwrap_err();

        match err {
            ActivationAuthError::Unauthorized(message) => {
                assert!(message.contains("does not match"));
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn activate_live_session_registers_only_when_called() {
        let registry = SessionRegistry::new();
        let device_id = DeviceId::new();
        let (handle, mut outbox_rx) = SessionHandle::new(device_id, DeviceRole::IosClient);

        assert!(
            registry.is_empty(),
            "pre-activation handle must not be live"
        );

        activate_live_session(&registry, &handle).await;

        assert!(
            registry.get(device_id).is_some(),
            "activation should register the live session"
        );

        let frame = outbox_rx
            .recv()
            .await
            .expect("initial event should be queued");
        match frame {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => {}
            other => panic!("expected initial Event::Unpaired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn activate_live_session_notifies_peer_once_and_preserves_reconnect_revocation() {
        let registry = SessionRegistry::new();

        let device_id = DeviceId::new();
        let peer_id = DeviceId::new();

        let (peer_handle, mut peer_outbox_rx) = SessionHandle::new(peer_id, DeviceRole::MacHost);
        *peer_handle.paired_with.write().await = Some(device_id);
        registry.insert(peer_handle.clone());

        let (first_handle, mut first_outbox_rx) =
            SessionHandle::new(device_id, DeviceRole::IosClient);
        *first_handle.paired_with.write().await = Some(peer_id);

        activate_live_session(&registry, &first_handle).await;

        let first_frame = first_outbox_rx
            .recv()
            .await
            .expect("first session should receive its initial event");
        match first_frame {
            Envelope::Event {
                event: EventKind::PeerOnline { peer_device_id },
                ..
            } => assert_eq!(peer_device_id, peer_id),
            other => panic!("expected initial Event::PeerOnline, got {other:?}"),
        }

        let peer_frame = peer_outbox_rx
            .recv()
            .await
            .expect("peer should be notified when a fresh live session connects");
        match peer_frame {
            Envelope::Event {
                event: EventKind::PeerOnline { peer_device_id },
                ..
            } => assert_eq!(peer_device_id, device_id),
            other => panic!("expected peer Event::PeerOnline, got {other:?}"),
        }

        let mut revoked = first_handle.subscribe_revocation();
        let (replacement_handle, mut replacement_outbox_rx) =
            SessionHandle::new(device_id, DeviceRole::IosClient);
        *replacement_handle.paired_with.write().await = Some(peer_id);

        activate_live_session(&registry, &replacement_handle).await;

        revoked
            .changed()
            .await
            .expect("replacement should revoke the prior live session");
        assert!(
            *revoked.borrow(),
            "prior live session should be marked revoked"
        );

        let replacement_frame = replacement_outbox_rx
            .recv()
            .await
            .expect("replacement session should receive its initial event");
        match replacement_frame {
            Envelope::Event {
                event: EventKind::PeerOnline { peer_device_id },
                ..
            } => assert_eq!(peer_device_id, peer_id),
            other => panic!("expected replacement initial Event::PeerOnline, got {other:?}"),
        }

        assert_eq!(
            peer_outbox_rx.try_recv(),
            Err(TryRecvError::Empty),
            "reconnect should not emit a duplicate peer-online notification"
        );
    }
}
