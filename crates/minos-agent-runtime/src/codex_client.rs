//! `CodexClient` — thin JSON-RPC 2.0 client speaking codex's WS dialect.
//!
//! ## WS ownership model (Option C — single-task writer)
//!
//! A WebSocket stream is inherently a duplex byte pipe. Multiple producers
//! pushing frames concurrently would need a write-side lock; multiple
//! consumers would need a read-side lock. Rather than a shared mutex, this
//! module picks a simpler pattern (Option C from the phase plan):
//!
//! 1. `connect()` establishes the WS, then moves both halves into a single
//!    `pump_task` (`tokio::spawn`'d). The task is the **only** owner of the
//!    socket from that point forward.
//! 2. All outbound writes (`call`, `reply`) flow over an `mpsc` channel the
//!    pump drains. The pump serialises writes, no lock required.
//! 3. Inbound frames are classified as JSON-RPC responses (dispatched to the
//!    matching `oneshot::Sender` held in a request-id → waker map) or
//!    forwarded as [`Inbound`] events to the consumer via another `mpsc`.
//!
//! ## JSON-RPC 2.0 framing over WS
//!
//! codex sends one JSON-RPC message per WS text frame. Disambiguation:
//!
//! - Has `id` + `method` ⇒ request (server → us).
//! - Has `id` + `result`/`error` ⇒ response (reply to one of our requests).
//! - Has `method`, no `id` ⇒ notification.
//!
//! Errors from codex (`{"error":{"code":..., "message":...}}`) are mapped to
//! [`MinosError::CodexProtocolError`] when surfaced through `call()`.
//!
//! The module is `pub(crate)` — external callers go through [`AgentRuntime`].
//! Only `Inbound` is exposed publicly to the `runtime` module.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use minos_domain::MinosError;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

/// An inbound JSON-RPC frame that isn't a response to an outstanding `call`.
/// Notifications and server requests are forwarded up; `Closed` is the
/// end-of-stream sentinel.
#[derive(Debug)]
pub(crate) enum Inbound {
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: Value,
        method: String,
        params: Value,
    },
    Closed,
}

/// Outbound command from the public API to the pump task.
enum Outbound {
    /// A JSON-RPC request expecting a response.
    Request {
        method: String,
        params: Value,
        reply_to: oneshot::Sender<Result<Value, MinosError>>,
    },
    /// A response to a server request — fire-and-forget.
    Reply {
        id: Value,
        result: Value,
        ack: oneshot::Sender<Result<(), MinosError>>,
    },
}

/// JSON-RPC client handle.
///
/// `call` and `reply` push commands through `outbound_tx` to the single
/// pump task, which owns the WS. Inbound frames the pump doesn't match to
/// a pending call are surfaced via `inbound_rx`. The `pump_task` field is
/// held only to abort the task on drop — without it the pump would outlive
/// `CodexClient` and the WS would stay open.
#[derive(Debug)]
pub(crate) struct CodexClient {
    outbound_tx: mpsc::Sender<Outbound>,
    inbound_rx: Arc<Mutex<mpsc::Receiver<Inbound>>>,
    pump_task: JoinHandle<()>,
}

impl Drop for CodexClient {
    fn drop(&mut self) {
        // Dropping `outbound_tx` closes the command channel; the pump's
        // select! returns None from `outbound_rx.recv()` and exits. We also
        // abort defensively so a pump blocked on WS read doesn't linger.
        self.pump_task.abort();
    }
}

impl CodexClient {
    /// Connect to `url` with a retry loop of 15 × 200 ms. Returns
    /// [`MinosError::CodexConnectFailed`] when all attempts are exhausted.
    pub(crate) async fn connect(url: &Url) -> Result<Self, MinosError> {
        let mut last_err: Option<String> = None;
        for _ in 0..15 {
            match tokio_tungstenite::connect_async(url.as_str()).await {
                Ok((ws, _resp)) => {
                    return Ok(Self::spawn_pump(ws));
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        Err(MinosError::CodexConnectFailed {
            url: url.to_string(),
            message: last_err.unwrap_or_else(|| "all retries exhausted".into()),
        })
    }

    /// Connect over an already-established WS stream (test entry point).
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) fn from_stream<S>(ws: tokio_tungstenite::WebSocketStream<S>) -> Self
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        Self::spawn_pump(ws)
    }

    fn spawn_pump<S>(ws: tokio_tungstenite::WebSocketStream<S>) -> Self
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (outbound_tx, outbound_rx) = mpsc::channel::<Outbound>(32);
        let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(64);
        let pump_task = tokio::spawn(pump_loop(ws, outbound_rx, inbound_tx));
        Self {
            outbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            pump_task,
        }
    }

    /// Issue a JSON-RPC `method` / `params` call and wait for the matching
    /// response. Any interleaved inbound frame (notifications, server
    /// requests) is buffered for `next_inbound()` to drain.
    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, MinosError> {
        let (reply_to, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Request {
                method: method.to_string(),
                params,
                reply_to,
            })
            .await
            .map_err(|_| MinosError::CodexProtocolError {
                method: method.to_string(),
                message: "codex client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::CodexProtocolError {
            method: method.to_string(),
            message: "codex client dropped the call response".into(),
        })?
    }

    /// Reply to a server request with a `result` value. Fire-and-forget from
    /// the caller's perspective — an error here means the WS went away.
    pub(crate) async fn reply(&self, id: Value, result: Value) -> Result<(), MinosError> {
        let (ack, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Reply { id, result, ack })
            .await
            .map_err(|_| MinosError::CodexProtocolError {
                method: "<reply>".into(),
                message: "codex client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::CodexProtocolError {
            method: "<reply>".into(),
            message: "codex client dropped the reply ack".into(),
        })?
    }

    /// Pull the next inbound frame (notification / server request / close).
    /// Returns `None` only after the stream is closed *and* all buffered
    /// frames have been drained.
    pub(crate) async fn next_inbound(&self) -> Option<Inbound> {
        self.inbound_rx.lock().await.recv().await
    }
}

/// Pump-task body. Owns the WebSocket exclusively.
async fn pump_loop<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    mut outbound_rx: mpsc::Receiver<Outbound>,
    inbound_tx: mpsc::Sender<Inbound>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();
    // request id → oneshot sender awaiting the matching response.
    let mut pending: HashMap<String, oneshot::Sender<Result<Value, MinosError>>> = HashMap::new();

    loop {
        tokio::select! {
            biased;
            maybe_cmd = outbound_rx.recv() => {
                let Some(cmd) = maybe_cmd else {
                    // All senders dropped → caller is done with us.
                    break;
                };
                match cmd {
                    Outbound::Request { method, params, reply_to } => {
                        let id = Uuid::new_v4().to_string();
                        let frame = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "method": method,
                            "params": params,
                        });
                        let send_res = sink.send(Message::text(frame.to_string())).await;
                        if let Err(e) = send_res {
                            let _ = reply_to.send(Err(MinosError::CodexProtocolError {
                                method,
                                message: format!("WS send failed: {e}"),
                            }));
                        } else {
                            pending.insert(id, reply_to);
                        }
                    }
                    Outbound::Reply { id, result, ack } => {
                        let frame = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": result,
                        });
                        let send_res = sink.send(Message::text(frame.to_string())).await;
                        let _ = ack.send(send_res.map_err(|e| MinosError::CodexProtocolError {
                            method: "<reply>".into(),
                            message: format!("WS send failed: {e}"),
                        }));
                    }
                }
            }
            maybe_msg = stream.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_inbound_frame(text.as_ref(), &mut pending, &inbound_tx).await;
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let text = if let Ok(s) = std::str::from_utf8(&bytes) {
                            s.to_string()
                        } else {
                            warn!("codex sent non-UTF-8 binary frame; ignoring");
                            continue;
                        };
                        handle_inbound_frame(&text, &mut pending, &inbound_tx).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        debug!("codex WS closed");
                        // Fail every outstanding call so the caller doesn't hang.
                        for (_id, tx) in pending.drain() {
                            let _ = tx.send(Err(MinosError::CodexProtocolError {
                                method: "<pending>".into(),
                                message: "WS closed before response".into(),
                            }));
                        }
                        let _ = inbound_tx.send(Inbound::Closed).await;
                        break;
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {
                        // tokio-tungstenite handles ping/pong internally at the
                        // default config; ignore the echoes.
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "codex WS read error");
                        for (_id, tx) in pending.drain() {
                            let _ = tx.send(Err(MinosError::CodexProtocolError {
                                method: "<pending>".into(),
                                message: format!("WS read error: {e}"),
                            }));
                        }
                        let _ = inbound_tx.send(Inbound::Closed).await;
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_inbound_frame(
    text: &str,
    pending: &mut HashMap<String, oneshot::Sender<Result<Value, MinosError>>>,
    inbound_tx: &mpsc::Sender<Inbound>,
) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        warn!(raw = %text, "codex sent malformed JSON-RPC frame; ignoring");
        return;
    };
    let id = value.get("id").cloned();
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let has_result_or_error = value.get("result").is_some() || value.get("error").is_some();

    match (id, method, has_result_or_error) {
        // Response: id + (result | error), no method.
        (Some(id_val), None, true) => {
            let key = match &id_val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => {
                    warn!(id = ?id_val, "response with non-string / non-number id; cannot dispatch");
                    return;
                }
            };
            let Some(tx) = pending.remove(&key) else {
                warn!(id = ?id_val, "response for unknown request id; dropping");
                return;
            };
            if let Some(err) = value.get("error") {
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("codex error without message")
                    .to_string();
                let _ = tx.send(Err(MinosError::CodexProtocolError {
                    method: "<response>".into(),
                    message,
                }));
            } else {
                let result = value.get("result").cloned().unwrap_or(Value::Null);
                let _ = tx.send(Ok(result));
            }
        }
        // Server request: id + method (no result/error).
        (Some(id_val), Some(method), false) => {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = inbound_tx
                .send(Inbound::ServerRequest {
                    id: id_val,
                    method,
                    params,
                })
                .await;
        }
        // Notification: method only.
        (None, Some(method), false) => {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = inbound_tx
                .send(Inbound::Notification { method, params })
                .await;
        }
        _ => {
            warn!(raw = %text, "codex sent ambiguous JSON-RPC frame; ignoring");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;
    use tokio_tungstenite::tungstenite::protocol::Role;
    use tokio_tungstenite::WebSocketStream;

    /// Build a connected pair of `WebSocketStream`s over an in-memory duplex
    /// pipe. One side roleplays as the server (our FakeCodex), the other is
    /// the client under test. The in-memory variant skips the WS handshake
    /// entirely — both ends are created in "already-connected" state via
    /// `from_raw_socket(.., Role::{Server, Client}, None)`.
    async fn duplex_pair() -> (
        WebSocketStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (a, b) = duplex(64 * 1024);
        let client = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(b, Role::Server, None).await;
        (client, server)
    }

    #[tokio::test]
    async fn call_receives_response_by_matching_id() {
        let (client_ws, server_ws) = duplex_pair().await;
        let client = CodexClient::from_stream(client_ws);

        // Server side reads a frame, extracts the id, echoes a success response.
        let server_task = tokio::spawn(async move {
            let (mut tx, mut rx) = server_ws.split();
            let frame = match rx.next().await {
                Some(Ok(Message::Text(t))) => t,
                other => panic!("expected text frame, got {other:?}"),
            };
            let parsed: Value = serde_json::from_str(frame.as_ref()).unwrap();
            assert_eq!(parsed["method"], "initialize");
            assert_eq!(parsed["params"]["foo"], "bar");
            let id = parsed["id"].clone();
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"ok": true},
            });
            tx.send(Message::text(response.to_string())).await.unwrap();
            // Keep the stream alive briefly so the client can drain the ack.
            tokio::time::sleep(Duration::from_millis(20)).await;
        });

        let result = client
            .call("initialize", serde_json::json!({"foo": "bar"}))
            .await
            .unwrap();
        assert_eq!(result["ok"], true);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn notifications_go_to_next_inbound() {
        let (client_ws, server_ws) = duplex_pair().await;
        let client = CodexClient::from_stream(client_ws);

        let server_task = tokio::spawn(async move {
            let (mut tx, _rx) = server_ws.split();
            let frame = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "item/agentMessage/delta",
                "params": {"delta": "Hello"},
            });
            tx.send(Message::text(frame.to_string())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        });

        let inbound = client.next_inbound().await.unwrap();
        match inbound {
            Inbound::Notification { method, params } => {
                assert_eq!(method, "item/agentMessage/delta");
                assert_eq!(params["delta"], "Hello");
            }
            other => panic!("expected notification, got {other:?}"),
        }
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn server_requests_surface_with_id_and_reply_round_trips() {
        let (client_ws, server_ws) = duplex_pair().await;
        let client = CodexClient::from_stream(client_ws);

        let server_task = tokio::spawn(async move {
            let (mut tx, mut rx) = server_ws.split();
            let frame = serde_json::json!({
                "jsonrpc": "2.0",
                "id": "srv-1",
                "method": "ExecCommandApproval",
                "params": {"command": ["ls"]},
            });
            tx.send(Message::text(frame.to_string())).await.unwrap();
            // Expect the client to reply.
            let reply = match rx.next().await {
                Some(Ok(Message::Text(t))) => t,
                other => panic!("expected reply frame, got {other:?}"),
            };
            let parsed: Value = serde_json::from_str(reply.as_ref()).unwrap();
            assert_eq!(parsed["id"], "srv-1");
            assert_eq!(parsed["result"]["decision"], "rejected");
        });

        let inbound = client.next_inbound().await.unwrap();
        let (id, method) = match inbound {
            Inbound::ServerRequest { id, method, .. } => (id, method),
            other => panic!("expected server request, got {other:?}"),
        };
        assert_eq!(method, "ExecCommandApproval");
        client
            .reply(id, serde_json::json!({"decision": "rejected"}))
            .await
            .unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn call_maps_jsonrpc_error_to_codex_protocol_error() {
        let (client_ws, server_ws) = duplex_pair().await;
        let client = CodexClient::from_stream(client_ws);

        let server_task = tokio::spawn(async move {
            let (mut tx, mut rx) = server_ws.split();
            let frame = match rx.next().await {
                Some(Ok(Message::Text(t))) => t,
                other => panic!("expected text frame, got {other:?}"),
            };
            let parsed: Value = serde_json::from_str(frame.as_ref()).unwrap();
            let id = parsed["id"].clone();
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32000, "message": "codex said no"},
            });
            tx.send(Message::text(response.to_string())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        });

        let err = client
            .call("turn/start", serde_json::json!({}))
            .await
            .expect_err("call should have failed");
        match err {
            MinosError::CodexProtocolError { message, .. } => {
                assert!(message.contains("codex said no"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn closed_stream_fails_in_flight_calls() {
        let (client_ws, server_ws) = duplex_pair().await;
        let client = CodexClient::from_stream(client_ws);

        let server_task = tokio::spawn(async move {
            let (_tx, mut rx) = server_ws.split();
            // Wait for the call, then drop the server to close the stream.
            let _ = rx.next().await;
        });

        let err = client
            .call("turn/start", serde_json::json!({}))
            .await
            .expect_err("call should have failed");
        match err {
            MinosError::CodexProtocolError { message, .. } => {
                assert!(
                    message.contains("WS closed") || message.contains("WS read error"),
                    "{message}",
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn connect_retry_exhausts_for_unreachable_port() {
        // Pick an ephemeral port and never bind it — connect will fail every
        // time. We shorten the sleep below by validating the error only, not
        // the elapsed time; 15×200ms = 3s is tolerable in the unit test.
        let url = Url::parse("ws://127.0.0.1:1").unwrap();
        let err = CodexClient::connect(&url)
            .await
            .expect_err("connect must fail");
        match err {
            MinosError::CodexConnectFailed { url: u, .. } => {
                // `Url::parse` canonicalises to include a trailing slash; we
                // accept either shape so a future parser tweak doesn't break us.
                assert!(u.starts_with("ws://127.0.0.1:1"), "{u}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn inbound_closed_fires_on_server_drop() {
        let (client_ws, server_ws) = duplex_pair().await;
        let client = CodexClient::from_stream(client_ws);

        // Drop the server — client's stream will see EOF.
        drop(server_ws);

        // Eventually the inbound channel yields Closed.
        let inbound = tokio::time::timeout(Duration::from_secs(2), client.next_inbound())
            .await
            .unwrap();
        matches!(inbound, Some(Inbound::Closed));
    }
}
