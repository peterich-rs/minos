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

/// Timeout applied to every pending `LocalRpc` response. Matches the
/// dispatch-loop jitter we saw in the Phase D e2e runs (well under 5s) with
/// enough margin to survive a lossy mobile network round trip.
const LOCAL_RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounded queue for outbound envelopes — deep enough to absorb a brief
/// handshake pause without back-pressuring callers. The dispatch loop
/// drains continuously, so the steady-state depth is effectively zero.
const OUTBOUND_QUEUE_DEPTH: usize = 64;

/// Channel between outside callers and the background dispatch task; the
/// task owns the WS stream and is the sole writer. Using `oneshot` for the
/// reply lets `send_local_rpc` await just its own correlated response
/// without scanning a shared queue.
struct Pending(HashMap<u64, oneshot::Sender<LocalRpcOutcome>>);

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
    ) -> (
        Arc<Self>,
        watch::Receiver<RelayLinkState>,
        watch::Receiver<PeerState>,
    ) {
        let (link_tx, link_rx) = watch::channel(RelayLinkState::Disconnected);
        let initial_peer = peer.as_ref().map_or(PeerState::Unpaired, |p| PeerState::Paired {
            peer_id: p.device_id,
            peer_name: p.name.clone(),
            // We haven't connected yet — the relay will emit PeerOnline
            // or PeerOffline inside the first authenticated frame.
            online: false,
        });
        let (peer_tx, peer_rx) = watch::channel(initial_peer);

        let (out_tx, out_rx) = mpsc::channel::<Envelope>(OUTBOUND_QUEUE_DEPTH);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let pending = Arc::new(Mutex::new(Pending(HashMap::new())));

        let dispatch_ctx = DispatchCtx {
            config,
            self_device_id,
            secret,
            mac_name: mac_name.clone(),
            backend_url: backend_url.clone(),
            link_tx,
            peer_tx,
            out_rx,
            shutdown_rx,
            pending: pending.clone(),
        };

        let task = tokio::spawn(run_dispatch(dispatch_ctx));

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
            pending.0.insert(id, tx);
        }

        let envelope = Envelope::LocalRpc {
            version: 1,
            id,
            method: method.clone(),
            params,
        };

        if let Err(e) = self.inner.out_tx.send(envelope).await {
            self.pending_map().lock().await.0.remove(&id);
            return Err(MinosError::RelayInternal {
                message: format!("relay dispatch task stopped: {e}"),
            });
        }

        match timeout(LOCAL_RPC_TIMEOUT, rx).await {
            Ok(Ok(LocalRpcOutcome::Ok { result })) => Ok(result),
            Ok(Ok(LocalRpcOutcome::Err { error })) => Err(rpc_error_to_minos(&error)),
            Ok(Err(_dropped)) => {
                self.pending_map().lock().await.0.remove(&id);
                Err(MinosError::RelayInternal {
                    message: "local rpc timeout".into(),
                })
            }
            Err(_elapsed) => {
                self.pending_map().lock().await.0.remove(&id);
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
struct DispatchCtx {
    config: RelayConfig,
    self_device_id: DeviceId,
    secret: Option<DeviceSecret>,
    mac_name: String,
    backend_url: String,
    link_tx: watch::Sender<RelayLinkState>,
    peer_tx: watch::Sender<PeerState>,
    out_rx: mpsc::Receiver<Envelope>,
    shutdown_rx: oneshot::Receiver<()>,
    pending: Arc<Mutex<Pending>>,
}

/// Background task body. Runs the connect → dispatch → reconnect loop
/// until signaled to exit via `shutdown_rx`.
async fn run_dispatch(_ctx: DispatchCtx) {
    // Phase E scaffold: real loop lands in the next commit.
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
    let uri: Uri = backend_url.parse().map_err(|e: tokio_tungstenite::tungstenite::http::uri::InvalidUri| {
        MinosError::ConnectFailed {
            url: backend_url.into(),
            message: format!("invalid backend URL: {e}"),
        }
    })?;
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
