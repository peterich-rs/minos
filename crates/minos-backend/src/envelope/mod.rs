//! Envelope dispatcher: the per-WebSocket state machine.
//!
//! Once an incoming WS is authenticated (step 9) and a `SessionHandle` is
//! inserted into the `SessionRegistry`, the backend transfers control to
//! [`run_session`]. That function owns the socket for its lifetime and
//! drives three concurrent branches via `tokio::select!`:
//!
//! 1. **Read**: `ws.next()` → decode one [`Envelope`] → dispatch
//!    ([`Forward`] → [`handle_forward`] / [`Ingest`] →
//!    `crate::ingest::dispatch`) → write any synthesised response back.
//! 2. **Write**: drain the `SessionHandle`'s outbox
//!    ([`mpsc::Receiver<Envelope>`]) onto the wire. Anything that
//!    originates server-side (peer forwards, events) lands here.
//! 3. **Heartbeat**: every 15s send a WS `Ping`; if no `Pong` returns
//!    within 90s (single window post ADR-0020 — see [`PAIRED_TIMEOUT`])
//!    close the socket with code 1011 per plan §8.
//!
//! # WS type choice
//!
//! The dispatcher is concrete on `axum::extract::ws::WebSocket`. A mock
//! WS pair for step-8 unit tests would require either a full axum test
//! harness (heavy) or a generic trait gate (intrusive). Per the plan's
//! "recommended simplification", we leave the full loop's e2e coverage to
//! step 12 (which uses a real `tokio-tungstenite::connect_async` against
//! a real axum router). This module's tests cover the PURE handler
//! [`handle_forward`] — which contains the actual business logic; the loop
//! itself is just glue.
//!
//! # Heartbeat policy
//!
//! Matches plan risks §2: bounded per-peer backpressure + liveness.
//!
//! Single 90s window post ADR-0020. The previous Unpaired/Paired split
//! depended on the per-session `paired_with` slot, which is gone — a Mac
//! may be paired to multiple iOS accounts, so there is no single boolean
//! we could derive from the handle alone. Anonymous sockets (no auth)
//! never reach this loop; they're rejected pre-upgrade.
//!
//! `last_pong_at` lives on [`SessionHandle`] and is updated from the read
//! branch when we see a `Pong` frame. The heartbeat branch only reads it.
//!
//! # Cleanup
//!
//! `run_session` removes the handle from the registry before returning,
//! but only if the registry still points at the same concrete session.
//! This keeps reconnect cleanup from evicting a replacement socket for the
//! same `DeviceId`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures::StreamExt;
use minos_protocol::Envelope;
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use crate::{
    error::BackendError,
    ingest::translate::ThreadTranslators,
    session::{ServerFrame, SessionHandle, SessionRegistry},
};

/// Cadence of the heartbeat tick. Spec / plan §8 name 15s as the ping
/// interval; this is the lower of our two timeout windows' granularity.
const HEARTBEAT_TICK: Duration = Duration::from_secs(15);

/// Liveness window for an authenticated session. 90s doesn't fit the
/// `from_mins` helper cleanly; keep the raw secs form for the
/// intermediate value.
///
/// ADR-0020 / Phase G: there is no longer a separate "unpaired" timeout.
/// Multi-mac removed the single `paired_with` slot, so the heartbeat
/// can't decide which window to use from the handle alone. Anonymous
/// sockets close at the auth-handshake step; once we're in this loop we
/// always grant the longer (formerly "paired") window.
const PAIRED_TIMEOUT: Duration = Duration::from_secs(90);

/// WS close code for heartbeat / internal server errors (RFC 6455).
const CLOSE_CODE_INTERNAL_ERROR: u16 = 1011;

/// Standard close code used when a reconnect supersedes an older socket.
const CLOSE_CODE_NORMAL: u16 = 1000;

/// WS close code "Bad Request" — our signal for malformed envelope kinds
/// or unsupported versions (per plan §8).
const CLOSE_CODE_BAD_REQUEST: u16 = 4400;

/// Main per-connection loop.
///
/// Takes ownership of `ws` and the outbox receiver `outbox_rx`, holds the
/// session's `SessionHandle` read-only, and drives the three-branch
/// `select!` until the socket closes, the heartbeat fires, or the peer
/// sends a kind we can't parse.
///
/// # Errors
///
/// Returns `Err(BackendError)` only for the internal book-keeping failures
/// that callers would plausibly surface; normal socket-close paths are
/// `Ok(())`. Step 10 wires a [`From<BackendError>`] into the outer error
/// surface at the axum handler layer.
pub async fn run_session(
    mut ws: WebSocket,
    session: SessionHandle,
    mut outbox_rx: mpsc::Receiver<ServerFrame>,
    registry: Arc<SessionRegistry>,
    store: SqlitePool,
    translators: Arc<ThreadTranslators>,
) -> Result<(), BackendError> {
    let result = run_session_inner(
        &mut ws,
        &session,
        &mut outbox_rx,
        &registry,
        &store,
        &translators,
    )
    .await;

    // Cleanup on any exit path: remove only if this is still the live
    // registry entry. A reconnect may already have replaced it.
    //
    // ADR-0020 / Phase G: comprehensive multi-mac presence broadcast on
    // disconnect is deferred to Phase M. We previously notified the single
    // `paired_with` peer here; that field no longer exists.
    let _ = registry.remove_current(&session);

    // Drain remaining outbox so the sender does not block; the receiver
    // goes out of scope right after anyway, but this keeps `Err` paths
    // obviously clean in tracing.
    outbox_rx.close();
    while outbox_rx.recv().await.is_some() {}

    result
}

/// Inner loop kept separate so `run_session` can guarantee cleanup on
/// every exit arm (including `?` short-circuits).
#[allow(clippy::too_many_lines)] // Central select! loop; splitting obscures the control flow.
async fn run_session_inner(
    ws: &mut WebSocket,
    session: &SessionHandle,
    outbox_rx: &mut mpsc::Receiver<ServerFrame>,
    registry: &SessionRegistry,
    store: &SqlitePool,
    translators: &ThreadTranslators,
) -> Result<(), BackendError> {
    let mut heartbeat = tokio::time::interval(HEARTBEAT_TICK);
    let mut revocation_rx = session.subscribe_revocation();
    // First tick fires immediately; skip it so we don't ping right after
    // accepting the socket.
    heartbeat.tick().await;

    loop {
        if *revocation_rx.borrow() {
            tracing::info!(
                target: "minos_backend::envelope",
                device = %session.device_id,
                "session superseded by reconnect; closing old socket"
            );
            close_with(ws, CLOSE_CODE_NORMAL, "session_superseded").await;
            break;
        }

        tokio::select! {
            biased;

            changed = revocation_rx.changed() => {
                if matches!(changed, Ok(())) && *revocation_rx.borrow_and_update() {
                    tracing::info!(
                        target: "minos_backend::envelope",
                        device = %session.device_id,
                        "session superseded by reconnect; closing old socket"
                    );
                    close_with(ws, CLOSE_CODE_NORMAL, "session_superseded").await;
                    break;
                }
            }

            // Outbound: frame ready for this client.
            maybe_frame = outbox_rx.recv() => {
                let Some(frame) = maybe_frame else {
                    // Outbox sender side has been dropped — shut down.
                    break;
                };
                if !send_envelope(ws, &frame).await {
                    break;
                }
            }

            // Inbound: message from the client (or socket end).
            maybe_msg = ws.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<Envelope>(&text) {
                            Ok(env) => {
                                if !dispatch_envelope(
                                    ws, session, registry, store, translators, env,
                                )
                                .await
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "minos_backend::envelope",
                                    error = %e,
                                    "malformed envelope; closing 4400"
                                );
                                close_with(ws, CLOSE_CODE_BAD_REQUEST, "envelope_decode").await;
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        tracing::warn!(
                            target: "minos_backend::envelope",
                            "binary frame rejected; closing 4400"
                        );
                        close_with(ws, CLOSE_CODE_BAD_REQUEST, "binary_unsupported").await;
                        break;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        // axum auto-replies to control-frame pings if we
                        // do nothing, but being explicit keeps us honest
                        // if the default changes.
                        let _ = ws.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        *session.last_pong_at.write().await = std::time::Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(
                            target: "minos_backend::envelope",
                            error = %e,
                            "ws read error; closing"
                        );
                        break;
                    }
                }
            }

            // Heartbeat: periodic liveness probe + timeout check.
            _ = heartbeat.tick() => {
                let elapsed = session.last_pong_at.read().await.elapsed();
                // ADR-0020: there is no per-session "paired" boolean
                // anymore. Treat any authenticated session as engaged-class
                // (longer timeout); truly anonymous (FirstConnect) sockets
                // close on the AUTH timeout before reaching the heartbeat
                // path.
                let limit = PAIRED_TIMEOUT;

                if elapsed > limit {
                    tracing::info!(
                        target: "minos_backend::envelope",
                        device = %session.device_id,
                        elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                        limit_ms = u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
                        "heartbeat timeout; closing 1011"
                    );
                    close_with(ws, CLOSE_CODE_INTERNAL_ERROR, "heartbeat_timeout").await;
                    break;
                }
                let _ = ws.send(Message::Ping(Vec::new())).await;
            }
        }
    }

    Ok(())
}

/// Serialise an envelope and send it as a text frame.
///
/// Returns `false` if the send failed (caller breaks out of the loop).
async fn send_envelope(ws: &mut WebSocket, env: &Envelope) -> bool {
    match serde_json::to_string(env) {
        Ok(json) => ws.send(Message::Text(json)).await.is_ok(),
        Err(e) => {
            tracing::error!(
                target: "minos_backend::envelope",
                error = %e,
                "envelope serialise failed; dropping frame"
            );
            // Serialise failures are internal bugs, not peer problems —
            // keep the socket alive so the next outbound frame has a shot.
            true
        }
    }
}

/// Dispatch a parsed envelope. Returns `false` to signal "break the loop".
async fn dispatch_envelope(
    ws: &mut WebSocket,
    session: &SessionHandle,
    registry: &SessionRegistry,
    store: &SqlitePool,
    translators: &ThreadTranslators,
    env: Envelope,
) -> bool {
    match env {
        Envelope::Forward {
            version,
            target_device_id,
            payload,
        } => {
            if version != 1 {
                close_with(ws, CLOSE_CODE_BAD_REQUEST, "version_unsupported").await;
                return false;
            }
            if let Some(back_frame) =
                handle_forward(session, registry, store, target_device_id, payload).await
            {
                return send_envelope(ws, &back_frame).await;
            }
            true
        }
        // The following two variants are server → client only; a client
        // that sends one is behaving incorrectly. Treat them as malformed
        // and close with 4400, same as an unknown kind.
        Envelope::Forwarded { .. } | Envelope::Event { .. } => {
            tracing::warn!(
                target: "minos_backend::envelope",
                "server-only envelope kind from client; closing 4400"
            );
            close_with(ws, CLOSE_CODE_BAD_REQUEST, "client_sent_server_frame").await;
            false
        }
        // Host → backend raw event stream. Only agent-host role is
        // permitted; anyone else is a protocol violation and the socket
        // closes 4400. The dispatch itself is crash-safe: translator errors
        // surface as synthetic UI-event frames, DB errors surface as
        // BackendError and drop the event (with a warn log) but keep the
        // session alive.
        Envelope::Ingest {
            version,
            agent,
            thread_id,
            seq,
            payload,
            ts_ms,
        } => {
            if version != 1 {
                close_with(ws, CLOSE_CODE_BAD_REQUEST, "version_unsupported").await;
                return false;
            }
            if session.role != minos_domain::DeviceRole::AgentHost {
                tracing::warn!(
                    target: "minos_backend::envelope",
                    role = ?session.role,
                    "ingest from non-agent-host role; closing 4400"
                );
                close_with(ws, CLOSE_CODE_BAD_REQUEST, "ingest_forbidden_role").await;
                return false;
            }
            if let Err(e) = crate::ingest::dispatch(
                store,
                registry,
                translators,
                agent,
                &thread_id,
                seq,
                &payload,
                ts_ms,
                session.device_id,
            )
            .await
            {
                tracing::warn!(
                    target: "minos_backend::envelope",
                    error = ?e,
                    thread_id = %thread_id,
                    seq,
                    "ingest dispatch failed; keeping session open"
                );
            }
            true
        }
    }
}

/// Handle a `Forward` envelope by routing it (or synthesising a peer-
/// offline JSON-RPC error if the peer is not present).
///
/// - Returns `None` when the payload was routed via the registry; the
///   caller does nothing.
/// - Returns `Some(Envelope::Forwarded{..})` carrying a synthesised
///   JSON-RPC error when the peer is offline; caller sends it back to the
///   sender (spec §7.3 `(*)` note).
///
/// Post ADR-0020 / Phase G: `target_device_id` is stamped on the wire by
/// the iOS sender (a single Mac it wants to reach). For Mac-side replies
/// the same field carries the originating iOS device id; the backend
/// double-checks the request_id → requester mapping first so legacy
/// reply-only flows that don't yet stamp `target_device_id` keep working.
pub async fn handle_forward(
    session: &SessionHandle,
    registry: &SessionRegistry,
    store: &SqlitePool,
    target_device_id: minos_domain::DeviceId,
    payload: serde_json::Value,
) -> Option<Envelope> {
    if session.role == minos_domain::DeviceRole::AgentHost {
        if let Some(reply_id) = json_rpc_id(&payload) {
            // Try the reply-target mapping first; if found, prefer it
            // over the wire-stamped target so legacy reply-only flows
            // still work and so daemons that don't yet stamp
            // `target_device_id` on replies stay routable.
            if let Some(target) = session.take_rpc_reply_target(reply_id) {
                return route_or_synth(session, registry, target, payload).await;
            }
        }
        // Mac-initiated forward to a specific iOS device. The route()
        // helper's account-mismatch gate (registry.rs) already enforces
        // same-account.
        return route_or_synth(session, registry, target_device_id, payload).await;
    }

    // iOS→Mac path: validate that target_device_id is paired to the iOS
    // caller's account.
    let Some(account_id) = session.account_id() else {
        tracing::warn!(
            target: "minos_backend::envelope",
            device = %session.device_id,
            "iOS forward without account_id; synthesising peer_offline"
        );
        return Some(synth_peer_offline_forwarded(session.device_id, &payload));
    };

    let paired = match crate::store::account_mac_pairings::exists(
        store,
        target_device_id,
        &account_id,
    )
    .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = %e,
                target = %target_device_id,
                "account_mac_pairings::exists failed; synthesising peer_offline"
            );
            return Some(synth_peer_offline_forwarded(session.device_id, &payload));
        }
    };
    if !paired {
        tracing::warn!(
            target: "minos_backend::envelope",
            device = %session.device_id,
            target = %target_device_id,
            "iOS forward to unpaired Mac; synthesising peer_offline"
        );
        return Some(synth_peer_offline_forwarded(session.device_id, &payload));
    }

    // Stamp the reply correlation: when the Mac replies with the same
    // jsonrpc id, the AgentHost branch above resolves it back to this
    // sender via take_rpc_reply_target.
    if let Some(request_id) = json_rpc_id(&payload) {
        if let Some(peer_handle) = registry.get(target_device_id) {
            peer_handle.remember_rpc_reply_target(request_id, session.device_id);
        }
    }

    route_or_synth(session, registry, target_device_id, payload).await
}

/// Route `payload` from `session` to `target` via the registry,
/// translating the routing error variants we care about into synthesised
/// `Forwarded` JSON-RPC errors so the caller can ship them back to the
/// sender.
async fn route_or_synth(
    session: &SessionHandle,
    registry: &SessionRegistry,
    target: minos_domain::DeviceId,
    payload: serde_json::Value,
) -> Option<Envelope> {
    match registry
        .route(session.device_id, target, payload.clone())
        .await
    {
        Ok(()) => None,
        Err(BackendError::PeerOffline { .. }) => {
            Some(synth_peer_offline_forwarded(session.device_id, &payload))
        }
        Err(BackendError::PeerBackpressure { .. }) => Some(synth_peer_backpressure_forwarded(
            session.device_id,
            &payload,
        )),
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = %e,
                target = %target,
                "forward route failed"
            );
            Some(synth_peer_offline_forwarded(session.device_id, &payload))
        }
    }
}

fn json_rpc_id(payload: &serde_json::Value) -> Option<u64> {
    payload.get("id").and_then(serde_json::Value::as_u64)
}

/// Synthesise a JSON-RPC 2.0 "peer offline" error response (spec §7.3 `(*)`).
///
/// The caller's `Forward.payload` is expected to look like a JSON-RPC
/// request; we copy its `id` across so the caller's jsonrpsee client can
/// correlate. If the inbound payload is malformed (no `id`), we emit
/// `"id": null` per JSON-RPC 2.0 rules.
fn synth_peer_offline_forwarded(
    from: minos_domain::DeviceId,
    orig_payload: &serde_json::Value,
) -> Envelope {
    synth_forward_error(from, orig_payload, -32001, "peer offline")
}

fn synth_peer_backpressure_forwarded(
    from: minos_domain::DeviceId,
    orig_payload: &serde_json::Value,
) -> Envelope {
    synth_forward_error(from, orig_payload, -32002, "peer backpressure")
}

fn synth_forward_error(
    from: minos_domain::DeviceId,
    orig_payload: &serde_json::Value,
    code: i64,
    message: &'static str,
) -> Envelope {
    let id = orig_payload
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let err_payload = serde_json::json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": message,
        },
        "id": id,
    });
    Envelope::Forwarded {
        version: 1,
        from,
        payload: err_payload,
    }
}

/// Send a WS Close frame with the given code and reason, best-effort.
async fn close_with(ws: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::registry::OUTBOX_CAPACITY;
    use crate::store::account_mac_pairings;
    use crate::store::devices::insert_device;
    use crate::store::test_support::{insert_account, insert_ios_device, memory_pool, T0};
    use minos_domain::{DeviceId, DeviceRole};
    use pretty_assertions::assert_eq;

    /// Shared fixture: an account, an iOS device on it, and a Mac
    /// already paired to the account. Returns (pool, account_id, mac_id,
    /// ios_id).
    async fn paired_fixture() -> (sqlx::SqlitePool, String, DeviceId, DeviceId) {
        let pool = memory_pool().await;
        let account = insert_account(&pool, "user@example.com").await;
        let mac = DeviceId::new();
        insert_device(&pool, mac, "Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        let ios = insert_ios_device(&pool, &account).await;
        account_mac_pairings::insert_pair(&pool, mac, &account, ios, T0)
            .await
            .unwrap();
        (pool, account, mac, ios)
    }

    // ── handle_forward: peer offline synthesises JSON-RPC error ───────

    #[tokio::test]
    async fn handle_forward_peer_offline_synthesizes_jsonrpc_error() {
        // Mac is paired in DB but no live session in the registry → offline.
        let (pool, account, mac, ios) = paired_fixture().await;
        let registry = SessionRegistry::new();
        let (session, _rx) = SessionHandle::new(ios, DeviceRole::MobileClient);
        session.set_account_id(account);

        let orig = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "list_clis",
            "id": 42,
            "params": {},
        });
        let back = handle_forward(&session, &registry, &pool, mac, orig).await;
        let env = back.expect("must synthesise Forwarded error");
        match env {
            Envelope::Forwarded {
                version,
                from,
                payload,
            } => {
                assert_eq!(version, 1);
                assert_eq!(from, ios);
                assert_eq!(payload["jsonrpc"], "2.0");
                assert_eq!(payload["error"]["code"], -32001);
                assert_eq!(payload["error"]["message"], "peer offline");
                assert_eq!(payload["id"], 42);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_forward_unpaired_synthesizes_jsonrpc_error_with_null_id() {
        // No pairing row → exists() returns false → synth peer_offline.
        let pool = memory_pool().await;
        let account = insert_account(&pool, "user@example.com").await;
        let ios = insert_ios_device(&pool, &account).await;
        let mac_target = DeviceId::new(); // never paired
        let registry = SessionRegistry::new();
        let (session, _rx) = SessionHandle::new(ios, DeviceRole::MobileClient);
        session.set_account_id(account);

        // Payload with no `id` key → synthesised id must be null.
        let orig = serde_json::json!({"method": "bogus"});
        let back = handle_forward(&session, &registry, &pool, mac_target, orig).await;
        let env = back.expect("must synthesise Forwarded error");
        match env {
            Envelope::Forwarded { payload, .. } => {
                assert!(payload["id"].is_null(), "id must be JSON null");
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_forward_ios_without_account_synthesizes_peer_offline() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let sender_id = DeviceId::new();
        let target = DeviceId::new();
        let (session, _rx) = SessionHandle::new(sender_id, DeviceRole::MobileClient);
        // No set_account_id → handler bails with peer_offline.

        let orig = serde_json::json!({"method": "x", "id": 1});
        let back = handle_forward(&session, &registry, &pool, target, orig).await;
        let env = back.expect("missing account_id forces peer_offline");
        match env {
            Envelope::Forwarded { from, payload, .. } => {
                assert_eq!(from, sender_id);
                assert_eq!(payload["error"]["code"], -32001);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    // ── handle_forward: happy path ────────────────────────────────────

    #[tokio::test]
    async fn handle_forward_happy_path_routes_via_registry() {
        let (pool, account, mac, ios) = paired_fixture().await;
        let registry = SessionRegistry::new();

        let (ha, _rxa) = SessionHandle::new(ios, DeviceRole::MobileClient);
        let (hb, mut rxb) = SessionHandle::new(mac, DeviceRole::AgentHost);
        ha.set_account_id(account.clone());
        hb.set_account_id(account);
        registry.insert(ha.clone());
        registry.insert(hb.clone());

        let payload = serde_json::json!({"jsonrpc": "2.0", "method": "ping", "id": 1});
        let back = handle_forward(&ha, &registry, &pool, mac, payload.clone()).await;
        assert!(
            back.is_none(),
            "happy path returns None; peer got the frame"
        );

        let frame = rxb.recv().await.expect("peer must receive forwarded frame");
        match frame {
            Envelope::Forwarded {
                version,
                from,
                payload: p,
            } => {
                assert_eq!(version, 1);
                assert_eq!(from, ios);
                assert_eq!(p, payload);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_forward_routes_mac_reply_to_original_requester_by_jsonrpc_id() {
        // One Mac, two iOS clients on the same account; both pair the
        // Mac. iOS-B sends a request, the Mac replies with the same id —
        // the reply must reach iOS-B (the original requester) via the
        // remembered rpc_reply_target, not iOS-A.
        let pool = memory_pool().await;
        let account = insert_account(&pool, "user@example.com").await;
        let mac_id = DeviceId::new();
        insert_device(&pool, mac_id, "Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        let ios_a = insert_ios_device(&pool, &account).await;
        let ios_b = insert_ios_device(&pool, &account).await;
        account_mac_pairings::insert_pair(&pool, mac_id, &account, ios_b, T0)
            .await
            .unwrap();

        let registry = SessionRegistry::new();
        let (mac, _mac_rx) = SessionHandle::new(mac_id, DeviceRole::AgentHost);
        let (a, _a_rx) = SessionHandle::new(ios_a, DeviceRole::MobileClient);
        let (b, mut b_rx) = SessionHandle::new(ios_b, DeviceRole::MobileClient);
        mac.set_account_id(account.clone());
        a.set_account_id(account.clone());
        b.set_account_id(account);
        registry.insert(mac.clone());
        registry.insert(a);
        registry.insert(b.clone());

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "minos_health",
            "params": {},
        });
        let back = handle_forward(&b, &registry, &pool, mac_id, request).await;
        assert!(back.is_none());

        let reply = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "result": {"ok": true},
        });
        // The Mac's reply: `target_device_id` here is intentionally
        // *wrong* (points at ios_a) so we prove the reply-target mapping
        // wins over the wire-stamped target. This protects against
        // legacy daemons that don't yet stamp `target_device_id` on
        // replies.
        let back = handle_forward(&mac, &registry, &pool, ios_a, reply.clone()).await;
        assert!(back.is_none());

        let frame = b_rx.recv().await.expect("ios_b receives the reply");
        match frame {
            Envelope::Forwarded { from, payload, .. } => {
                assert_eq!(from, mac_id);
                assert_eq!(payload, reply);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_forward_full_outbox_synthesizes_jsonrpc_backpressure_error() {
        let (pool, account, mac, ios) = paired_fixture().await;
        let registry = SessionRegistry::new();
        let (ha, _rxa) = SessionHandle::new(ios, DeviceRole::MobileClient);
        let (hb, _rxb) = SessionHandle::new(mac, DeviceRole::AgentHost);
        ha.set_account_id(account.clone());
        hb.set_account_id(account);
        registry.insert(ha.clone());
        registry.insert(hb);

        for id in 0..OUTBOX_CAPACITY {
            registry
                .route(
                    ios,
                    mac,
                    serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "fill"}),
                )
                .await
                .expect("fill routes must succeed before the outbox is full");
        }

        let payload = serde_json::json!({"jsonrpc": "2.0", "method": "ping", "id": 2});
        let back = handle_forward(&ha, &registry, &pool, mac, payload).await;
        let env = back.expect("full outbox must synthesize a retryable error");
        match env {
            Envelope::Forwarded { from, payload, .. } => {
                assert_eq!(from, ios);
                assert_eq!(payload["error"]["code"], -32002);
                assert_eq!(payload["error"]["message"], "peer backpressure");
                assert_eq!(payload["id"], 2);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    // ── synth helper: shape sanity ────────────────────────────────────

    #[test]
    fn synth_peer_offline_carries_jsonrpc_2_0_envelope() {
        let from = DeviceId::new();
        let env = synth_peer_offline_forwarded(
            from,
            &serde_json::json!({"id": 7, "jsonrpc": "2.0", "method": "x"}),
        );
        match env {
            Envelope::Forwarded {
                version,
                from: f,
                payload,
            } => {
                assert_eq!(version, 1);
                assert_eq!(f, from);
                assert_eq!(payload["jsonrpc"], "2.0");
                assert_eq!(payload["error"]["code"], -32001);
                assert_eq!(payload["id"], 7);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    #[test]
    fn synth_peer_backpressure_carries_jsonrpc_2_0_envelope() {
        let from = DeviceId::new();
        let env = synth_peer_backpressure_forwarded(
            from,
            &serde_json::json!({"id": 9, "jsonrpc": "2.0", "method": "x"}),
        );
        match env {
            Envelope::Forwarded {
                version,
                from: f,
                payload,
            } => {
                assert_eq!(version, 1);
                assert_eq!(f, from);
                assert_eq!(payload["jsonrpc"], "2.0");
                assert_eq!(payload["error"]["code"], -32002);
                assert_eq!(payload["error"]["message"], "peer backpressure");
                assert_eq!(payload["id"], 9);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }
}
