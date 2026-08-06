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

use crate::approvals::ApprovalService;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures::StreamExt;
use minos_protocol::Envelope;
use tokio::sync::mpsc;

use crate::{
    error::BackendError,
    ingest::use_case::{IngestCommand, IngestUseCase},
    session::{ServerFrame, SessionHandle, SessionRegistry, SessionRevocation},
    store::{AsStorePool, StoreHandle},
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

/// WS close code used when a live session's auth backing was revoked.
const CLOSE_CODE_AUTH_FAILURE: u16 = 4401;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionExit {
    OutboxClosed,
    ClientClose,
    StreamEnded,
    SessionSuperseded,
    AuthRevoked,
    EnvelopeDecode,
    BinaryUnsupported,
    VersionUnsupported,
    ClientSentServerFrame,
    IngestForbiddenRole,
    HeartbeatTimeout { elapsed_ms: u64, limit_ms: u64 },
    ReadError,
    WriteFailed,
}

impl SessionExit {
    fn metric_reason(self) -> &'static str {
        match self {
            Self::OutboxClosed => "outbox_closed",
            Self::ClientClose => "client_close",
            Self::StreamEnded => "stream_ended",
            Self::SessionSuperseded => "session_superseded",
            Self::AuthRevoked => "auth_revoked",
            Self::EnvelopeDecode => "envelope_decode",
            Self::BinaryUnsupported => "binary_unsupported",
            Self::VersionUnsupported => "version_unsupported",
            Self::ClientSentServerFrame => "client_sent_server_frame",
            Self::IngestForbiddenRole => "ingest_forbidden_role",
            Self::HeartbeatTimeout { .. } => "heartbeat_timeout",
            Self::ReadError => "read_error",
            Self::WriteFailed => "write_failed",
        }
    }

    fn close_frame(self) -> Option<(u16, &'static str)> {
        match self {
            Self::SessionSuperseded => Some((CLOSE_CODE_NORMAL, "session_superseded")),
            Self::AuthRevoked => Some((CLOSE_CODE_AUTH_FAILURE, "auth_revoked")),
            Self::EnvelopeDecode => Some((CLOSE_CODE_BAD_REQUEST, "envelope_decode")),
            Self::BinaryUnsupported => Some((CLOSE_CODE_BAD_REQUEST, "binary_unsupported")),
            Self::VersionUnsupported => Some((CLOSE_CODE_BAD_REQUEST, "version_unsupported")),
            Self::ClientSentServerFrame => {
                Some((CLOSE_CODE_BAD_REQUEST, "client_sent_server_frame"))
            }
            Self::IngestForbiddenRole => Some((CLOSE_CODE_BAD_REQUEST, "ingest_forbidden_role")),
            Self::HeartbeatTimeout { .. } => Some((CLOSE_CODE_INTERNAL_ERROR, "heartbeat_timeout")),
            Self::OutboxClosed
            | Self::ClientClose
            | Self::StreamEnded
            | Self::ReadError
            | Self::WriteFailed => None,
        }
    }
}

fn session_exit_for_revocation(session: &SessionHandle, reason: SessionRevocation) -> SessionExit {
    match reason {
        SessionRevocation::Superseded => {
            tracing::info!(
                target: "minos_backend::envelope",
                device = %session.device_id,
                "session superseded by reconnect; closing old socket"
            );
            SessionExit::SessionSuperseded
        }
        SessionRevocation::AuthRevoked => {
            tracing::info!(
                target: "minos_backend::envelope",
                device = %session.device_id,
                "session auth/token revoked; closing socket"
            );
            SessionExit::AuthRevoked
        }
    }
}

struct SessionReader<'a> {
    session: &'a SessionHandle,
    registry: &'a SessionRegistry,
    store: &'a StoreHandle,
    ingest: &'a IngestUseCase,
}

impl<'a> SessionReader<'a> {
    fn new(
        session: &'a SessionHandle,
        registry: &'a SessionRegistry,
        store: &'a StoreHandle,
        ingest: &'a IngestUseCase,
    ) -> Self {
        Self {
            session,
            registry,
            store,
            ingest,
        }
    }

    async fn on_message(
        &self,
        ws: &mut WebSocket,
        maybe_msg: Option<Result<Message, axum::Error>>,
    ) -> Result<(), SessionExit> {
        match maybe_msg {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<Envelope>(&text) {
                Ok(env) => {
                    dispatch_envelope(
                        ws,
                        self.session,
                        self.registry,
                        self.store,
                        self.ingest,
                        env,
                    )
                    .await
                }
                Err(e) => {
                    tracing::warn!(
                        target: "minos_backend::envelope",
                        error = %e,
                        "malformed envelope; closing 4400"
                    );
                    Err(SessionExit::EnvelopeDecode)
                }
            },
            Some(Ok(Message::Binary(_))) => {
                tracing::warn!(
                    target: "minos_backend::envelope",
                    "binary frame rejected; closing 4400"
                );
                Err(SessionExit::BinaryUnsupported)
            }
            Some(Ok(Message::Ping(payload))) => ws
                .send(Message::Pong(payload))
                .await
                .map(|()| ())
                .map_err(|_| SessionExit::WriteFailed),
            Some(Ok(Message::Pong(_))) => {
                *self.session.last_pong_at.write().await = std::time::Instant::now();
                Ok(())
            }
            Some(Ok(Message::Close(_))) => Err(SessionExit::ClientClose),
            None => Err(SessionExit::StreamEnded),
            Some(Err(e)) => {
                tracing::warn!(
                    target: "minos_backend::envelope",
                    error = %e,
                    "ws read error; closing"
                );
                Err(SessionExit::ReadError)
            }
        }
    }
}

struct SessionWriter;

impl SessionWriter {
    async fn send_frame(ws: &mut WebSocket, frame: &ServerFrame) -> Result<(), SessionExit> {
        send_envelope(ws, frame).await
    }
}

struct HeartbeatPolicy {
    limit: Duration,
}

impl HeartbeatPolicy {
    fn new(limit: Duration) -> Self {
        Self { limit }
    }

    async fn on_tick(
        &self,
        ws: &mut WebSocket,
        session: &SessionHandle,
    ) -> Result<(), SessionExit> {
        let elapsed = session.last_pong_at.read().await.elapsed();
        if elapsed > self.limit {
            let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            let limit_ms = u64::try_from(self.limit.as_millis()).unwrap_or(u64::MAX);
            tracing::info!(
                target: "minos_backend::envelope",
                device = %session.device_id,
                elapsed_ms,
                limit_ms,
                "heartbeat timeout; closing 1011"
            );
            return Err(SessionExit::HeartbeatTimeout {
                elapsed_ms,
                limit_ms,
            });
        }

        ws.send(Message::Ping(Vec::new()))
            .await
            .map(|()| ())
            .map_err(|_| SessionExit::WriteFailed)
    }
}

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
    store: StoreHandle,
    approvals: Arc<dyn ApprovalService>,
    ingest: Arc<IngestUseCase>,
) -> Result<(), BackendError> {
    let role_label = role_metric_label(session.role);
    crate::telemetry::record_ws_connect(role_label, crate::telemetry::OUTCOME_OK);
    crate::telemetry::record_session_role_open(role_label);

    let exit = run_session_inner(
        &mut ws,
        &session,
        &mut outbox_rx,
        &registry,
        &store,
        ingest.as_ref(),
    )
    .await;

    finalize_session_exit(&mut ws, &session, exit).await;

    crate::telemetry::record_session_role_close(role_label);

    // Cleanup on any exit path: remove only if this is still the live
    // registry entry. A reconnect may already have replaced it.
    //
    // ADR-0020 / Phase G: comprehensive multi-mac presence broadcast on
    // disconnect is deferred to Phase M. We previously notified the single
    // `paired_with` peer here; that field no longer exists.
    let _ = registry.remove_current(&session);

    if session.role.is_account_client() && store.is_sqlite() {
        if let Some(account_id) = session.account_id() {
            if let Err(error) = approvals
                .resolve_disconnected_for_account(&account_id)
                .await
            {
                tracing::warn!(
                    target: "minos_backend::envelope",
                    error = %error,
                    account_id,
                    "failed to resolve pending approvals after mobile disconnect"
                );
            }
        }
    }

    // Drain remaining outbox so the sender does not block; the receiver
    // goes out of scope right after anyway, but this keeps `Err` paths
    // obviously clean in tracing.
    outbox_rx.close();
    while outbox_rx.recv().await.is_some() {}

    Ok(())
}

/// Inner loop kept separate so `run_session` can guarantee cleanup on
/// every exit arm (including `?` short-circuits).
#[allow(clippy::too_many_lines)] // Central select! loop; splitting obscures the control flow.
async fn run_session_inner(
    ws: &mut WebSocket,
    session: &SessionHandle,
    outbox_rx: &mut mpsc::Receiver<ServerFrame>,
    registry: &SessionRegistry,
    store: &StoreHandle,
    ingest: &IngestUseCase,
) -> SessionExit {
    let mut heartbeat = tokio::time::interval(HEARTBEAT_TICK);
    let mut revocation_rx = session.subscribe_revocation();
    let reader = SessionReader::new(session, registry, store, ingest);
    let heartbeat_policy = HeartbeatPolicy::new(PAIRED_TIMEOUT);
    // First tick fires immediately; skip it so we don't ping right after
    // accepting the socket.
    heartbeat.tick().await;

    loop {
        if let Some(reason) = *revocation_rx.borrow() {
            return session_exit_for_revocation(session, reason);
        }

        tokio::select! {
            biased;

            changed = revocation_rx.changed() => {
                if matches!(changed, Ok(())) {
                    if let Some(reason) = *revocation_rx.borrow_and_update() {
                        return session_exit_for_revocation(session, reason);
                    }
                }
            }

            // Outbound: frame ready for this client.
            maybe_frame = outbox_rx.recv() => {
                let Some(frame) = maybe_frame else {
                    // Outbox sender side has been dropped — shut down.
                    return SessionExit::OutboxClosed;
                };
                if let Err(exit) = SessionWriter::send_frame(ws, &frame).await {
                    return exit;
                }
            }

            // Inbound: message from the client (or socket end).
            maybe_msg = ws.next() => {
                if let Err(exit) = reader.on_message(ws, maybe_msg).await {
                    return exit;
                }
            }

            // Heartbeat: periodic liveness probe + timeout check.
            _ = heartbeat.tick() => {
                // ADR-0020: there is no per-session "paired" boolean
                // anymore. Treat any authenticated session as engaged-class
                // (longer timeout); truly anonymous (FirstConnect) sockets
                // close on the AUTH timeout before reaching the heartbeat
                // path.
                if let Err(exit) = heartbeat_policy.on_tick(ws, session).await {
                    return exit;
                }
            }
        }
    }
}

/// Serialise an envelope and send it as a text frame.
///
/// Returns [`SessionExit::WriteFailed`] if the send failed.
async fn send_envelope(ws: &mut WebSocket, env: &Envelope) -> Result<(), SessionExit> {
    crate::telemetry::record_envelope_out(envelope_kind_label(env), envelope_version(env));
    match serde_json::to_string(env) {
        Ok(json) => ws
            .send(Message::Text(json))
            .await
            .map(|()| ())
            .map_err(|_| SessionExit::WriteFailed),
        Err(e) => {
            tracing::error!(
                target: "minos_backend::envelope",
                error = %e,
                "envelope serialise failed; dropping frame"
            );
            // Serialise failures are internal bugs, not peer problems —
            // keep the socket alive so the next outbound frame has a shot.
            Ok(())
        }
    }
}

fn envelope_kind_label(env: &Envelope) -> &'static str {
    match env {
        Envelope::Forward { .. } => crate::telemetry::KIND_FORWARD,
        Envelope::Forwarded { .. } => crate::telemetry::KIND_FORWARDED,
        Envelope::Event { .. } => crate::telemetry::KIND_EVENT,
        Envelope::Ingest { .. } => crate::telemetry::KIND_INGEST,
    }
}

fn envelope_version(env: &Envelope) -> u8 {
    match env {
        Envelope::Forward { version, .. }
        | Envelope::Forwarded { version, .. }
        | Envelope::Event { version, .. }
        | Envelope::Ingest { version, .. } => *version,
    }
}

/// Dispatch a parsed envelope. Returns `false` to signal "break the loop".
async fn dispatch_envelope(
    ws: &mut WebSocket,
    session: &SessionHandle,
    registry: &SessionRegistry,
    store: &StoreHandle,
    ingest: &IngestUseCase,
    env: Envelope,
) -> Result<(), SessionExit> {
    crate::telemetry::record_envelope_in(envelope_kind_label(&env), envelope_version(&env));
    match env {
        Envelope::Forward {
            version,
            target_device_id,
            payload,
        } => {
            if version != 1 {
                return Err(SessionExit::VersionUnsupported);
            }
            if let Some(back_frame) =
                handle_forward(session, registry, store, target_device_id, payload).await
            {
                send_envelope(ws, &back_frame).await?;
            }
            Ok(())
        }
        // The following two variants are server → client only; a client
        // that sends one is behaving incorrectly. Treat them as malformed
        // and close with 4400, same as an unknown kind.
        Envelope::Forwarded { .. } | Envelope::Event { .. } => {
            tracing::warn!(
                target: "minos_backend::envelope",
                "server-only envelope kind from client; closing 4400"
            );
            Err(SessionExit::ClientSentServerFrame)
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
            session_id,
            seq,
            payload,
            ts_ms,
        } => {
            if version != 1 {
                return Err(SessionExit::VersionUnsupported);
            }
            if session.role != minos_domain::DeviceRole::AgentHost {
                tracing::warn!(
                    target: "minos_backend::envelope",
                    role = ?session.role,
                    "ingest from non-agent-host role; closing 4400"
                );
                return Err(SessionExit::IngestForbiddenRole);
            }
            let command = IngestCommand {
                agent,
                session_id: session_id.clone(),
                seq,
                payload,
                ts_ms,
                owner_device_id: session.device_id,
            };
            if let Err(e) = ingest.execute(command).await {
                tracing::warn!(
                    target: "minos_backend::envelope",
                    error = ?e,
                    session_id = %session_id,
                    seq,
                    "ingest dispatch failed; keeping session open"
                );
            }
            Ok(())
        }
    }
}

async fn finalize_session_exit(ws: &mut WebSocket, session: &SessionHandle, exit: SessionExit) {
    let role = role_metric_label(session.role);
    let close = exit.close_frame();

    match exit {
        SessionExit::HeartbeatTimeout {
            elapsed_ms,
            limit_ms,
        } => {
            tracing::info!(
                target: "minos_backend::envelope",
                device_id = %session.device_id,
                role,
                reason = exit.metric_reason(),
                close_code = ?close.map(|(code, _)| code),
                elapsed_ms,
                limit_ms,
                "websocket session exiting"
            );
        }
        _ => {
            tracing::info!(
                target: "minos_backend::envelope",
                device_id = %session.device_id,
                role,
                reason = exit.metric_reason(),
                close_code = ?close.map(|(code, _)| code),
                "websocket session exiting"
            );
        }
    }

    crate::telemetry::record_ws_close(role, exit.metric_reason());

    if let Some((code, reason)) = close {
        close_with(ws, code, reason).await;
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
    store: &impl AsStorePool,
    target_device_id: minos_domain::DeviceId,
    payload: serde_json::Value,
) -> Option<Envelope> {
    if session.role == minos_domain::DeviceRole::AgentHost {
        if let Some(reply_id) = json_rpc_id(&payload) {
            if crate::host_commands::resolve_pending_host_command(
                store,
                session.device_id,
                reply_id,
                payload.clone(),
            )
            .await
            {
                return None;
            }
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

    let paired = match crate::store::host_links::exists(store, target_device_id, &account_id).await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = %e,
                target = %target_device_id,
                "host_links::exists failed; synthesising peer_offline"
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

/// Stable metric label for a device role — `to_string()` returns
/// kebab-case which keeps the label cardinality matched with the
/// existing wire shape.
pub(crate) fn role_metric_label(role: minos_domain::DeviceRole) -> &'static str {
    match role {
        minos_domain::DeviceRole::AgentHost => "agent-host",
        minos_domain::DeviceRole::MobileClient => "mobile-client",
        minos_domain::DeviceRole::BrowserAdmin => "browser-admin",
        minos_domain::DeviceRole::DesktopConsole => "desktop-console",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::registry::OUTBOX_CAPACITY;
    use crate::store::test_support::{insert_test_host};
    use crate::store::host_links;
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
        insert_test_host(&pool, mac, "Mac", T0).await;
        let ios = insert_ios_device(&pool, &account).await;
        host_links::insert_pair(&pool, mac, &account, ios, T0)
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
        insert_test_host(&pool, mac_id, "Mac", T0).await;
        let ios_a = insert_ios_device(&pool, &account).await;
        let ios_b = insert_ios_device(&pool, &account).await;
        host_links::insert_pair(&pool, mac_id, &account, ios_b, T0)
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
