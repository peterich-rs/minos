//! Outbound WebSocket client of the `minos-backend` broker.
//!
//! The Mac daemon runs exactly one `RelayClient` in steady state. It owns
//! a single background task that:
//!
//!   1. opens a WSS handshake to the compile-time `BACKEND_URL`, stamping
//!      the full auth-header bundle from [`minos_transport::auth::AuthHeaders`];
//!   2. publishes relay-link transitions onto a `watch::Receiver<RelayLinkState>`
//!      so UI can react to connect/disconnect / reconnect attempts;
//!   3. publishes peer-state transitions (`Paired` → `PeerOnline` → `PeerOffline`
//!      / `Unpaired`) onto a `watch::Receiver<PeerState>`;
//!   4. relays `Envelope::Forwarded` peer payloads to the local jsonrpsee
//!      surface (`RpcServerImpl`) and pushes the response back over the
//!      WebSocket so the iPhone sees a paired-RPC round trip.
//!
//! Pairing token issuance and `forget_peer` go through the backend's HTTP
//! `/v1/*` control plane on a separate [`RelayHttpClient`] handle; this
//! module no longer carries an in-flight `LocalRpc` correlation table.
//!
//! The module intentionally does NOT touch `DaemonHandle`. Plan 05 Phase F
//! wires the handle to this client; Phase E (this module) only has to stand
//! on its own with the `relay_client_smoke` integration tests.
//!
//! # Error handling
//!
//! - A connect-time HTTP 401 (CF Access or relay's pre-upgrade auth check)
//!   is unambiguously an auth failure: `MinosError::CfAuthFailed` is written
//!   into the shared `last_error` slot and the task exits with a
//!   `Disconnected` link state. The caller must call [`RelayClient::stop`]
//!   and spawn a fresh client once creds have been rotated.
//! - WS close code `4401` (relay's post-upgrade stale-auth signal) is
//!   terminal too: `MinosError::DeviceNotTrusted` lands in `last_error`
//!   and the task exits — re-pairing is required before another connect
//!   can succeed. Close code `4400` (malformed envelope / version mismatch)
//!   records `MinosError::EnvelopeVersionUnsupported` but falls back to
//!   the reconnect backoff, since a bug fix in-flight may re-establish the
//!   link on the next cycle.
//! - All other errors fall back to exponential-backoff reconnect
//!   (1s → 2s → 4s → 8s → 16s → 30s cap, no max attempts).
//!
//! # Pairing state on `EventKind::Paired`
//!
//! Pairing is now process-local on the host. When the relay finalises a
//! pair and forwards us `your_device_secret`, the dispatch task updates its
//! in-memory peer mirror and publishes `PeerState::Paired`, but it no longer
//! writes `device-secret` to the macOS Keychain or saves `local-state.json`.

use std::sync::{Arc, Mutex as StdMutex};

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret, MinosError, PeerState, RelayLinkState};
use minos_protocol::envelope::{Envelope, EventKind};
use minos_transport::auth::{AuthHeaders, CfAccessToken};
use minos_transport::backoff::delay_for_attempt;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};
use tokio_tungstenite::tungstenite::Error as WsError;

use crate::config::RelayConfig;
use crate::relay_http::RelayHttpClient;
use crate::relay_pairing::{PeerRecord, RelayQrPayload};
use crate::rpc_server::{invoke_forwarded, wrap_response_envelope, RpcServerImpl};

/// Bounded queue for outbound envelopes — deep enough to absorb a brief
/// handshake pause without back-pressuring callers. The dispatch loop
/// drains continuously, so the steady-state depth is effectively zero.
const OUTBOUND_QUEUE_DEPTH: usize = 64;

struct Inner {
    /// Shutdown signal — one-shot, captured behind a `Mutex` so a repeat
    /// `stop()` after the first call is a benign no-op.
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    /// The dispatch task join handle; taken on `stop()`.
    task: Mutex<Option<JoinHandle<()>>>,
    /// The Mac's display name — sent to the backend in `RequestPairingQr`
    /// so the assembled QR carries it through to the iPhone.
    mac_name: String,
    /// Spawn-time snapshot of the device secret. Used by [`Self::forget_peer`]
    /// to authenticate the HTTP `DELETE /v1/pairing` call. `None` until the
    /// daemon respawns the client with a fresh secret post-pairing.
    secret: Option<DeviceSecret>,
    /// HTTP client for the backend's `/v1/*` control plane.
    http: Arc<RelayHttpClient>,
    /// Cloneable producer side of the dispatcher's outbound queue.
    /// Other in-process producers (e.g. the agent-ingest forwarder) clone
    /// this via [`RelayClient::outbound_sender`] so every host-side WS frame
    /// goes through the single `/devices` socket the dispatcher owns —
    /// preventing the device-id collision that two parallel WS clients
    /// would otherwise trigger on the backend.
    out_tx: mpsc::Sender<Envelope>,
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
        config: RelayConfig,
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
                // We haven't connected yet — the relay will emit PeerOnline
                // or PeerOffline inside the first authenticated frame.
                online: false,
            });
        let (peer_tx, peer_rx) = watch::channel(initial_peer);

        let (out_tx, out_rx) = mpsc::channel::<Envelope>(OUTBOUND_QUEUE_DEPTH);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let inner_secret = secret.clone();
        let http = match RelayHttpClient::new(
            &backend_url,
            self_device_id,
            mac_name.clone(),
            config.clone(),
        ) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::error!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    backend_url = %backend_url,
                    "failed to construct RelayHttpClient; pairing/forget HTTP calls will fail",
                );
                // Build a placeholder client against the same URL — every
                // attempt will surface the same error path.
                Arc::new(
                    RelayHttpClient::new(
                        "ws://invalid.localhost/devices",
                        self_device_id,
                        mac_name.clone(),
                        config.clone(),
                    )
                    .expect("placeholder RelayHttpClient builds against canonical URL"),
                )
            }
        };
        let dispatch_ctx = DispatchCtx {
            config,
            self_device_id,
            secret,
            mac_name: mac_name.clone(),
            backend_url: backend_url.clone(),
            link_tx,
            peer_tx,
            out_tx: out_tx.clone(),
            out_rx,
            rpc_server,
            peer_store: persistence.peer_store,
            last_error: persistence.last_error,
        };

        let task = tokio::spawn(run_dispatch(dispatch_ctx, shutdown_rx));

        let inner = Arc::new(Inner {
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            task: Mutex::new(Some(task)),
            mac_name,
            secret: inner_secret,
            http,
            out_tx,
        });

        (Arc::new(Self { inner }), link_rx, peer_rx)
    }

    /// Issue `request_pairing_qr` against the backend's HTTP control plane
    /// and wrap the response into the Mac-side QR payload shape.
    ///
    /// The QR carries only the host display name, the one-shot token, and
    /// its expiry. Backend URL and any CF Access edge credentials live in
    /// the mobile client's compile-time build config — never in the QR.
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

    /// Issue `DELETE /v1/pairing` against the backend. The backend then
    /// emits `Event::Unpaired` to the live WS, which the dispatch loop
    /// pushes onto the peer-state watch channel — callers do NOT need to
    /// await that event here.
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        let secret = self
            .inner
            .secret
            .clone()
            .ok_or_else(|| MinosError::DeviceNotTrusted {
                device_id: "(none)".into(),
            })?;
        self.inner.http.forget_pairing(&secret).await
    }

    /// Clone the producer side of the dispatcher's outbound queue.
    ///
    /// Used by in-process forwarders (e.g. `agent_ingest`) to push
    /// `Envelope::Ingest` / `Envelope::Forward` frames through the same
    /// `/devices` socket the dispatcher owns. Centralising on a single WS
    /// per device is required by the backend, whose session registry is
    /// keyed by `DeviceId` alone and revokes any prior socket on
    /// reconnect — two parallel WS clients with the same identity would
    /// supersede each other in a tight loop.
    #[must_use]
    pub fn outbound_sender(&self) -> mpsc::Sender<Envelope> {
        self.inner.out_tx.clone()
    }

    /// Signal the dispatch task to exit and await its join. Idempotent:
    /// calling twice is a benign no-op after the first success.
    pub async fn stop(&self) {
        // Take the shutdown sender once; drop it if already taken.
        if let Some(tx) = self.inner.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.inner.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

/// Shared persistence handles threaded into the dispatcher.
///
/// `DaemonInner` owns the same `Arc`s; the dispatch task updates them when
/// the relay forwards a `Paired` / `Unpaired` event so warm restarts pick
/// up the most recent peer + error state.
pub struct PersistenceCtx {
    /// In-memory mirror of the persisted `PeerRecord` — same `Arc` as the
    /// one in `DaemonInner::peer`. Updated on `EventKind::Paired` /
    /// `Unpaired`; read by `DaemonHandle::current_trusted_device`.
    pub peer_store: Arc<StdMutex<Option<PeerRecord>>>,
    /// Same `Arc` as `DaemonInner::last_error`. Populated on fatal-exit
    /// paths (HTTP 401, WS close 4401/4400). Drained on `DaemonHandle::
    /// last_error`.
    pub last_error: Arc<StdMutex<Option<MinosError>>>,
}

/// Shared state plumbed into the dispatcher. Built once at spawn time; the
/// dispatcher holds it until shutdown.
///
/// `shutdown_rx` lives as a sibling variable in `run_dispatch` instead of a
/// field so `tokio::select!` can borrow it independently of `&mut ctx`.
struct DispatchCtx {
    config: RelayConfig,
    self_device_id: DeviceId,
    secret: Option<DeviceSecret>,
    mac_name: String,
    backend_url: String,
    link_tx: watch::Sender<RelayLinkState>,
    peer_tx: watch::Sender<PeerState>,
    /// Producer side of the outbound queue. Held alongside `out_rx` so
    /// inbound dispatch (e.g. forwarded RPC responses) can push frames
    /// without going through the public [`RelayClient`] handle.
    out_tx: mpsc::Sender<Envelope>,
    out_rx: mpsc::Receiver<Envelope>,
    /// Local jsonrpsee surface invoked when the relay delivers an
    /// `Envelope::Forwarded`. `None` in tests that don't exercise the
    /// peer-RPC path; production wires the daemon's `RpcServerImpl` here.
    rpc_server: Option<Arc<RpcServerImpl>>,
    /// Shared with `DaemonInner::peer`. Updated on every `EventKind::Paired`
    /// / `Unpaired` so warm reads via `current_trusted_device` see the
    /// newest record without round-tripping the watch channel.
    peer_store: Arc<StdMutex<Option<PeerRecord>>>,
    /// One-shot fatal-error signal drained by `DaemonHandle::last_error`.
    last_error: Arc<StdMutex<Option<MinosError>>>,
}

/// Outcome of a single connect-attempt cycle. Drives the outer
/// connect → dispatch → reconnect loop.
enum CycleOutcome {
    /// Either a WS error, a clean close from the relay, or a server-shutdown
    /// event. Back off and retry.
    Reconnect,
    /// Handshake rejected with HTTP 401 (CF Access or the relay's own
    /// pre-upgrade auth check). Fatal — exit the task so the caller can
    /// rotate creds and spawn a new client.
    AuthFailed,
    /// External `stop()` signal. Exit cleanly without notifying further.
    Shutdown,
}

/// Background task body. Runs the connect → dispatch → reconnect loop
/// until signaled to exit via `shutdown_rx` or a fatal auth failure.
///
/// Shutdown is polled inside each inner awaitable — `run_once` (which
/// races the connect handshake and dispatch loop against shutdown) and
/// the backoff sleep — so the outer loop never holds a second borrow of
/// `shutdown_rx`.
async fn run_dispatch(mut ctx: DispatchCtx, mut shutdown_rx: oneshot::Receiver<()>) {
    let mut attempt: u32 = 0;

    loop {
        // Announce the intent to connect (or reconnect). The caller's UI
        // reads this to show a spinner and surface the retry count.
        let _ = ctx.link_tx.send(RelayLinkState::Connecting { attempt });

        let outcome = run_once(&mut ctx, &mut shutdown_rx).await;

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

/// One connect + dispatch cycle. Returns `Reconnect` on any transport-level
/// failure, `AuthFailed` on a pre-upgrade HTTP 401, and `Shutdown` when the
/// outer `shutdown_rx` fires mid-cycle.
async fn run_once(ctx: &mut DispatchCtx, shutdown_rx: &mut oneshot::Receiver<()>) -> CycleOutcome {
    let headers = build_headers(
        &ctx.config,
        ctx.self_device_id,
        ctx.secret.as_ref(),
        &ctx.mac_name,
    );
    let request = match build_request(&ctx.backend_url, &headers) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                target: "minos_daemon::relay_client",
                error = %e,
                "invalid backend URL — treating as auth-failure-equivalent"
            );
            store_last_error(
                &ctx.last_error,
                MinosError::ConnectFailed {
                    url: ctx.backend_url.clone(),
                    message: e.to_string(),
                },
            );
            return CycleOutcome::AuthFailed;
        }
    };

    let ws = tokio::select! {
        biased;
        _ = &mut *shutdown_rx => return CycleOutcome::Shutdown,
        res = tokio_tungstenite::connect_async(request) => match res {
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
                    "relay handshake returned HTTP 401 (CF Access or relay pre-upgrade check)".into()
                });
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    %message,
                    "relay handshake returned HTTP 401 — auth failure, exiting task"
                );
                store_last_error(&ctx.last_error, MinosError::CfAuthFailed { message });
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

    dispatch_loop(ws, ctx, shutdown_rx).await
}

/// Inbound + outbound dispatch pump over an upgraded WebSocket. Returns
/// when the stream ends, errors, or `shutdown_rx` fires.
async fn dispatch_loop(
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ctx: &mut DispatchCtx,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> CycleOutcome {
    let (mut sink, mut stream) = ws.split();

    loop {
        tokio::select! {
            biased;
            _ = &mut *shutdown_rx => {
                let _ = sink.send(Message::Close(None)).await;
                return CycleOutcome::Shutdown;
            }
            out = ctx.out_rx.recv() => {
                let Some(envelope) = out else {
                    // `out_tx` dropped — client handle gone. Exit quietly.
                    return CycleOutcome::Shutdown;
                };
                let text = match serde_json::to_string(&envelope) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!(
                            target: "minos_daemon::relay_client",
                            error = %e,
                            "failed to serialize outbound envelope"
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
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Pong(_) | Message::Binary(_) | Message::Frame(_))) => {}
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
        }
    }
}

/// Parse an inbound text frame and route it. Non-fatal parse failures are
/// logged and swallowed — the dispatch loop stays alive.
async fn handle_inbound_text(text: &str, ctx: &DispatchCtx) -> Result<(), serde_json::Error> {
    let envelope: Envelope = serde_json::from_str(text)?;
    route_envelope(envelope, ctx).await;
    Ok(())
}

/// Route a parsed envelope to the watch channels (events), the local
/// jsonrpsee surface (forwarded RPC), or the debug log (unexpected kinds).
async fn route_envelope(envelope: Envelope, ctx: &DispatchCtx) {
    match envelope {
        Envelope::Event { event, .. } => route_event(event, ctx),
        Envelope::Forwarded { from, payload, .. } => {
            let Some(rpc_server) = ctx.rpc_server.clone() else {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    %from,
                    "received Forwarded with no rpc_server wired — dropping (test fixture?)"
                );
                return;
            };
            let response = invoke_forwarded(payload, &rpc_server).await;
            let envelope = wrap_response_envelope(response);
            // The relay re-wraps our Forward back to the originating peer
            // as Forwarded; correlation is the peers' responsibility (the
            // jsonrpc id is preserved end-to-end inside the payload).
            if let Err(e) = ctx.out_tx.send(envelope).await {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    %from,
                    "failed to enqueue forwarded RPC response"
                );
            }
        }
        Envelope::Forward { .. } | Envelope::Ingest { .. } => {
            // These are client → relay frames; the relay never emits them
            // to us. A misbehaving peer is the only way we'd see one.
            tracing::warn!(
                target: "minos_daemon::relay_client",
                "unexpected envelope kind from relay — dropping"
            );
        }
    }
}

fn route_event(event: EventKind, ctx: &DispatchCtx) {
    match event {
        EventKind::Paired {
            peer_device_id,
            peer_name,
            your_device_secret,
        } => {
            let record = PeerRecord {
                device_id: peer_device_id,
                name: peer_name.clone(),
                paired_at: Utc::now(),
            };
            persist_pairing(&record, &your_device_secret, ctx);
            if let Ok(mut guard) = ctx.peer_store.lock() {
                *guard = Some(record);
            }
            let _ = ctx.peer_tx.send(PeerState::Paired {
                peer_id: peer_device_id,
                peer_name,
                online: true,
            });
        }
        EventKind::PeerOnline { peer_device_id } => {
            ctx.peer_tx.send_if_modified(|s| match s {
                PeerState::Paired {
                    peer_id, online, ..
                } if *peer_id == peer_device_id && !*online => {
                    *online = true;
                    true
                }
                _ => false,
            });
        }
        EventKind::PeerOffline { peer_device_id } => {
            ctx.peer_tx.send_if_modified(|s| match s {
                PeerState::Paired {
                    peer_id, online, ..
                } if *peer_id == peer_device_id && *online => {
                    *online = false;
                    true
                }
                _ => false,
            });
        }
        EventKind::Unpaired => {
            clear_pairing(ctx);
            if let Ok(mut guard) = ctx.peer_store.lock() {
                *guard = None;
            }
            let _ = ctx.peer_tx.send(PeerState::Unpaired);
        }
        EventKind::ServerShutdown => {
            // The dispatch loop will observe the socket closing next and
            // fall through to the reconnect path; nothing to do here
            // beyond noting it for operators.
            tracing::info!(
                target: "minos_daemon::relay_client",
                "relay signalled server_shutdown; awaiting socket close"
            );
        }
        EventKind::UiEventMessage { thread_id, seq, .. } => {
            // Mobile-only fan-out frame. The host receives these only when
            // the backend relays a translated event to the paired iPhone,
            // and the host's role here is observational. Log + drop so
            // the dispatch loop stays cheap.
            tracing::debug!(
                target: "minos_daemon::relay_client",
                thread_id = %thread_id,
                seq,
                "ignoring UiEventMessage on the host side"
            );
        }
    }
}

/// Host pairing is process-local, so paired transitions no longer write to
/// the macOS Keychain or `local-state.json`.
fn persist_pairing(record: &PeerRecord, secret: &DeviceSecret, ctx: &DispatchCtx) {
    let _ = (record, secret, ctx);
}

/// Mirror of [`persist_pairing`] for `Unpaired`. Host pairing state is
/// process-local, so the relay echo only clears the in-memory mirror.
fn clear_pairing(ctx: &DispatchCtx) {
    let _ = ctx;
}

/// Map a WS close frame onto the outer `CycleOutcome`, populating
/// `last_error` when the code is one the spec §7.5 table calls out.
///
/// - `4401`: terminal — the relay has revoked our `device-secret`; exit
///   the task so the caller can prompt re-pair.
/// - `4400`: non-terminal — malformed / version-mismatched envelope.
///   Record `EnvelopeVersionUnsupported` so a follow-up UI read surfaces
///   the hint, but still reconnect: transient bugs may resolve on their
///   own and the user otherwise gets no signal at all.
/// - anything else: quiet reconnect (unchanged behaviour).
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
                "relay closed socket with 4400 — envelope rejected; \
                 will reconnect but recording EnvelopeVersionUnsupported"
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

/// Write a fatal error into the shared slot, overwriting any prior
/// value. Callers drain via [`crate::DaemonHandle::last_error`], so a
/// second error arriving before the first drain is expected to win —
/// the more recent signal is more useful to the UI.
fn store_last_error(slot: &Arc<StdMutex<Option<MinosError>>>, err: MinosError) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(err);
    }
}

/// Build the outbound auth-header bundle. Role is always `AgentHost` here —
/// this module is the Mac-side client by construction.
fn build_headers(
    config: &RelayConfig,
    device_id: DeviceId,
    secret: Option<&DeviceSecret>,
    mac_name: &str,
) -> AuthHeaders {
    let mut headers = AuthHeaders::new(device_id, DeviceRole::AgentHost).with_name(mac_name);
    if let Some(s) = secret {
        headers = headers.with_secret(s.clone());
    }
    if !config.cf_client_id.is_empty() && !config.cf_client_secret.is_empty() {
        headers = headers.with_cf_access(CfAccessToken::new(
            config.cf_client_id.clone(),
            config.cf_client_secret.clone(),
        ));
    }
    headers
}

/// Assemble the tungstenite request with all auth headers stamped. The URI
/// is expected to be a `ws://` or `wss://` URL pointing at `/devices`.
fn build_request(
    backend_url: &str,
    headers: &AuthHeaders,
) -> Result<ClientRequestBuilder, MinosError> {
    let uri: Uri = backend_url.parse().map_err(
        |e: tokio_tungstenite::tungstenite::http::uri::InvalidUri| MinosError::ConnectFailed {
            url: backend_url.into(),
            message: format!("invalid backend URL: {e}"),
        },
    )?;
    let mut builder = ClientRequestBuilder::new(uri);
    for (k, v) in headers.iter() {
        builder = builder.with_header(k, v);
    }
    Ok(builder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn build_headers_without_cf_omits_cf_headers() {
        let cfg = RelayConfig::new(String::new(), String::new(), String::new());
        let headers = build_headers(&cfg, DeviceId::new(), None, "my-mac");
        let keys: Vec<_> = headers.iter().map(|(k, _)| k).collect();
        assert!(
            !keys.iter().any(|k| k.starts_with("CF-Access")),
            "unexpected CF-Access headers: {keys:?}"
        );
        assert!(keys.contains(&"X-Device-Name"));
    }

    #[test]
    fn build_headers_with_cf_includes_both_cf_headers() {
        let cfg = RelayConfig::new(String::new(), "cid".into(), "csec".into());
        let headers = build_headers(&cfg, DeviceId::new(), None, "my-mac");
        let keys: Vec<_> = headers.iter().map(|(k, _)| k).collect();
        assert!(keys.contains(&"CF-Access-Client-Id"));
        assert!(keys.contains(&"CF-Access-Client-Secret"));
    }

    #[test]
    fn build_headers_with_secret_includes_x_device_secret() {
        let cfg = RelayConfig::new(String::new(), String::new(), String::new());
        let secret = DeviceSecret("sentinel-123".into());
        let headers = build_headers(&cfg, DeviceId::new(), Some(&secret), "my-mac");
        let entry = headers
            .iter()
            .find(|(k, _)| *k == "X-Device-Secret")
            .expect("X-Device-Secret stamped");
        assert_eq!(entry.1, "sentinel-123");
    }
}
