//! Outbound WebSocket client of the `minos-backend` broker.
//!
//! The Mac daemon runs exactly one `RelayClient` in steady state. It owns
//! a single background task that:
//!
//!   1. fetches a short-lived ws-ticket via `POST /v1/host/realtime/ws-ticket`;
//!   2. opens a WSS handshake to `/ws/host?ticket=…` (no custom headers);
//!   3. receives `ServerFrame` messages from the topic-based realtime gateway;
//!   4. dispatches `DurableEvent::HostCommandIssued` to the local
//!      [`RpcServerImpl`] and pushes `ClientFrame::HostCommandResult` back;
//!   5. sends `ClientFrame::HostStreamEvent` for agent ingest data.
//!
//! Pairing token issuance and `forget_peer` go through the backend's HTTP
//! `/v1/*` control plane on a separate [`RelayHttpClient`] handle.
//!
//! # Error handling
//!
//! - A connect-time HTTP 401 is treated as a terminal auth failure:
//!   `MinosError::Unauthorized` is written into the shared `last_error` slot
//!   and the task exits with a `Disconnected` link state.
//! - WS close code `4401` is terminal too: `MinosError::DeviceNotTrusted`
//!   lands in `last_error` and the task exits — re-pairing is required.
//! - WS close code `4400` (malformed frame) records
//!   `MinosError::EnvelopeVersionUnsupported` but reconnects.
//! - All other errors fall back to exponential-backoff reconnect
//!   (1s → 2s → 4s → 8s → 16s → 30s cap, no max attempts).

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use futures_util::{SinkExt, StreamExt};
use minos_domain::{DeviceId, DeviceSecret, MinosError, PeerState, RelayLinkState};
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use minos_protocol::HostPeerSummary;
use minos_transport::backoff::delay_for_attempt;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};
use tokio_tungstenite::tungstenite::Error as WsError;

use crate::config::RelayConfig;
use crate::relay_http::RelayHttpClient;
use crate::relay_pairing::{PeerRecord, RelayQrPayload};
use crate::rpc_server::{invoke_host_command, RpcServerImpl};

/// Bounded queue for outbound client frames — deep enough to absorb a brief
/// handshake pause without back-pressuring callers. The dispatch loop
/// drains continuously, so the steady-state depth is effectively zero.
const OUTBOUND_QUEUE_DEPTH: usize = 64;
const RELAY_PING_INTERVAL: Duration = Duration::from_secs(30);
const RELAY_PONG_TIMEOUT: Duration = Duration::from_secs(45);

struct Inner {
    /// Shutdown signal — one-shot, captured behind a `Mutex` so a repeat
    /// `stop()` after the first call is a benign no-op.
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    /// The dispatch task join handle; taken on `stop()`.
    task: Mutex<Option<JoinHandle<()>>>,
    /// The Mac's display name — sent to the backend in `RequestPairingQr`
    /// so the assembled QR carries it through to the iPhone.
    mac_name: String,
    /// Live device secret. Pairing can mint it while this relay client is
    /// already running, so reconnects and `forget_peer` read it through a
    /// shared slot instead of a spawn-time snapshot.
    secret: Arc<StdMutex<Option<DeviceSecret>>>,
    /// HTTP client for the backend's `/v1/*` control plane.
    http: Arc<RelayHttpClient>,
    /// Cloneable producer side of the dispatcher's outbound queue.
    /// Other in-process producers (e.g. the agent-ingest forwarder) clone
    /// this via [`RelayClient::outbound_sender`] so every host-side WS frame
    /// goes through the single socket the dispatcher owns.
    out_tx: mpsc::Sender<ClientFrame>,
}

pub struct RelayClient {
    inner: Arc<Inner>,
}

impl RelayClient {
    /// Spawn the relay-client background task. Returns immediately with a
    /// handle plus two watch receivers the caller can wire into UI.
    ///
    /// The task reconnects forever unless the relay rejects the handshake
    /// with HTTP 401, in which case it exits after broadcasting
    /// `RelayLinkState::Disconnected`. Call [`Self::stop`] to tear the task
    /// down cleanly; the returned `JoinHandle` is awaited internally.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        _config: RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        mac_name: String,
        backend_url: String,
        rpc_server: Option<Arc<RpcServerImpl>>,
        persistence: PersistenceCtx,
    ) -> (
        Arc<Self>,
        watch::Receiver<RelayLinkState>,
        watch::Receiver<PeerState>,
    ) {
        let (link_tx, link_rx) = watch::channel(RelayLinkState::Disconnected);
        let initial_peer = peer
            .as_ref()
            .map_or(PeerState::Unpaired, |p| PeerState::Paired {
                peer_id: p.device_id,
                peer_name: p.name.clone(),
                online: false,
            });
        let (peer_tx, peer_rx) = watch::channel(initial_peer);

        let (out_tx, out_rx) = mpsc::channel::<ClientFrame>(OUTBOUND_QUEUE_DEPTH);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let secret_store = Arc::new(StdMutex::new(secret));
        let http = match RelayHttpClient::new(&backend_url, self_device_id, mac_name.clone()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::error!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    backend_url = %backend_url,
                    "failed to construct RelayHttpClient; pairing/forget HTTP calls will fail",
                );
                Arc::new(
                    RelayHttpClient::new(
                        "ws://invalid.localhost/devices",
                        self_device_id,
                        mac_name.clone(),
                    )
                    .expect("placeholder RelayHttpClient builds against canonical URL"),
                )
            }
        };
        let dispatch_ctx = DispatchCtx {
            self_device_id,
            secret: secret_store.clone(),
            backend_url: backend_url.clone(),
            link_tx,
            peer_tx,
            out_tx: out_tx.clone(),
            out_rx,
            http: http.clone(),
            rpc_server,
            peer_store: persistence.peer_store,
            peers_store: persistence.peers_store,
            last_error: persistence.last_error,
            reconciliator: persistence.reconciliator,
        };

        let task = tokio::spawn(run_dispatch(dispatch_ctx, shutdown_rx));

        let inner = Arc::new(Inner {
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            task: Mutex::new(Some(task)),
            mac_name,
            secret: secret_store,
            http,
            out_tx,
        });

        (Arc::new(Self { inner }), link_rx, peer_rx)
    }

    /// Issue `request_pairing_qr` against the backend's HTTP control plane
    /// and wrap the response into the Mac-side QR payload shape.
    pub async fn request_pairing_token(&self) -> Result<RelayQrPayload, MinosError> {
        let qr = self
            .inner
            .http
            .request_pairing_qr(self.inner.mac_name.clone())
            .await?;

        Ok(RelayQrPayload {
            v: qr.v,
            host_display_name: qr.host_display_name,
            pairing_token: minos_domain::PairingToken(qr.pairing_token),
            expires_at_ms: qr.expires_at_ms,
        })
    }

    /// Back-compat helper for callers that still think in terms of a
    /// single paired device. Deletes the first currently paired row, if
    /// any, via the host-scoped `/v1/me/peers/{mobile_device_id}` route.
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        let secret = self
            .inner
            .secret
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| MinosError::DeviceNotTrusted {
                device_id: "(none)".into(),
            })?;
        let Some(mobile_device_id) = self
            .inner
            .http
            .get_me_peers(&secret)
            .await?
            .into_iter()
            .next()
            .map(|peer| peer.mobile_device_id)
        else {
            return Ok(());
        };
        self.inner
            .http
            .forget_peer_device(&secret, mobile_device_id)
            .await
    }

    /// Issue `DELETE /v1/me/peers/{mobile_device_id}` for one specific
    /// mobile/account row on the current host.
    pub async fn forget_peer_device(&self, mobile_device_id: DeviceId) -> Result<(), MinosError> {
        let secret = self
            .inner
            .secret
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| MinosError::DeviceNotTrusted {
                device_id: "(none)".into(),
            })?;
        self.inner
            .http
            .forget_peer_device(&secret, mobile_device_id)
            .await
    }

    /// Clone the producer side of the dispatcher's outbound queue.
    ///
    /// Used by in-process forwarders (e.g. `agent_ingest`) to push
    /// `ClientFrame::HostStreamEvent` frames through the same socket the
    /// dispatcher owns.
    #[must_use]
    pub fn outbound_sender(&self) -> mpsc::Sender<ClientFrame> {
        self.inner.out_tx.clone()
    }

    /// Signal the dispatch task to exit and await its join. Idempotent:
    /// calling twice is a benign no-op after the first success.
    pub async fn stop(&self) {
        if let Some(tx) = self.inner.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.inner.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

/// Shared persistence handles threaded into the dispatcher.
pub struct PersistenceCtx {
    pub peer_store: Arc<StdMutex<Option<PeerRecord>>>,
    pub peers_store: Arc<StdMutex<Vec<HostPeerSummary>>>,
    pub last_error: Arc<StdMutex<Option<MinosError>>>,
    pub reconciliator: Option<Arc<crate::reconciliator::Reconciliator>>,
}

struct DispatchCtx {
    self_device_id: DeviceId,
    secret: Arc<StdMutex<Option<DeviceSecret>>>,
    backend_url: String,
    link_tx: watch::Sender<RelayLinkState>,
    peer_tx: watch::Sender<PeerState>,
    out_tx: mpsc::Sender<ClientFrame>,
    out_rx: mpsc::Receiver<ClientFrame>,
    http: Arc<RelayHttpClient>,
    rpc_server: Option<Arc<RpcServerImpl>>,
    peer_store: Arc<StdMutex<Option<PeerRecord>>>,
    peers_store: Arc<StdMutex<Vec<HostPeerSummary>>>,
    last_error: Arc<StdMutex<Option<MinosError>>>,
    #[allow(dead_code)]
    reconciliator: Option<Arc<crate::reconciliator::Reconciliator>>,
}

enum CycleOutcome {
    Reconnect,
    AuthFailed,
    Shutdown,
}

async fn run_dispatch(mut ctx: DispatchCtx, mut shutdown_rx: oneshot::Receiver<()>) {
    let mut attempt: u32 = 0;

    loop {
        let _ = ctx.link_tx.send(RelayLinkState::Connecting { attempt });

        let outcome = Box::pin(run_once(&mut ctx, &mut shutdown_rx)).await;

        match outcome {
            CycleOutcome::Shutdown | CycleOutcome::AuthFailed => {
                let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
                return;
            }
            CycleOutcome::Reconnect => {
                attempt = attempt.saturating_add(1);
                let delay = delay_for_attempt(attempt);
                tracing::info!(
                    target: "minos_daemon::relay_client",
                    attempt,
                    delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                    "relay link dropped, backing off before reconnect"
                );
                let _ = ctx.link_tx.send(RelayLinkState::Connecting { attempt });

                tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => {
                        let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
                        return;
                    }
                    () = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

async fn run_once(
    ctx: &mut DispatchCtx,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> CycleOutcome {
    let secret = secret_snapshot_or_reload(&ctx.secret, &ctx.last_error);

    // Fetch a short-lived ws-ticket from the backend.
    let ticket = match &secret {
        Some(s) => match ctx.http.fetch_host_ws_ticket(s).await {
            Ok(resp) => resp.ticket,
            Err(e) => {
                tracing::error!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    "failed to fetch host ws-ticket — treating as auth-failure-equivalent"
                );
                store_last_error(&ctx.last_error, e);
                return CycleOutcome::AuthFailed;
            }
        },
        None => {
            tracing::error!(
                target: "minos_daemon::relay_client",
                "no device secret available — cannot fetch ws-ticket"
            );
            store_last_error(
                &ctx.last_error,
                MinosError::DeviceNotTrusted {
                    device_id: ctx.self_device_id.to_string(),
                },
            );
            return CycleOutcome::AuthFailed;
        }
    };

    let ws_url = build_ws_url(&ctx.backend_url, &ticket);

    let ws = tokio::select! {
        biased;
        _ = &mut *shutdown_rx => return CycleOutcome::Shutdown,
        res = tokio_tungstenite::connect_async(&ws_url) => match res {
            Ok((stream, _resp)) => stream,
            Err(WsError::Http(resp)) if resp.status().as_u16() == 401 => {
                let body = resp
                    .body()
                    .as_ref()
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned);
                let message = body.unwrap_or_else(|| {
                    "relay handshake returned HTTP 401".into()
                });
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    %message,
                    "relay handshake returned HTTP 401 — auth failure, exiting task"
                );
                store_last_error(&ctx.last_error, MinosError::Unauthorized { reason: message });
                return CycleOutcome::AuthFailed;
            }
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    "relay handshake failed; will reconnect with backoff"
                );
                return CycleOutcome::Reconnect;
            }
        }
    };

    let _ = ctx.link_tx.send(RelayLinkState::Connected);
    tracing::info!(target: "minos_daemon::relay_client", "relay link up");
    refresh_peers_from_backend(ctx, secret.as_ref()).await;

    Box::pin(dispatch_loop(ws, ctx, shutdown_rx)).await
}

async fn refresh_peers_from_backend(ctx: &DispatchCtx, secret: Option<&DeviceSecret>) {
    let Some(secret) = secret else {
        apply_peers_snapshot(ctx, Vec::new());
        return;
    };
    match ctx.http.get_me_peers(secret).await {
        Ok(peers) => apply_peers_snapshot(ctx, peers),
        Err(e) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                error = %e,
                "failed to refresh paired peers after relay connect/event",
            );
        }
    }
}

fn apply_peers_snapshot(ctx: &DispatchCtx, peers: Vec<HostPeerSummary>) {
    if let Ok(mut guard) = ctx.peers_store.lock() {
        guard.clone_from(&peers);
    }
    if let Ok(mut guard) = ctx.peer_store.lock() {
        *guard = peers.first().map(peer_record_from_summary);
    }
    let _ = ctx.peer_tx.send(aggregate_peer_state(&peers));
}

fn aggregate_peer_state(peers: &[HostPeerSummary]) -> PeerState {
    let Some(primary) = peers
        .iter()
        .find(|peer| peer.online)
        .or_else(|| peers.first())
    else {
        return PeerState::Unpaired;
    };
    PeerState::Paired {
        peer_id: primary.mobile_device_id,
        peer_name: primary.mobile_device_name.clone(),
        online: primary.online,
    }
}

fn peer_record_from_summary(summary: &HostPeerSummary) -> PeerRecord {
    let paired_at = Utc
        .timestamp_millis_opt(summary.paired_at_ms)
        .single()
        .unwrap_or_else(Utc::now);
    PeerRecord {
        device_id: summary.mobile_device_id,
        name: summary.mobile_device_name.clone(),
        paired_at,
    }
}

/// Inbound + outbound dispatch pump over an upgraded WebSocket.
#[allow(clippy::too_many_lines)]
async fn dispatch_loop(
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ctx: &mut DispatchCtx,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> CycleOutcome {
    let (mut sink, mut stream) = ws.split();
    let mut heartbeat = tokio::time::interval(RELAY_PING_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pong_at = Instant::now();
    heartbeat.tick().await;

    loop {
        tokio::select! {
            biased;
            _ = &mut *shutdown_rx => {
                let _ = sink.send(Message::Close(None)).await;
                return CycleOutcome::Shutdown;
            }
            out = ctx.out_rx.recv() => {
                let Some(frame) = out else {
                    return CycleOutcome::Shutdown;
                };
                let text = match serde_json::to_string(&frame) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!(
                            target: "minos_daemon::relay_client",
                            error = %e,
                            "failed to serialize outbound ClientFrame"
                        );
                        continue;
                    }
                };
                if let Err(e) = sink.send(Message::Text(text.into())).await {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        error = %e,
                        "failed to send outbound frame; reconnecting"
                    );
                    return CycleOutcome::Reconnect;
                }
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_inbound_text(&text, ctx).await {
                            tracing::warn!(
                                target: "minos_daemon::relay_client",
                                error = %e,
                                "failed to handle inbound frame"
                            );
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        if let Err(e) = sink.send(Message::Pong(p)).await {
                            tracing::warn!(
                                target: "minos_daemon::relay_client",
                                error = %e,
                                "failed to send relay pong; reconnecting"
                            );
                            return CycleOutcome::Reconnect;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_pong_at = Instant::now();
                    }
                    Some(Ok(Message::Binary(_) | Message::Frame(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        return classify_close(frame, ctx);
                    }
                    Some(Err(e)) => {
                        tracing::warn!(
                            target: "minos_daemon::relay_client",
                            error = %e,
                            "ws read error; reconnecting"
                        );
                        return CycleOutcome::Reconnect;
                    }
                    None => {
                        tracing::info!(
                            target: "minos_daemon::relay_client",
                            "ws stream ended; reconnecting"
                        );
                        return CycleOutcome::Reconnect;
                    }
                }
            }
            _ = heartbeat.tick() => {
                let elapsed = last_pong_at.elapsed();
                if elapsed > RELAY_PONG_TIMEOUT {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                        "relay pong timeout; reconnecting"
                    );
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code: 1011u16.into(),
                            reason: "ping_timeout".into(),
                        })))
                        .await;
                    return CycleOutcome::Reconnect;
                }
                if let Err(e) = sink.send(Message::Ping(Vec::new().into())).await {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        error = %e,
                        "failed to send relay ping; reconnecting"
                    );
                    return CycleOutcome::Reconnect;
                }
            }
        }
    }
}

/// Parse an inbound text frame as a `ServerFrame` and route it.
async fn handle_inbound_text(text: &str, ctx: &DispatchCtx) -> Result<(), serde_json::Error> {
    let frame: ServerFrame = serde_json::from_str(text)?;
    route_server_frame(frame, ctx).await;
    Ok(())
}

/// Route a parsed `ServerFrame` to the appropriate handler.
async fn route_server_frame(frame: ServerFrame, ctx: &DispatchCtx) {
    match frame {
        ServerFrame::Hello {
            conn_id,
            heartbeat_interval_ms,
            ..
        } => {
            tracing::info!(
                target: "minos_daemon::relay_client",
                conn_id,
                heartbeat_interval_ms,
                "received Hello from realtime gateway"
            );
            // Auto-subscribe to the host topic.
            let host_topic = format!("host:{}", ctx.self_device_id);
            let subscribe = ClientFrame::Subscribe {
                topics: vec![host_topic],
                resume_after: None,
                client_request_id: None,
            };
            if ctx.out_tx.send(subscribe).await.is_err() {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    "failed to enqueue host topic subscribe"
                );
            }
        }
        ServerFrame::SubscribeAck { topics, .. } => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                ?topics,
                "subscription acknowledged"
            );
        }
        ServerFrame::DurableEvent {
            kind, payload, ..
        } => {
            route_durable_event(&kind, &payload, ctx).await;
        }
        ServerFrame::HostForceClose { reason, close_code } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                reason,
                close_code,
                "server force-closed the connection"
            );
        }
        ServerFrame::StreamEvent { kind, topic, .. } => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                kind,
                topic,
                "ignoring stream event on host side"
            );
        }
        ServerFrame::SnapshotRequired { topic, .. } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                topic,
                "snapshot required — host needs full state rebuild"
            );
        }
        ServerFrame::SubscriptionDenied { topic, reason } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                topic,
                reason,
                "subscription denied"
            );
        }
        ServerFrame::SubscriptionLimitExceeded { limit, current } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                limit,
                current,
                "subscription limit exceeded"
            );
        }
        ServerFrame::Pong { .. } => {}
        ServerFrame::Error {
            code,
            message,
            request_id,
        } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                code,
                message,
                request_id,
                "server error frame"
            );
        }
    }
}

/// Route a durable event extracted from a `ServerFrame::DurableEvent`.
async fn route_durable_event(kind: &str, payload: &Value, ctx: &DispatchCtx) {
    match kind {
        "host_command_issued" => {
            let command_id = payload
                .get("command_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let method = payload
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let params = payload
                .get("params")
                .cloned()
                .unwrap_or(Value::Null);

            tracing::debug!(
                target: "minos_daemon::relay_client",
                command_id,
                method,
                "received host command"
            );

            let Some(rpc_server) = ctx.rpc_server.clone() else {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    command_id,
                    "no rpc_server wired — dropping host command"
                );
                return;
            };

            let ack = build_host_command_ack(&command_id, chrono::Utc::now().timestamp_millis());
            let _ = ctx.out_tx.send(ack).await;

            let result = invoke_host_command(&method, params, &rpc_server).await;
            let finished_at_ms = chrono::Utc::now().timestamp_millis();
            let response = build_host_command_result(
                &command_id,
                result.is_ok(),
                result.ok(),
                None,
                finished_at_ms,
            );
            if let Err(e) = ctx.out_tx.send(response).await {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    command_id,
                    "failed to enqueue host command result"
                );
            }
        }
        "host_linked" | "host_unlinked" => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                kind,
                "peer state changed, refreshing"
            );
            let secret = ctx.secret.lock().ok().and_then(|guard| guard.clone());
            refresh_peers_from_backend(ctx, secret.as_ref()).await;
        }
        "host_force_close" => {
            let reason = payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            tracing::warn!(
                target: "minos_daemon::relay_client",
                reason,
                "host force close durable event"
            );
        }
        other => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                kind = other,
                "ignoring durable event on host side"
            );
        }
    }
}

fn classify_close(frame: Option<CloseFrame>, ctx: &DispatchCtx) -> CycleOutcome {
    let code: Option<u16> = frame.as_ref().map(|f| f.code.into());
    let reason: Option<String> = frame
        .as_ref()
        .map(|f| f.reason.to_string())
        .filter(|s| !s.is_empty());
    match code {
        Some(4401) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                code = 4401,
                ?reason,
                "relay closed socket with 4401 — stale device auth, re-pair required"
            );
            store_last_error(
                &ctx.last_error,
                MinosError::DeviceNotTrusted {
                    device_id: ctx.self_device_id.to_string(),
                },
            );
            CycleOutcome::AuthFailed
        }
        Some(4400) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                code = 4400,
                ?reason,
                "relay closed socket with 4400 — frame rejected; will reconnect"
            );
            store_last_error(
                &ctx.last_error,
                MinosError::EnvelopeVersionUnsupported { version: 1 },
            );
            CycleOutcome::Reconnect
        }
        other => {
            tracing::info!(
                target: "minos_daemon::relay_client",
                code = ?other,
                ?reason,
                "relay sent Close; reconnecting"
            );
            CycleOutcome::Reconnect
        }
    }
}

fn store_last_error(slot: &Arc<StdMutex<Option<MinosError>>>, err: MinosError) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(err);
    }
}

fn secret_snapshot(slot: &Arc<StdMutex<Option<DeviceSecret>>>) -> Option<DeviceSecret> {
    slot.lock().ok().and_then(|guard| guard.clone())
}

fn secret_snapshot_or_reload(
    slot: &Arc<StdMutex<Option<DeviceSecret>>>,
    last_error: &Arc<StdMutex<Option<MinosError>>>,
) -> Option<DeviceSecret> {
    if let Some(secret) = secret_snapshot(slot) {
        return Some(secret);
    }

    match crate::device_secret_store::read() {
        Ok(Some(secret)) => {
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(secret.clone());
            }
            tracing::info!(
                target: "minos_daemon::relay_client",
                "reloaded persisted device secret before reconnect"
            );
            Some(secret)
        }
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                error = %error,
                "failed to reload persisted device secret before reconnect"
            );
            store_last_error(last_error, error);
            None
        }
    }
}

// ─── ClientFrame builders ──────────────────────────────────────────────

fn build_host_command_ack(command_id: &str, ack_at_ms: i64) -> ClientFrame {
    ClientFrame::HostCommandAck {
        command_id: command_id.to_string(),
        ack_at_ms,
    }
}

fn build_host_command_result(
    command_id: &str,
    succeeded: bool,
    result: Option<Value>,
    error: Option<Value>,
    finished_at_ms: i64,
) -> ClientFrame {
    ClientFrame::HostCommandResult {
        command_id: command_id.to_string(),
        status: if succeeded {
            "succeeded".into()
        } else {
            "failed".into()
        },
        result,
        error,
        finished_at_ms,
    }
}

/// Build a `ClientFrame::HostStreamEvent` for agent ingest data.
pub fn build_host_stream_event(
    topic: &str,
    kind: &str,
    payload: Value,
) -> ClientFrame {
    ClientFrame::HostStreamEvent {
        topic: topic.to_string(),
        kind: kind.to_string(),
        payload,
    }
}

// ─── URL helpers ───────────────────────────────────────────────────────

fn build_ws_url(base_url: &str, ticket: &str) -> String {
    let ws_url = base_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    format!("{ws_url}/ws/host?ticket={ticket}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn build_ws_url_ws() {
        assert_eq!(
            build_ws_url("ws://127.0.0.1:8787", "ticket-abc"),
            "ws://127.0.0.1:8787/ws/host?ticket=ticket-abc"
        );
    }

    #[test]
    fn build_ws_url_wss() {
        assert_eq!(
            build_ws_url("wss://example.com", "ticket-xyz"),
            "wss://example.com/ws/host?ticket=ticket-xyz"
        );
    }
}
