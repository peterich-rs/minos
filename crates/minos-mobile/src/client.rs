//! Envelope-aware mobile client.
//!
//! Plan 05 replaces the jsonrpsee-backed client with a bespoke envelope
//! WebSocket loop (spec §6) so the mobile side can consume
//! `EventKind::UiEventMessage` frames live and call the new
//! `LocalRpcMethod::{ListThreads, ReadThread}` for history.
//!
//! Responsibilities:
//!
//! - Parse a scanned QR v2 payload (`PairingQrPayload` from
//!   `minos_protocol::messages`) and persist its fields into the
//!   [`MobilePairingStore`]: backend URL, CF Access tokens (if any),
//!   eventually the `DeviceSecret` minted by the backend on successful
//!   pair.
//! - Maintain a single outbound WebSocket; expose `ConnectionState` via a
//!   `watch::Receiver` and live `UiEventFrame` over a `broadcast::Sender`.
//! - Correlate `LocalRpc` requests by `id` using a `DashMap<u64,
//!   oneshot::Sender<Envelope>>`; callers (`pair`, `list_threads`,
//!   `read_thread`) send and await one-shot responses.
//!
//! For FFI use, [`MobileClient::new_with_in_memory_store`] avoids exposing
//! the `Arc<dyn MobilePairingStore>` trait object across the frb boundary
//! (real Keychain persistence lives on the Dart side; see plan D5).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use minos_domain::{ConnectionState, DeviceId, DeviceSecret, MinosError};
use minos_protocol::{
    Envelope, EventKind, GetThreadLastSeqParams, GetThreadLastSeqResponse, ListThreadsParams,
    ListThreadsResponse, LocalRpcMethod, LocalRpcOutcome, PairingQrPayload, ReadThreadParams,
    ReadThreadResponse, RpcError,
};
use minos_ui_protocol::UiEventMessage;
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::store::{InMemoryPairingStore, MobilePairingStore};

/// One live UI event pushed from backend fan-out. Mobile layers consume
/// these via [`MobileClient::ui_events_stream`] (broadcast receiver).
#[derive(Debug, Clone)]
pub struct UiEventFrame {
    pub thread_id: String,
    pub seq: u64,
    pub ui: UiEventMessage,
    pub ts_ms: i64,
}

/// Envelope-speaking mobile client. One instance per iPhone process.
pub struct MobileClient {
    store: Arc<dyn MobilePairingStore>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    ui_events_tx: broadcast::Sender<UiEventFrame>,
    outbox: Mutex<Option<mpsc::Sender<Envelope>>>,
    next_rpc_id: AtomicU64,
    pending: Arc<DashMap<u64, oneshot::Sender<Envelope>>>,
    device_id: DeviceId,
    self_name: String,
    #[allow(dead_code)] // held so drop closes the inbound loop
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

/// Timeout for one local-RPC round trip. Generous; typical RTT < 100ms.
const LOCAL_RPC_TIMEOUT: Duration = Duration::from_secs(15);

impl MobileClient {
    #[must_use]
    pub fn new(store: Arc<dyn MobilePairingStore>, self_name: String) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let (ui_events_tx, _) = broadcast::channel(256);
        Self {
            store,
            state_tx,
            state_rx,
            ui_events_tx,
            outbox: Mutex::new(None),
            next_rpc_id: AtomicU64::new(1),
            pending: Arc::new(DashMap::new()),
            device_id: DeviceId::new(),
            self_name,
            tasks: Mutex::new(Vec::new()),
        }
    }

    /// FFI-friendly constructor. The Dart side owns real persistence via
    /// `flutter_secure_storage` (plan D5); this default is the in-memory
    /// backing so the FFI surface never leaks `Arc<dyn MobilePairingStore>`.
    #[must_use]
    pub fn new_with_in_memory_store(self_name: String) -> Self {
        Self::new(Arc::new(InMemoryPairingStore::new()), self_name)
    }

    /// Current connection state snapshot. Cheap and synchronous.
    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    /// Subscribe to connection-state transitions. First read on the
    /// receiver returns the current cached value.
    #[must_use]
    pub fn events_stream(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Subscribe to live UI events from backend fan-out. Creates a fresh
    /// broadcast receiver so each subscriber gets its own lag window.
    #[must_use]
    pub fn ui_events_stream(&self) -> broadcast::Receiver<UiEventFrame> {
        self.ui_events_tx.subscribe()
    }

    /// Return the device id the client registered with. Stable for the
    /// lifetime of the process (re-generated on restart until persisted).
    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    // ─────────────────────────── pairing flow ────────────────────────────

    /// Scan a QR v2 payload (raw JSON). Persists `backend_url` + CF tokens
    /// to the store, opens the WebSocket, sends `LocalRpc::Pair`, persists
    /// the returned `DeviceSecret` on success, and transitions
    /// [`ConnectionState`] through `Pairing → Connected`.
    ///
    /// Errors:
    /// - `StoreCorrupt { path: "qr_payload", .. }` when the JSON doesn't
    ///   parse.
    /// - `PairingQrVersionUnsupported` when `qr.v != 2`.
    /// - `ConnectFailed` / `Disconnected` on WS or RPC round-trip failures.
    pub async fn pair_with_qr_json(&self, qr_json: String) -> Result<(), MinosError> {
        let qr: PairingQrPayload =
            serde_json::from_str(&qr_json).map_err(|e| MinosError::StoreCorrupt {
                path: "qr_payload".into(),
                message: e.to_string(),
            })?;
        if qr.v != 2 {
            return Err(MinosError::PairingQrVersionUnsupported { version: qr.v });
        }
        self.store.save_backend_url(&qr.backend_url).await?;
        let cf = match (
            qr.cf_access_client_id.clone(),
            qr.cf_access_client_secret.clone(),
        ) {
            (Some(id), Some(sec)) => {
                self.store.save_cf_access(&id, &sec).await?;
                Some((id, sec))
            }
            _ => None,
        };

        let _ = self.state_tx.send(ConnectionState::Pairing);

        // First-boot: no DeviceSecret yet. Connect without the secret
        // header; the backend allows bearer-less handshakes on the
        // pairing path and issues the secret inside the `Pair` result.
        self.connect(&qr.backend_url, None, cf).await?;

        // Perform the `Pair` RPC.
        let params = serde_json::json!({
            "token": qr.pairing_token,
            "device_name": self.self_name,
        });
        let result = self.local_rpc(LocalRpcMethod::Pair, params).await?;

        // Backend replies with the minted DeviceSecret. Persist it so the
        // next connect can resume authenticated. Shape (spec §6.1):
        // { "device_secret": "<base64url>" }.
        let secret_str = result
            .get("device_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MinosError::RpcCallFailed {
                method: "pair".into(),
                message: "pair response missing device_secret".into(),
            })?;
        let device_secret = DeviceSecret(secret_str.to_string());
        self.store
            .save_device(&self.device_id, &device_secret)
            .await?;

        let _ = self.state_tx.send(ConnectionState::Connected);
        Ok(())
    }

    /// Forget the current pairing. Clears secure storage, drops the
    /// socket, and emits `Disconnected`. Idempotent.
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        // Best-effort: ask the server to tear down its side too.
        let _ = self
            .local_rpc(LocalRpcMethod::ForgetPeer, serde_json::json!({}))
            .await;

        self.store.clear_all().await?;
        self.shutdown_outbound().await;
        let _ = self.state_tx.send(ConnectionState::Disconnected);
        Ok(())
    }

    // ─────────────────────────── history rpcs ────────────────────────────

    /// Request a page of thread summaries from the backend.
    pub async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        let value = self
            .local_rpc(
                LocalRpcMethod::ListThreads,
                serde_json::to_value(&req).expect("ListThreadsParams is always serializable"),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| MinosError::RpcCallFailed {
            method: "list_threads".into(),
            message: e.to_string(),
        })
    }

    /// Read a window of translated UI events from one thread.
    pub async fn read_thread(
        &self,
        req: ReadThreadParams,
    ) -> Result<ReadThreadResponse, MinosError> {
        let value = self
            .local_rpc(
                LocalRpcMethod::ReadThread,
                serde_json::to_value(&req).expect("ReadThreadParams is always serializable"),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| MinosError::RpcCallFailed {
            method: "read_thread".into(),
            message: e.to_string(),
        })
    }

    /// Host-only helper (mobile rarely uses this; included for parity).
    pub async fn get_thread_last_seq(
        &self,
        req: GetThreadLastSeqParams,
    ) -> Result<GetThreadLastSeqResponse, MinosError> {
        let value = self
            .local_rpc(
                LocalRpcMethod::GetThreadLastSeq,
                serde_json::to_value(&req).expect("GetThreadLastSeqParams is always serializable"),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| MinosError::RpcCallFailed {
            method: "get_thread_last_seq".into(),
            message: e.to_string(),
        })
    }

    // ─────────────────────────── internals ────────────────────────────

    async fn connect(
        &self,
        url: &str,
        device_secret: Option<&str>,
        cf_access: Option<(String, String)>,
    ) -> Result<(), MinosError> {
        let mut req = url
            .into_client_request()
            .map_err(|e| MinosError::ConnectFailed {
                url: url.to_string(),
                message: e.to_string(),
            })?;
        let headers = req.headers_mut();
        headers.insert(
            "X-Device-Id",
            self.device_id
                .to_string()
                .parse()
                .map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "device_id is not a valid header value".into(),
                })?,
        );
        headers.insert(
            "X-Device-Role",
            "ios-client".parse().expect("static header value is valid"),
        );
        if let Some(sec) = device_secret {
            headers.insert(
                "X-Device-Secret",
                sec.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "device_secret is not a valid header value".into(),
                })?,
            );
        }
        if let Some((id, sec)) = cf_access {
            headers.insert(
                "CF-Access-Client-Id",
                id.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "cf_access_client_id is not a valid header value".into(),
                })?,
            );
            headers.insert(
                "CF-Access-Client-Secret",
                sec.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "cf_access_client_secret is not a valid header value".into(),
                })?,
            );
        }

        let (ws, _resp) = connect_async(req)
            .await
            .map_err(|e| MinosError::ConnectFailed {
                url: url.to_string(),
                message: e.to_string(),
            })?;
        let (mut write, read) = ws.split();

        let (tx, mut rx) = mpsc::channel::<Envelope>(256);

        let send_handle = tokio::spawn(async move {
            while let Some(env) = rx.recv().await {
                let text = match serde_json::to_string(&env) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(?e, "mobile: envelope serialise failed");
                        continue;
                    }
                };
                if let Err(e) = write.send(Message::Text(text.into())).await {
                    tracing::warn!(?e, "mobile: WS write failed; send loop exiting");
                    break;
                }
            }
        });

        let recv_handle = tokio::spawn(recv_loop(
            read,
            self.ui_events_tx.clone(),
            self.pending.clone(),
            self.state_tx.clone(),
        ));

        *self.outbox.lock().await = Some(tx);
        let mut tasks = self.tasks.lock().await;
        tasks.push(send_handle);
        tasks.push(recv_handle);
        Ok(())
    }

    async fn shutdown_outbound(&self) {
        let mut guard = self.outbox.lock().await;
        *guard = None; // drops the Sender; send task exits when channel closes
        self.pending.clear();
    }

    async fn local_rpc(
        &self,
        method: LocalRpcMethod,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, MinosError> {
        let id = self.next_rpc_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);

        let env = Envelope::LocalRpc {
            version: 1,
            id,
            method,
            params,
        };

        let outbox = {
            let guard = self.outbox.lock().await;
            guard
                .as_ref()
                .cloned()
                .ok_or_else(|| MinosError::Disconnected {
                    reason: "mobile: no live outbound channel".into(),
                })?
        };
        outbox
            .send(env)
            .await
            .map_err(|_| MinosError::Disconnected {
                reason: "mobile: outbound mpsc closed".into(),
            })?;

        let resp = tokio::time::timeout(LOCAL_RPC_TIMEOUT, rx)
            .await
            .map_err(|_| MinosError::RpcCallFailed {
                method: "local_rpc".into(),
                message: "timed out after 15s".into(),
            })?
            .map_err(|_| MinosError::Disconnected {
                reason: "mobile: oneshot dropped before response".into(),
            })?;

        match resp {
            Envelope::LocalRpcResponse { outcome, .. } => match outcome {
                LocalRpcOutcome::Ok { result } => Ok(result),
                LocalRpcOutcome::Err { error } => Err(from_rpc_error(&error)),
            },
            other => Err(MinosError::RpcCallFailed {
                method: "local_rpc".into(),
                message: format!("unexpected envelope in local_rpc response: {other:?}"),
            }),
        }
    }
}

/// Inbound read loop. Decodes each text frame as `Envelope`, routes
/// `LocalRpcResponse` to its pending oneshot, and surfaces
/// `UiEventMessage` events to the broadcast channel.
async fn recv_loop<S>(
    mut read: S,
    ui_events_tx: broadcast::Sender<UiEventFrame>,
    pending: Arc<DashMap<u64, oneshot::Sender<Envelope>>>,
    state_tx: watch::Sender<ConnectionState>,
) where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                let text: &str = t.as_ref();
                handle_text_frame(text, &ui_events_tx, &pending, &state_tx);
            }
            Ok(Message::Close(_)) => {
                let _ = state_tx.send(ConnectionState::Disconnected);
                break;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(?e, "mobile: WS read error; inbound loop exiting");
                let _ = state_tx.send(ConnectionState::Disconnected);
                break;
            }
        }
    }
}

fn handle_text_frame(
    text: &str,
    ui_events_tx: &broadcast::Sender<UiEventFrame>,
    pending: &DashMap<u64, oneshot::Sender<Envelope>>,
    state_tx: &watch::Sender<ConnectionState>,
) {
    let env = match serde_json::from_str::<Envelope>(text) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(?e, text = %text, "mobile: inbound decode error");
            return;
        }
    };
    match env {
        Envelope::LocalRpcResponse { id, .. } if pending.contains_key(&id) => {
            if let Some((_, sink)) = pending.remove(&id) {
                if let Ok(env) = serde_json::from_str::<Envelope>(text) {
                    let _ = sink.send(env);
                }
            }
        }
        Envelope::Event { event, .. } => match event {
            EventKind::UiEventMessage {
                thread_id,
                seq,
                ui,
                ts_ms,
            } => {
                let _ = ui_events_tx.send(UiEventFrame {
                    thread_id,
                    seq,
                    ui,
                    ts_ms,
                });
            }
            EventKind::Unpaired | EventKind::ServerShutdown => {
                let _ = state_tx.send(ConnectionState::Disconnected);
            }
            _ => tracing::debug!(?event, "mobile: ignored event"),
        },
        other => tracing::debug!(?other, "mobile: ignored inbound envelope"),
    }
}

/// Map a backend-reported RPC error code into a typed `MinosError`.
/// Unknown codes fall through to `RpcCallFailed` so the localization table
/// still produces something the UI can render.
fn from_rpc_error(err: &RpcError) -> MinosError {
    match err.code.as_str() {
        "pairing_token_invalid" => MinosError::PairingTokenInvalid,
        "unauthorized" => MinosError::Unauthorized {
            reason: err.message.clone(),
        },
        "thread_not_found" => MinosError::ThreadNotFound {
            thread_id: err.message.clone(),
        },
        _ => MinosError::RpcCallFailed {
            method: err.code.clone(),
            message: err.message.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_with_in_memory_store_starts_disconnected() {
        let client = MobileClient::new_with_in_memory_store("test".into());
        assert_eq!(client.current_state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn pair_with_qr_json_rejects_invalid_json_as_store_corrupt() {
        let client = MobileClient::new_with_in_memory_store("test".into());
        let err = client
            .pair_with_qr_json("not json".into())
            .await
            .expect_err("invalid JSON must not parse into PairingQrPayload");
        assert!(
            matches!(&err, MinosError::StoreCorrupt { path, .. } if path == "qr_payload"),
            "expected StoreCorrupt {{ path: \"qr_payload\", .. }}, got {err:?}"
        );
    }

    #[tokio::test]
    async fn pair_with_qr_json_rejects_wrong_version() {
        let client = MobileClient::new_with_in_memory_store("test".into());
        let qr = serde_json::json!({
            "v": 1,
            "backend_url": "wss://x/devices",
            "host_display_name": "Mac",
            "pairing_token": "tok",
            "expires_at_ms": 1_i64,
        });
        let err = client
            .pair_with_qr_json(qr.to_string())
            .await
            .expect_err("v=1 must be rejected");
        assert!(
            matches!(err, MinosError::PairingQrVersionUnsupported { version: 1 }),
            "unexpected error: {err:?}"
        );
    }

    #[tokio::test]
    async fn list_threads_without_outbound_errors_disconnected() {
        let client = MobileClient::new_with_in_memory_store("test".into());
        let err = client
            .list_threads(ListThreadsParams {
                limit: 10,
                before_ts_ms: None,
                agent: None,
            })
            .await
            .expect_err("RPC with no connection must error");
        assert!(
            matches!(err, MinosError::Disconnected { .. }),
            "unexpected error: {err:?}"
        );
    }
}
