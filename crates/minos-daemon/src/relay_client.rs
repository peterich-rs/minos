//! Outbound WebSocket client of the `minos-relay` broker.
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
//!   4. serializes in-flight `LocalRpc` traffic behind an id-correlated
//!      pending map so `send_local_rpc` can be called concurrently from
//!      multiple awaiters.
//!
//! The module intentionally does NOT touch `DaemonHandle`. Plan 05 Phase F
//! wires the handle to this client; Phase E (this module) only has to stand
//! on its own with the `relay_client_smoke` integration tests.
//!
//! # Error handling
//!
//! - A connect-time HTTP 401 (CF Access or relay's pre-upgrade auth check)
//!   is unambiguously an auth failure and is surfaced as
//!   `MinosError::CfAuthFailed`. The task then exits with a `Disconnected`
//!   link state — the caller must call [`RelayClient::stop`] and spawn a
//!   fresh client once creds have been rotated. All other errors (including
//!   full WS close-code mapping) fall back to exponential-backoff reconnect
//!   (1s → 2s → 4s → 8s → 16s → 30s cap, no max attempts).
//! - `send_local_rpc` has a 10-second timeout. On timeout or on a dropped
//!   dispatch task the entry is cleaned out of the pending map and
//!   `MinosError::RelayInternal { message: "local rpc timeout" }` is
//!   returned.
//!
//! # Phase F work (not yet wired here)
//!
//! - Mapping WS close codes 4400/4401/4409 into typed [`MinosError`] variants
//!   beyond the pre-upgrade 401 path. For now those codes just trigger a
//!   reconnect.
//! - Delivering `Envelope::Forwarded` payloads into the jsonrpsee dispatch
//!   loop. Phase E drops them at `warn!` level for visibility.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret, MinosError, PeerState, RelayLinkState};
use minos_protocol::envelope::{Envelope, EventKind, LocalRpcMethod, LocalRpcOutcome, RpcError};
use minos_transport::auth::{AuthHeaders, CfAccessToken};
use minos_transport::backoff::delay_for_attempt;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::tungstenite::Error as WsError;

use crate::config::RelayConfig;
use crate::relay_pairing::{PeerRecord, RelayQrPayload};
use crate::rpc_server::{invoke_forwarded, wrap_response_envelope, RpcServerImpl};

/// Timeout applied to every pending `LocalRpc` response. Matches the
/// dispatch-loop jitter we saw in the Phase D e2e runs (well under 5s) with
/// enough margin to survive a lossy mobile network round trip.
const LOCAL_RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounded queue for outbound envelopes — deep enough to absorb a brief
/// handshake pause without back-pressuring callers. The dispatch loop
/// drains continuously, so the steady-state depth is effectively zero.
const OUTBOUND_QUEUE_DEPTH: usize = 64;

/// Correlation table for outbound `LocalRpc`. The dispatch task inserts
/// arriving `LocalRpcResponse` outcomes by id; `send_local_rpc` removes
/// on timeout / success. Using a plain `HashMap` (not `DashMap`) is fine
/// here — the send path is the sole writer and contention is low.
type Pending = HashMap<u64, oneshot::Sender<LocalRpcOutcome>>;

struct Inner {
    /// Correlation ids for outbound `LocalRpc`. Starts at 1, monotonic for
    /// the lifetime of the client; relay treats these as opaque.
    next_id: AtomicU64,
    /// Awaiters keyed by correlation id. Shared with the dispatch task via
    /// `Arc<Mutex<_>>`; the task inserts `LocalRpcResponse` outcomes by
    /// looking up the id and this handle removes on timeout / success.
    pending: Arc<Mutex<Pending>>,
    /// Producer side of the dispatcher's outbound queue.
    out_tx: mpsc::Sender<Envelope>,
    /// Shutdown signal — one-shot, captured behind a `Mutex` so a repeat
    /// `stop()` after the first call is a benign no-op.
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    /// The dispatch task join handle; taken on `stop()`.
    task: Mutex<Option<JoinHandle<()>>>,
    /// The Mac's display name — embedded in every `RelayQrPayload` we mint.
    mac_name: String,
    /// The relay's backend URL — embedded in every `RelayQrPayload` we mint
    /// so the iPhone learns where to dial.
    backend_url: String,
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
    pub fn spawn(
        config: RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        mac_name: String,
        backend_url: String,
        rpc_server: Option<Arc<RpcServerImpl>>,
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

        let pending: Arc<Mutex<Pending>> = Arc::new(Mutex::new(HashMap::new()));

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
            pending: pending.clone(),
            rpc_server,
        };

        let task = tokio::spawn(run_dispatch(dispatch_ctx, shutdown_rx));

        let inner = Arc::new(Inner {
            next_id: AtomicU64::new(1),
            pending,
            out_tx,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            task: Mutex::new(Some(task)),
            mac_name,
            backend_url,
        });

        (Arc::new(Self { inner }), link_rx, peer_rx)
    }

    /// Correlation-id allocator. Public only within the crate for tests
    /// that want to check monotonicity; steady-state callers always go
    /// through [`Self::send_local_rpc`].
    fn alloc_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a `LocalRpc` envelope and await its correlated response.
    ///
    /// Maps a `LocalRpcOutcome::Err` whose code is otherwise unknown to
    /// `MinosError::RelayInternal`. See [`rpc_error_to_minos`].
    pub async fn send_local_rpc(
        &self,
        method: LocalRpcMethod,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, MinosError> {
        let id = self.alloc_id();
        let (tx, rx) = oneshot::channel::<LocalRpcOutcome>();

        // Register first so a racing response isn't lost if the dispatcher
        // drains quickly.
        {
            let mut pending = self.pending_map().lock().await;
            pending.insert(id, tx);
        }

        let envelope = Envelope::LocalRpc {
            version: 1,
            id,
            method: method.clone(),
            params,
        };

        if let Err(e) = self.inner.out_tx.send(envelope).await {
            self.pending_map().lock().await.remove(&id);
            return Err(MinosError::RelayInternal {
                message: format!("relay dispatch task stopped: {e}"),
            });
        }

        match timeout(LOCAL_RPC_TIMEOUT, rx).await {
            Ok(Ok(LocalRpcOutcome::Ok { result })) => Ok(result),
            Ok(Ok(LocalRpcOutcome::Err { error })) => Err(rpc_error_to_minos(&error)),
            Ok(Err(_dropped)) => {
                self.pending_map().lock().await.remove(&id);
                Err(MinosError::RelayInternal {
                    message: "local rpc timeout".into(),
                })
            }
            Err(_elapsed) => {
                self.pending_map().lock().await.remove(&id);
                Err(MinosError::RelayInternal {
                    message: "local rpc timeout".into(),
                })
            }
        }
    }

    /// Issue `RequestPairingToken` and wrap the response into the Mac-side
    /// QR payload shape. The relay returns `{token, expires_at}`; we embed
    /// the token plus the Mac's display name and the backend URL the
    /// iPhone should dial.
    pub async fn request_pairing_token(&self) -> Result<RelayQrPayload, MinosError> {
        let result = self
            .send_local_rpc(LocalRpcMethod::RequestPairingToken, serde_json::json!({}))
            .await?;
        let token = result
            .get("token")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| MinosError::RelayInternal {
                message: "request_pairing_token response missing 'token'".into(),
            })?
            .to_owned();

        Ok(RelayQrPayload {
            v: 1,
            backend_url: self.inner.backend_url.clone(),
            token: minos_domain::PairingToken(token),
            mac_display_name: self.inner.mac_name.clone(),
        })
    }

    /// Issue `ForgetPeer`. The relay then emits `Event::Unpaired` which the
    /// dispatch loop pushes onto the peer-state watch channel — callers do
    /// NOT need to await that event here.
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        self.send_local_rpc(LocalRpcMethod::ForgetPeer, serde_json::json!({}))
            .await?;
        Ok(())
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

    fn pending_map(&self) -> &Arc<Mutex<Pending>> {
        &self.inner.pending
    }
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
    pending: Arc<Mutex<Pending>>,
    /// Local jsonrpsee surface invoked when the relay delivers an
    /// `Envelope::Forwarded`. `None` in tests that don't exercise the
    /// peer-RPC path; production wires the daemon's `RpcServerImpl` here.
    rpc_server: Option<Arc<RpcServerImpl>>,
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
            return CycleOutcome::AuthFailed;
        }
    };

    let ws = tokio::select! {
        biased;
        _ = &mut *shutdown_rx => return CycleOutcome::Shutdown,
        res = tokio_tungstenite::connect_async(request) => match res {
            Ok((stream, _resp)) => stream,
            Err(WsError::Http(resp)) if resp.status().as_u16() == 401 => {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    "relay handshake returned HTTP 401 — auth failure, exiting task"
                );
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
                        tracing::info!(
                            target: "minos_daemon::relay_client",
                            close = ?frame,
                            "relay sent Close; reconnecting"
                        );
                        return CycleOutcome::Reconnect;
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

/// Route a parsed envelope to the pending map (responses), watch channels
/// (events), or the debug log (unexpected / not-yet-wired kinds).
async fn route_envelope(envelope: Envelope, ctx: &DispatchCtx) {
    match envelope {
        Envelope::LocalRpcResponse { id, outcome, .. } => {
            let entry = { ctx.pending.lock().await.remove(&id) };
            if let Some(tx) = entry {
                let _ = tx.send(outcome);
            } else {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    id,
                    "local rpc response for unknown id — dropping"
                );
            }
        }
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
        Envelope::LocalRpc { .. } | Envelope::Forward { .. } => {
            // These are client → relay frames; the relay never emits them
            // to us. A misbehaving peer is the only way we'd see one.
            tracing::warn!(
                target: "minos_daemon::relay_client",
                "unexpected envelope kind from relay — dropping"
            );
        }
    }
}

/// Apply a server-initiated `EventKind` to the peer-state watch channel.
fn route_event(event: EventKind, ctx: &DispatchCtx) {
    match event {
        EventKind::Paired {
            peer_device_id,
            peer_name,
            your_device_secret: _,
        } => {
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
    }
}

/// Map a relay `RpcError` code onto the closest typed `MinosError` variant.
/// Unknown codes collapse to `RelayInternal`.
fn rpc_error_to_minos(error: &RpcError) -> MinosError {
    match error.code.as_str() {
        "pairing_token_invalid" => MinosError::PairingTokenInvalid,
        "unauthorized" => MinosError::Unauthorized {
            reason: error.message.clone(),
        },
        _ => MinosError::RelayInternal {
            message: format!("{}: {}", error.code, error.message),
        },
    }
}

/// Build the outbound auth-header bundle. Role is always `MacHost` here —
/// this module is the Mac-side client by construction.
fn build_headers(
    config: &RelayConfig,
    device_id: DeviceId,
    secret: Option<&DeviceSecret>,
    mac_name: &str,
) -> AuthHeaders {
    let mut headers = AuthHeaders::new(device_id, DeviceRole::MacHost).with_name(mac_name);
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
    fn rpc_error_known_codes_map_to_typed_variants() {
        let tok = RpcError {
            code: "pairing_token_invalid".into(),
            message: "token expired".into(),
        };
        assert!(matches!(
            rpc_error_to_minos(&tok),
            MinosError::PairingTokenInvalid
        ));

        let unauth = RpcError {
            code: "unauthorized".into(),
            message: "nope".into(),
        };
        match rpc_error_to_minos(&unauth) {
            MinosError::Unauthorized { reason } => assert_eq!(reason, "nope"),
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn rpc_error_unknown_code_collapses_to_relay_internal() {
        let e = RpcError {
            code: "bogus_new_code".into(),
            message: "reason".into(),
        };
        match rpc_error_to_minos(&e) {
            MinosError::RelayInternal { message } => {
                assert!(
                    message.contains("bogus_new_code") && message.contains("reason"),
                    "expected code+message in message, got {message}"
                );
            }
            other => panic!("expected RelayInternal, got {other:?}"),
        }
    }

    #[test]
    fn build_headers_without_cf_omits_cf_headers() {
        let cfg = RelayConfig::new(String::new(), String::new());
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
        let cfg = RelayConfig::new("cid".into(), "csec".into());
        let headers = build_headers(&cfg, DeviceId::new(), None, "my-mac");
        let keys: Vec<_> = headers.iter().map(|(k, _)| k).collect();
        assert!(keys.contains(&"CF-Access-Client-Id"));
        assert!(keys.contains(&"CF-Access-Client-Secret"));
    }

    #[test]
    fn build_headers_with_secret_includes_x_device_secret() {
        let cfg = RelayConfig::new(String::new(), String::new());
        let secret = DeviceSecret("sentinel-123".into());
        let headers = build_headers(&cfg, DeviceId::new(), Some(&secret), "my-mac");
        let entry = headers
            .iter()
            .find(|(k, _)| *k == "X-Device-Secret")
            .expect("X-Device-Secret stamped");
        assert_eq!(entry.1, "sentinel-123");
    }
}
