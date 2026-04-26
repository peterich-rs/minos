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
//! the backend closes the already-opened socket with close code `4401`
//! instead of activating it.
//!
//! # Role default
//!
//! The plan pins `IosClient` as the MVP default when `X-Device-Role` is
//! absent on first registration. For existing rows, the backend trusts the
//! stored role and rejects any mismatching header instead of reclassifying
//! the device from client input.
//!
//! # Cloudflare Access
//!
//! `CF-Access-Client-Id` / `CF-Access-Client-Secret` are validated at the
//! edge. The backend does not re-verify them; the auth helper emits a
//! debug-level log when they are observed so dev builds can confirm header
//! plumbing.
//!
//! # Unpaired-mode gating
//!
//! The HTTP `/v1/pairing/*` routes apply role / state gates per route
//! (see `http::v1::pairing`). On the WebSocket itself the dispatcher only
//! handles `Forward` and `Ingest`: `Forward` synthesises a JSON-RPC "peer
//! offline" response when `session.paired_with` is `None` (the plan's
//! "reject `Forward` entirely" in spirit) and `Ingest` is restricted to
//! the `AgentHost` role.

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

use super::BackendState;
use crate::{
    envelope::run_session,
    http::auth::{self, AuthError, Classification},
    session::{SessionHandle, SessionRegistry},
    store,
};

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
    State(state): State<BackendState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let outcome = auth::authenticate(&state.store, &headers)
        .await
        .map_err(AuthError::into_response_tuple)?;

    let device_id = outcome.device_id;
    let role = outcome.role;
    let device_secret = auth::extract_device_secret(&headers);
    // The header may be missing or malformed; both cases collapse to "no
    // requested role" because pre-upgrade `authenticate` already validated
    // the value when present. We use `.ok().flatten()` so we don't bubble
    // a fresh error for a stale header read.
    let requested_role = auth::extract_device_role(&headers).ok().flatten();

    let (handle, outbox_rx) = SessionHandle::new(device_id, role);

    // Perform the upgrade; `run_session` owns the socket for its lifetime.
    let registry = Arc::clone(&state.registry);
    let store = state.store.clone();
    let translators = Arc::clone(&state.translators);
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

        if let Err(e) = run_session(socket, handle, outbox_rx, registry, store, translators).await {
            tracing::warn!(
                target: "minos_backend::http",
                error = %e,
                device_id = %device_id,
                "run_session exited with error"
            );
        }
    }))
}

#[derive(Debug)]
enum ActivationAuthError {
    Unauthorized(String),
    Internal(String),
}

impl From<AuthError> for ActivationAuthError {
    fn from(value: AuthError) -> Self {
        match value {
            AuthError::Unauthorized(m) => Self::Unauthorized(m),
            AuthError::Internal(m) => Self::Internal(m),
        }
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

    let resolved_role = auth::resolve_device_role(Some(&row), requested_role)?;
    if resolved_role != expected_role {
        return Err(ActivationAuthError::Unauthorized(format!(
            "device role changed during websocket activation: expected {expected_role}, got {resolved_role}"
        )));
    }

    match auth::classify(Some(row), provided_secret)? {
        Classification::FirstConnect => Err(ActivationAuthError::Unauthorized(
            "device row missing during websocket activation".to_string(),
        )),
        Classification::UnpairedExisting => Ok(None),
        Classification::Authenticated => store::pairings::get_pair(store, device_id)
            .await
            .map_err(|e| ActivationAuthError::Internal(e.to_string())),
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

        insert_device(&pool, mac_id, "mac", DeviceRole::AgentHost, 0)
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
            DeviceRole::AgentHost,
            Some(DeviceRole::AgentHost),
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

        insert_device(&pool, id, "mac", DeviceRole::AgentHost, 0)
            .await
            .unwrap();
        store::devices::upsert_secret_hash(&pool, id, &original_hash)
            .await
            .unwrap();

        let existing = store::devices::get_device(&pool, id).await.unwrap();
        let role =
            auth::resolve_device_role(existing.as_ref(), Some(DeviceRole::AgentHost)).unwrap();
        let classification = auth::classify(existing, Some(secret.as_str())).unwrap();
        assert!(matches!(classification, Classification::Authenticated));

        store::devices::upsert_secret_hash(&pool, id, &replacement_hash)
            .await
            .unwrap();

        let err = revalidate_live_session_auth(
            &pool,
            id,
            role,
            Some(DeviceRole::AgentHost),
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

        let (peer_handle, mut peer_outbox_rx) = SessionHandle::new(peer_id, DeviceRole::AgentHost);
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
