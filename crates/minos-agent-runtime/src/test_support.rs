//! `FakeCodexServer` — a scripted tokio-tungstenite WS accept loop that
//! stands in for `codex app-server` during integration tests.
//!
//! This module is **only** compiled when the `test-support` feature is on.
//! Production builds never link this code; the feature is enabled exclusively
//! from dev-dependencies (this crate's own tests via self-ref; `minos-daemon`
//! for its `agent_e2e.rs`).
//!
//! ## Script semantics
//!
//! A test hands [`FakeCodexServer::bind`] a `Vec<Step>`. The server accepts
//! **exactly one** client (the agent-runtime WS client), then drains the
//! script in order:
//!
//! - [`Step::ExpectRequest`] — reads one frame, asserts it's a JSON-RPC
//!   request whose `method` matches, replies with a typed `result`.
//! - [`Step::ExpectNotification`] — reads one client notification frame and
//!   asserts its method/params shape.
//! - [`Step::EmitNotification`] — writes a JSON-RPC notification frame.
//! - [`Step::EmitServerRequest`] — writes a JSON-RPC request frame with a
//!   fresh string id. The id is stored in the server's `server_request_ids`
//!   vector so the caller can later correlate how the agent-runtime replied.
//! - [`Step::DieUnexpectedly`] — closes the WS abruptly (no close frame).
//!
//! After the last step, the accept task exits; the WS stream is dropped
//! naturally when the `Step::DieUnexpectedly` sentinel isn't present.
//!
//! ## Why not wrap `minos-transport::WsServer`?
//!
//! `WsServer` is a jsonrpsee server built around our own trait. codex's WS
//! schema differs (method names, param shapes). Wrapping `WsServer` would
//! require bypassing its type-safe pipeline anyway — we'd be writing raw
//! JSON into jsonrpsee's internals. `tokio-tungstenite::accept_async`
//! matches codex's own client examples one-for-one and is the path of
//! least surprise.

use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// One step of a scripted interaction with a fake codex app-server.
#[derive(Debug, Clone)]
pub enum Step {
    /// Read one client frame; assert it's a JSON-RPC request with `method`;
    /// reply with `{"jsonrpc":"2.0","id":<received id>,"result":<reply>}`.
    ExpectRequest {
        method: String,
        reply: serde_json::Value,
    },
    /// Read one client notification frame and assert its method/params.
    ExpectNotification {
        method: String,
        params: serde_json::Value,
    },
    /// Send `{"jsonrpc":"2.0","method":<method>,"params":<params>}`.
    EmitNotification {
        method: String,
        params: serde_json::Value,
    },
    /// Send a JSON-RPC request with a freshly-minted id. The id is recorded
    /// in `FakeCodexServer::server_request_ids()` so the test can assert
    /// how the agent-runtime replied (e.g. confirm auto-reject shape).
    EmitServerRequest {
        method: String,
        params: serde_json::Value,
    },
    /// Close the WS abruptly without a close frame.
    DieUnexpectedly,
}

/// A scripted WS server masquerading as `codex app-server`.
///
/// Accepts exactly one client connection. Drop the returned handle (or call
/// [`FakeCodexServer::stop`]) to abort the accept task.
pub struct FakeCodexServer {
    accept_task: JoinHandle<()>,
    server_request_ids: Arc<Mutex<Vec<String>>>,
}

impl FakeCodexServer {
    /// Bind to an ephemeral loopback port and spawn the accept task.
    /// Returns `(self, port)`; the port is what Phase C's agent-runtime
    /// connects to via `ws://127.0.0.1:<port>`.
    pub async fn bind(script: Vec<Step>) -> (Self, u16) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("FakeCodexServer failed to bind loopback port");
        let port = listener
            .local_addr()
            .expect("FakeCodexServer local_addr failed")
            .port();

        let server_request_ids = Arc::new(Mutex::new(Vec::<String>::new()));
        let ids_clone = Arc::clone(&server_request_ids);

        let accept_task = tokio::spawn(async move {
            // Accept exactly one client per test.
            let (stream, _peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::error!(error = %e, "FakeCodexServer accept failed");
                    return;
                }
            };
            let ws = match accept_async(stream).await {
                Ok(ws) => ws,
                Err(e) => {
                    tracing::error!(error = %e, "FakeCodexServer WS handshake failed");
                    return;
                }
            };
            run_script(ws, script, ids_clone).await;
        });

        (
            Self {
                accept_task,
                server_request_ids,
            },
            port,
        )
    }

    /// Shut down the accept task, surfacing any panic it produced.
    ///
    /// If the script task already finished on its own (naturally or via
    /// panic), we await the join handle to surface any panic — silently
    /// discarding it here would let script-drift bugs (e.g. method mismatch
    /// on `ExpectRequest`) pass as green tests. If the task is still running
    /// (a test intentionally stopped before draining all steps), we abort
    /// silently — that's a valid early-shutdown path.
    pub async fn stop(self) {
        if self.accept_task.is_finished() {
            match self.accept_task.await {
                Ok(()) => {}
                Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
                Err(e) => panic!("FakeCodexServer task unexpectedly cancelled: {e}"),
            }
        } else {
            self.accept_task.abort();
        }
    }

    /// Snapshot the ids assigned to each [`Step::EmitServerRequest`] so the
    /// test can assert how the agent-runtime replied.
    pub async fn server_request_ids(&self) -> Vec<String> {
        self.server_request_ids.lock().await.clone()
    }
}

async fn run_script(
    ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    script: Vec<Step>,
    ids: Arc<Mutex<Vec<String>>>,
) {
    let (mut tx, mut rx) = ws.split();
    for step in script {
        match step {
            Step::ExpectRequest { method, reply } => {
                let frame = match rx.next().await {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Binary(b))) => {
                        String::from_utf8(b.to_vec()).unwrap_or_default()
                    }
                    Some(Ok(other)) => {
                        panic!("FakeCodexServer: expected text/binary frame, got {other:?}");
                    }
                    Some(Err(e)) => panic!("FakeCodexServer: WS read error: {e}"),
                    None => panic!("FakeCodexServer: WS closed before ExpectRequest({method})"),
                };
                let parsed: serde_json::Value = serde_json::from_str(&frame)
                    .unwrap_or_else(|e| panic!("FakeCodexServer: bad JSON from client: {e}"));
                let got_method = parsed
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                assert_eq!(
                    got_method, method,
                    "FakeCodexServer: method mismatch on ExpectRequest"
                );
                let id = parsed.get("id").cloned().unwrap_or(serde_json::Value::Null);
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": reply,
                });
                if let Err(e) = tx.send(Message::text(response.to_string())).await {
                    tracing::warn!(error = %e, "FakeCodexServer: send reply failed");
                    return;
                }
            }
            Step::ExpectNotification { method, params } => {
                let frame = match rx.next().await {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Binary(b))) => {
                        String::from_utf8(b.to_vec()).unwrap_or_default()
                    }
                    Some(Ok(other)) => {
                        panic!("FakeCodexServer: expected text/binary frame, got {other:?}");
                    }
                    Some(Err(e)) => panic!("FakeCodexServer: WS read error: {e}"),
                    None => {
                        panic!("FakeCodexServer: WS closed before ExpectNotification({method})")
                    }
                };
                let parsed: serde_json::Value = serde_json::from_str(&frame)
                    .unwrap_or_else(|e| panic!("FakeCodexServer: bad JSON from client: {e}"));
                let got_method = parsed
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                assert_eq!(
                    got_method, method,
                    "FakeCodexServer: method mismatch on ExpectNotification"
                );
                assert!(
                    parsed.get("id").is_none(),
                    "FakeCodexServer: notifications must not carry an id"
                );
                assert_eq!(parsed.get("params").cloned().unwrap_or_default(), params);
            }
            Step::EmitNotification { method, params } => {
                let frame = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params,
                });
                if let Err(e) = tx.send(Message::text(frame.to_string())).await {
                    tracing::warn!(error = %e, "FakeCodexServer: send notification failed");
                    return;
                }
            }
            Step::EmitServerRequest { method, params } => {
                let id = format!("fake-srv-{}", uuid::Uuid::new_v4());
                ids.lock().await.push(id.clone());
                let frame = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": params,
                });
                if let Err(e) = tx.send(Message::text(frame.to_string())).await {
                    tracing::warn!(error = %e, "FakeCodexServer: send server request failed");
                    return;
                }
            }
            Step::DieUnexpectedly => {
                // Drop the sender/receiver without sending a close frame.
                drop(tx);
                drop(rx);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::connect_async;

    /// Smoke: bind the fake, drive one ExpectRequest round-trip with a
    /// real client, assert the reply shape and the stop() teardown path.
    #[tokio::test]
    async fn expect_request_round_trip() {
        let script = vec![Step::ExpectRequest {
            method: "initialize".into(),
            reply: serde_json::json!({"ok": true}),
        }];
        let (server, port) = FakeCodexServer::bind(script).await;
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = connect_async(&url).await.unwrap();
        let (mut tx, mut rx) = ws.split();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {},
        });
        tx.send(Message::text(req.to_string())).await.unwrap();
        let reply_frame = rx.next().await.unwrap().unwrap();
        let reply_text = match reply_frame {
            Message::Text(t) => t.to_string(),
            other => panic!("unexpected: {other:?}"),
        };
        let reply: serde_json::Value = serde_json::from_str(&reply_text).unwrap();
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["ok"], true);
        server.stop().await;
    }

    #[tokio::test]
    async fn expect_notification_round_trip() {
        let script = vec![Step::ExpectNotification {
            method: "notifications/initialized".into(),
            params: serde_json::json!({}),
        }];
        let (server, port) = FakeCodexServer::bind(script).await;
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = connect_async(&url).await.unwrap();
        let (mut tx, _rx) = ws.split();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {},
        });
        tx.send(Message::text(req.to_string())).await.unwrap();
        server.stop().await;
    }

    /// Notifications flow unprompted; no client request needed.
    #[tokio::test]
    async fn emit_notification_arrives_at_client() {
        let script = vec![Step::EmitNotification {
            method: "item/agentMessage/delta".into(),
            params: serde_json::json!({"delta": "Hi"}),
        }];
        let (server, port) = FakeCodexServer::bind(script).await;
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = connect_async(&url).await.unwrap();
        let (_tx, mut rx) = ws.split();
        let frame = rx.next().await.unwrap().unwrap();
        let text = match frame {
            Message::Text(t) => t.to_string(),
            other => panic!("unexpected: {other:?}"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["method"], "item/agentMessage/delta");
        assert_eq!(parsed["params"]["delta"], "Hi");
        server.stop().await;
    }

    /// Server requests get a fresh id that the caller can later correlate.
    #[tokio::test]
    async fn emit_server_request_records_id() {
        let script = vec![Step::EmitServerRequest {
            method: "ExecCommandApproval".into(),
            params: serde_json::json!({"command": ["ls"]}),
        }];
        let (server, port) = FakeCodexServer::bind(script).await;
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = connect_async(&url).await.unwrap();
        let (_tx, mut rx) = ws.split();
        let frame = rx.next().await.unwrap().unwrap();
        let text = match frame {
            Message::Text(t) => t.to_string(),
            other => panic!("unexpected: {other:?}"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let id = parsed["id"].as_str().unwrap().to_string();
        let recorded = server.server_request_ids().await;
        assert_eq!(recorded, vec![id]);
        server.stop().await;
    }

    /// Regression: `stop()` must surface a panic from a finished accept task
    /// rather than silently discard it. If the script expects `method: "foo"`
    /// but the client sends `method: "bar"`, `run_script`'s assert_eq! panics;
    /// `stop()` must re-raise that panic so the test fails loudly.
    #[tokio::test]
    #[should_panic(expected = "method mismatch on ExpectRequest")]
    async fn stop_propagates_script_drift_panic() {
        let script = vec![Step::ExpectRequest {
            method: "foo".into(),
            reply: serde_json::json!({}),
        }];
        let (server, port) = FakeCodexServer::bind(script).await;
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _resp) = connect_async(&url).await.unwrap();
        let (mut tx, mut rx) = ws.split();
        // Send the wrong method; this trips the assert_eq! inside run_script.
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "bar",
            "params": {},
        });
        tx.send(Message::text(req.to_string())).await.unwrap();
        // Drain until the server-side panic closes the stream, so the accept
        // task has definitely terminated by the time we call stop(). Without
        // this, stop()'s is_finished() check could race and abort the task
        // before it records its panic.
        while rx.next().await.is_some() {}
        server.stop().await;
    }
}
