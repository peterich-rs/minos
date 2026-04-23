//! Local-RPC dispatch for the four backend-terminated methods.
//!
//! The WebSocket dispatcher (`envelope::mod`) decodes an incoming
//! [`Envelope::LocalRpc`] frame and hands the (id, method, params) triple
//! to [`handle`]. Each method maps to a single async fn on
//! [`LocalRpcContext`]; the master [`handle`] is a thin match over
//! [`LocalRpcMethod`] that returns a [`LocalRpcOutcome`] for the dispatcher
//! to wrap back up into an [`Envelope::LocalRpcResponse`].
//!
//! # Method menu (spec §6.1)
//!
//! | Method | Role gate | Pre-state | Notes |
//! |---|---|---|---|
//! | `Ping` | any | any | returns `{"ok": true}` verbatim |
//! | `RequestPairingToken` | `mac-host` | any | mints 5-minute token |
//! | `Pair` | any | unpaired | consumes token, emits `Event::Paired` to issuer |
//! | `ForgetPeer` | any | paired | emits `Event::Unpaired` to both sides |
//!
//! # Error code strings
//!
//! Snake-case, stable across releases; clients match on these:
//!
//! - `"unauthorized"` — role gate violated (e.g. ios-client asking for a token).
//! - `"pairing_token_invalid"` — unknown/expired/consumed token.
//! - `"pairing_state_mismatch"` — already paired / not paired for the method.
//! - `"internal"` — any underlying [`RelayError::StoreQuery`] etc. Caller
//!   should log and retry / escalate.
//!
//! # Peer-event delivery (§10.2 R4)
//!
//! After a successful `pair`, we push `Event::Paired` onto the issuer's
//! outbox via `try_send`. Failure to deliver is logged at warn-level but
//! does not roll back the DB state — if the issuer's socket went away mid-
//! pair, the DB is already consistent and the next reconnect will fetch
//! the secret via the normal auth handshake (spec §7.2 / §9.4).

use std::time::Duration;

use minos_domain::{DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind, LocalRpcMethod, LocalRpcOutcome, RpcError};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    error::RelayError,
    pairing::PairingService,
    session::{SessionHandle, SessionRegistry},
    store::devices,
};

/// Token TTL matches spec §6.1 "5 minutes".
const PAIRING_TOKEN_TTL: Duration = Duration::from_mins(5);

/// Read-only context threaded into every per-method handler.
///
/// Reference-only so the dispatcher can build a fresh context per inbound
/// frame without cloning `Arc`s on the hot path.
pub struct LocalRpcContext<'a> {
    pub session: &'a SessionHandle,
    pub registry: &'a SessionRegistry,
    pub pairing: &'a PairingService,
    pub store: &'a SqlitePool,
}

/// Dispatch a decoded [`Envelope::LocalRpc`] to the right handler.
///
/// `id` is echoed back by the caller; this fn returns only the outcome.
pub async fn handle(
    ctx: &LocalRpcContext<'_>,
    method: &LocalRpcMethod,
    params: &serde_json::Value,
) -> LocalRpcOutcome {
    match method {
        LocalRpcMethod::Ping => handle_ping().await,
        LocalRpcMethod::RequestPairingToken => handle_request_pairing_token(ctx).await,
        LocalRpcMethod::Pair => handle_pair(ctx, params).await,
        LocalRpcMethod::ForgetPeer => handle_forget_peer(ctx).await,
    }
}

/// Build an error outcome with a machine-readable code + human message.
///
/// Pulled out so the set of valid codes is visible at a glance, and to
/// guarantee every `Err` goes through one funnel (easier to audit).
pub(crate) fn err(code: &str, message: impl Into<String>) -> LocalRpcOutcome {
    LocalRpcOutcome::Err {
        error: RpcError {
            code: code.to_string(),
            message: message.into(),
        },
    }
}

/// Always returns `{"ok": true}`; see spec §6.1.
///
/// `async` is kept for symmetry with the other handlers — the master
/// [`handle`] awaits them uniformly, and the trivial body lets this
/// compile to a zero-cost future.
#[allow(clippy::unused_async)]
async fn handle_ping() -> LocalRpcOutcome {
    LocalRpcOutcome::Ok {
        result: serde_json::json!({"ok": true}),
    }
}

/// `request_pairing_token`: mac-host only; mints a fresh 5-minute token.
///
/// Spec §6.1 gates the caller's role. `device_name` is not a parameter —
/// the mac already has a row in `devices` by the time this fires (inserted
/// on handshake).
async fn handle_request_pairing_token(ctx: &LocalRpcContext<'_>) -> LocalRpcOutcome {
    if ctx.session.role != DeviceRole::MacHost {
        return err("unauthorized", "only mac-host may request pairing tokens");
    }

    match ctx
        .pairing
        .request_token(ctx.session.device_id, PAIRING_TOKEN_TTL)
        .await
    {
        Ok((token, expires_at)) => LocalRpcOutcome::Ok {
            result: serde_json::json!({
                "token": token.as_str(),
                "expires_at": expires_at.to_rfc3339(),
            }),
        },
        Err(e) => {
            tracing::warn!(
                target: "minos_relay::envelope",
                error = %e,
                "request_pairing_token failed"
            );
            err("internal", e.to_string())
        }
    }
}

#[derive(Debug, Deserialize)]
struct PairParams {
    token: String,
    device_name: String,
}

/// `pair`: consume a token, mint two DeviceSecrets, notify the issuer.
///
/// Guards:
/// - reject if this session already reports a peer (must go through
///   `forget_peer` first, spec §10.2 R4).
/// - reject invalid/expired/consumed tokens → `pairing_token_invalid`.
/// - reject already-paired peer → `pairing_state_mismatch`.
///
/// On success:
/// 1. update the consumer session's `paired_with` slot.
/// 2. look up the issuer's display name for the consumer's response.
/// 3. push `Event::Paired` to the issuer's outbox (if live) and update
///    its `paired_with` slot.
/// 4. return the consumer's `{peer_device_id, peer_name, your_device_secret}`
///    payload per spec §7.1 step 11.
async fn handle_pair(ctx: &LocalRpcContext<'_>, params: &serde_json::Value) -> LocalRpcOutcome {
    {
        // Read lock scoped so we don't hold it across the DB round-trip.
        if ctx.session.paired_with.read().await.is_some() {
            return err(
                "pairing_state_mismatch",
                "session is already paired; call forget_peer first",
            );
        }
    }

    let PairParams { token, device_name } =
        match serde_json::from_value::<PairParams>(params.clone()) {
            Ok(p) => p,
            Err(e) => {
                return err("invalid_params", format!("pair params: {e}"));
            }
        };

    let candidate = minos_domain::PairingToken(token);

    let outcome = match ctx
        .pairing
        .consume_token(&candidate, ctx.session.device_id, device_name.clone())
        .await
    {
        Ok(o) => o,
        Err(RelayError::PairingTokenInvalid) => {
            return err(
                "pairing_token_invalid",
                "pairing token is unknown, expired, or already consumed",
            );
        }
        Err(RelayError::PairingStateMismatch { actual }) => {
            return err(
                "pairing_state_mismatch",
                format!("peer already paired (state: {actual})"),
            );
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_relay::envelope",
                error = %e,
                "pair consume_token failed"
            );
            return err("internal", e.to_string());
        }
    };

    let issuer = outcome.issuer_device_id;

    // Update this session's pairing state to reflect the new peer.
    *ctx.session.paired_with.write().await = Some(issuer);

    // Fetch issuer's display name for the consumer-side response payload.
    // Falls back to "Mac" if the row is somehow missing (shouldn't happen
    // post-consume_token, but defensive).
    let mac_name = match devices::get_device(ctx.store, issuer).await {
        Ok(Some(row)) => row.display_name,
        Ok(None) => {
            tracing::warn!(
                target: "minos_relay::envelope",
                issuer = %issuer,
                "pair succeeded but issuer device row missing"
            );
            "Mac".to_string()
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_relay::envelope",
                error = %e,
                issuer = %issuer,
                "pair: lookup of issuer display_name failed"
            );
            "Mac".to_string()
        }
    };

    // Push Event::Paired to the issuer. Best-effort: if the issuer's
    // socket dropped between request_pairing_token and pair, the event
    // fails silently — the issuer's next reconnect will observe the pair
    // via the handshake path instead.
    if let Some(issuer_handle) = ctx.registry.get(issuer) {
        let frame = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: ctx.session.device_id,
                peer_name: device_name.clone(),
                your_device_secret: DeviceSecret(outcome.issuer_secret.as_str().to_string()),
            },
        };
        if let Err(e) = issuer_handle.outbox.try_send(frame) {
            tracing::warn!(
                target: "minos_relay::envelope",
                error = ?e,
                issuer = %issuer,
                "failed to push Event::Paired to issuer outbox"
            );
        }
        // Mirror the pair update on the issuer's handle so routes from
        // this consumer side find the peer immediately.
        *issuer_handle.paired_with.write().await = Some(ctx.session.device_id);
    }

    LocalRpcOutcome::Ok {
        result: serde_json::json!({
            "peer_device_id": issuer,
            "peer_name": mac_name,
            "your_device_secret": outcome.consumer_secret.as_str(),
        }),
    }
}

/// `forget_peer`: tear down the pairing, notify both sides.
///
/// Guards: reject if unpaired (nothing to forget). On success, emit
/// `Event::Unpaired` to this session's outbox and the peer's (if live),
/// and clear both `paired_with` slots.
async fn handle_forget_peer(ctx: &LocalRpcContext<'_>) -> LocalRpcOutcome {
    // Snapshot pairing state under a short read lock so we don't hold it
    // across the DB round-trip.
    let peer = {
        let guard = ctx.session.paired_with.read().await;
        *guard
    };
    let Some(peer) = peer else {
        return err(
            "pairing_state_mismatch",
            "session is not paired; nothing to forget",
        );
    };

    match ctx.pairing.forget_pair(ctx.session.device_id).await {
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                target: "minos_relay::envelope",
                error = %e,
                "forget_pair failed"
            );
            return err("internal", e.to_string());
        }
    }

    // Clear our side first so a concurrent route() sees no peer.
    *ctx.session.paired_with.write().await = None;

    // Push Event::Unpaired to both sides. Own side may already have its
    // UI wired via the LocalRpcResponse we're about to return, but the
    // wire contract (spec §7.4) says both sides receive the event — so
    // push it explicitly and let the client decide how to reconcile.
    let unpaired = Envelope::Event {
        version: 1,
        event: EventKind::Unpaired,
    };
    if let Err(e) = ctx.session.outbox.try_send(unpaired.clone()) {
        tracing::warn!(
            target: "minos_relay::envelope",
            error = ?e,
            device = %ctx.session.device_id,
            "failed to push Event::Unpaired to self"
        );
    }

    if let Some(peer_handle) = ctx.registry.get(peer) {
        if let Err(e) = peer_handle.outbox.try_send(unpaired) {
            tracing::warn!(
                target: "minos_relay::envelope",
                error = ?e,
                peer = %peer,
                "failed to push Event::Unpaired to peer"
            );
        }
        *peer_handle.paired_with.write().await = None;
    }

    LocalRpcOutcome::Ok {
        result: serde_json::json!({"ok": true}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{devices, test_support::memory_pool};
    use minos_domain::{DeviceId, DeviceRole};
    use pretty_assertions::assert_eq;

    // ── small helpers ─────────────────────────────────────────────────

    /// Insert a devices row and return its id; mimics the handshake path
    /// (step 9) that creates the row on WS upgrade.
    async fn insert_device_row(pool: &SqlitePool, name: &str, role: DeviceRole) -> DeviceId {
        let id = DeviceId::new();
        devices::insert_device(pool, id, name, role, 0)
            .await
            .unwrap();
        id
    }

    /// Build a [`SessionHandle`] + receiver for the given id/role. Mirrors
    /// what step 9's WS accept will do; scoped to tests.
    fn make_session(
        id: DeviceId,
        role: DeviceRole,
    ) -> (SessionHandle, tokio::sync::mpsc::Receiver<Envelope>) {
        SessionHandle::new(id, role)
    }

    // ── ping ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn handle_ping_returns_ok_ok_true() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let mac = insert_device_row(&pool, "mac", DeviceRole::MacHost).await;
        let (session, _rx) = make_session(mac, DeviceRole::MacHost);
        let ctx = LocalRpcContext {
            session: &session,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };

        let out = handle(&ctx, &LocalRpcMethod::Ping, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Ok { result } => assert_eq!(result, serde_json::json!({"ok": true})),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    // ── request_pairing_token ─────────────────────────────────────────

    #[tokio::test]
    async fn request_pairing_token_rejects_ios_client() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = LocalRpcContext {
            session: &session,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };

        let out = handle(
            &ctx,
            &LocalRpcMethod::RequestPairingToken,
            &serde_json::json!({}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "unauthorized");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_pairing_token_happy_path_returns_token_and_expires_at() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let mac = insert_device_row(&pool, "mac", DeviceRole::MacHost).await;
        let (session, _rx) = make_session(mac, DeviceRole::MacHost);
        let ctx = LocalRpcContext {
            session: &session,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };

        let out = handle(
            &ctx,
            &LocalRpcMethod::RequestPairingToken,
            &serde_json::json!({}),
        )
        .await;
        match out {
            LocalRpcOutcome::Ok { result } => {
                assert!(result["token"].is_string());
                // Plaintext token is a base64url-style string ≥ 32 chars.
                let tok = result["token"].as_str().unwrap();
                assert!(tok.len() >= 32, "token too short: {tok:?}");
                let expires = result["expires_at"].as_str().expect("expires_at string");
                // RFC3339 parse round-trip sanity check.
                let _: chrono::DateTime<chrono::Utc> =
                    expires.parse().expect("expires_at is RFC3339");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    // ── pair ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn pair_rejects_if_already_paired() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        // Pre-seed pairing state: session already has a peer.
        *session.paired_with.write().await = Some(DeviceId::new());
        let ctx = LocalRpcContext {
            session: &session,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": "x", "device_name": "y"}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "pairing_state_mismatch");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pair_rejects_on_invalid_token() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = LocalRpcContext {
            session: &session,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": "never-issued", "device_name": "iphone"}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "pairing_token_invalid");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pair_happy_path_returns_peer_info_and_pushes_event_to_issuer() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        // Mac issuer: registered + live session in the registry.
        let mac = insert_device_row(&pool, "Fan's Mac", DeviceRole::MacHost).await;
        let (mac_handle, mut mac_rx) = make_session(mac, DeviceRole::MacHost);
        registry.insert(mac_handle.clone());

        // iOS consumer: separate session.
        let ios = DeviceId::new();
        let (ios_handle, _ios_rx) = make_session(ios, DeviceRole::IosClient);
        registry.insert(ios_handle.clone());

        // Mac issues a token (via the service directly; bypasses the
        // role-gated handler for test brevity).
        let (token, _exp) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        let ctx = LocalRpcContext {
            session: &ios_handle,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": token.as_str(), "device_name": "Fan's iPhone"}),
        )
        .await;

        match out {
            LocalRpcOutcome::Ok { result } => {
                assert_eq!(result["peer_device_id"], serde_json::json!(mac));
                assert_eq!(result["peer_name"], "Fan's Mac");
                assert!(result["your_device_secret"].is_string());
            }
            other => panic!("expected Ok, got {other:?}"),
        }

        // iOS session now reflects the pairing.
        assert_eq!(*ios_handle.paired_with.read().await, Some(mac));
        // Mac session likewise (we mirrored it when pushing Paired).
        assert_eq!(*mac_handle.paired_with.read().await, Some(ios));

        // Mac's outbox received Event::Paired carrying issuer secret.
        let frame = mac_rx.recv().await.expect("Mac must receive Event::Paired");
        match frame {
            Envelope::Event { version, event } => {
                assert_eq!(version, 1);
                match event {
                    EventKind::Paired {
                        peer_device_id,
                        peer_name,
                        your_device_secret,
                    } => {
                        assert_eq!(peer_device_id, ios);
                        assert_eq!(peer_name, "Fan's iPhone");
                        assert!(!your_device_secret.as_str().is_empty());
                    }
                    other => panic!("expected Paired, got {other:?}"),
                }
            }
            other => panic!("expected Event envelope, got {other:?}"),
        }
    }

    // ── forget_peer ───────────────────────────────────────────────────

    #[tokio::test]
    async fn forget_peer_rejects_if_unpaired() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = LocalRpcContext {
            session: &session,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };

        let out = handle(&ctx, &LocalRpcMethod::ForgetPeer, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "pairing_state_mismatch");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forget_peer_pushes_unpaired_event_to_both_sides() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        // Fully wire a pair: Mac issues token, iPhone consumes it.
        let mac = insert_device_row(&pool, "mac", DeviceRole::MacHost).await;
        let (mac_handle, mut mac_rx) = make_session(mac, DeviceRole::MacHost);
        registry.insert(mac_handle.clone());

        let ios = DeviceId::new();
        let (ios_handle, mut ios_rx) = make_session(ios, DeviceRole::IosClient);
        registry.insert(ios_handle.clone());

        let (token, _) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        // Run the pair handler so both `paired_with` slots and the DB are
        // in the post-pair state. Drain the Mac's Event::Paired from the
        // inbox so the subsequent Unpaired is the next event.
        {
            let ctx = LocalRpcContext {
                session: &ios_handle,
                registry: &registry,
                pairing: &pairing,
                store: &pool,
            };
            let _ = handle(
                &ctx,
                &LocalRpcMethod::Pair,
                &serde_json::json!({"token": token.as_str(), "device_name": "iphone"}),
            )
            .await;
            // Drop the Event::Paired so the assertion below sees Unpaired
            // first.
            let _paired = mac_rx.recv().await;
        }

        // Now call forget_peer from the Mac's side.
        let ctx = LocalRpcContext {
            session: &mac_handle,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
        };
        let out = handle(&ctx, &LocalRpcMethod::ForgetPeer, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Ok { result } => {
                assert_eq!(result, serde_json::json!({"ok": true}));
            }
            other => panic!("expected Ok, got {other:?}"),
        }

        // Both paired_with slots cleared.
        assert_eq!(*mac_handle.paired_with.read().await, None);
        assert_eq!(*ios_handle.paired_with.read().await, None);

        // Mac (the caller) received Event::Unpaired.
        let self_event = mac_rx.recv().await.expect("Mac must get Unpaired");
        match self_event {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => {}
            other => panic!("expected Unpaired, got {other:?}"),
        }
        // iOS peer also got Event::Unpaired.
        let peer_event = ios_rx.recv().await.expect("iPhone must get Unpaired");
        match peer_event {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => {}
            other => panic!("expected Unpaired, got {other:?}"),
        }
    }
}
