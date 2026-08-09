//! `AcpClient` — JSON-RPC 2.0 client over stdio for ACP agents.
//!
//! Architecture mirrors `CodexClient` (Option C — single-task writer):
//!
//! 1. `new()` takes a spawned child process, wraps stdin/stdout in a pump task.
//! 2. Outbound writes flow over `mpsc` channel → pump serializes to stdin.
//! 3. Inbound frames from stdout are dispatched: responses to pending calls,
//!    notifications and server requests forwarded via `inbound_rx`.

use std::collections::HashMap;
use std::sync::Arc;

use minos_acp_protocol::AcpClientNotification;
use minos_acp_protocol::AcpClientRequest;
use minos_domain::MinosError;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};
use uuid::Uuid;

#[derive(Debug)]
pub enum Inbound {
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

#[allow(dead_code)]
enum Outbound {
    Request {
        method: String,
        params: Value,
        reply_to: oneshot::Sender<Result<Value, MinosError>>,
    },
    Notification {
        method: String,
        params: Value,
        ack: oneshot::Sender<Result<(), MinosError>>,
    },
    Reply {
        id: Value,
        result: Value,
        ack: oneshot::Sender<Result<(), MinosError>>,
    },
    Error {
        id: Value,
        code: i64,
        message: String,
        ack: oneshot::Sender<Result<(), MinosError>>,
    },
}

#[derive(Debug)]
pub struct AcpClient {
    outbound_tx: mpsc::Sender<Outbound>,
    inbound_rx: Arc<Mutex<mpsc::Receiver<Inbound>>>,
    pump_task: JoinHandle<()>,
    /// Shared with the owning Gemini/Grok instance so shutdown can process-group
    /// kill the real child (not a None placeholder).
    child: Arc<tokio::sync::Mutex<Option<Child>>>,
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        self.pump_task.abort();
    }
}

impl AcpClient {
    pub fn new(child: Child) -> Result<Self, MinosError> {
        Self::from_shared_child(Arc::new(tokio::sync::Mutex::new(Some(child))))
    }

    /// Build a client that shares `child` ownership with the host instance.
    pub fn from_shared_child(
        child: Arc<tokio::sync::Mutex<Option<Child>>>,
    ) -> Result<Self, MinosError> {
        let (outbound_tx, outbound_rx) = mpsc::channel::<Outbound>(32);
        let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(64);

        let stdin = {
            let mut guard = child
                .try_lock()
                .map_err(|_| MinosError::GeminiSpawnFailed {
                    message: "could not lock child".into(),
                })?;
            guard
                .as_mut()
                .ok_or_else(|| MinosError::GeminiSpawnFailed {
                    message: "child is None".into(),
                })?
                .stdin
                .take()
                .ok_or_else(|| MinosError::GeminiSpawnFailed {
                    message: "could not take child stdin".into(),
                })?
        };

        let stdout = {
            let mut guard = child
                .try_lock()
                .map_err(|_| MinosError::GeminiSpawnFailed {
                    message: "could not lock child".into(),
                })?;
            guard
                .as_mut()
                .ok_or_else(|| MinosError::GeminiSpawnFailed {
                    message: "child is None".into(),
                })?
                .stdout
                .take()
                .ok_or_else(|| MinosError::GeminiSpawnFailed {
                    message: "could not take child stdout".into(),
                })?
        };

        let pump_task = tokio::spawn(pump_loop(stdin, stdout, outbound_rx, inbound_tx));

        Ok(Self {
            outbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            pump_task,
            child,
        })
    }

    /// Shared child handle for process-group kill on manager shutdown.
    #[must_use]
    pub fn child_handle(&self) -> Arc<tokio::sync::Mutex<Option<Child>>> {
        self.child.clone()
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, MinosError> {
        let (reply_to, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Request {
                method: method.to_string(),
                params,
                reply_to,
            })
            .await
            .map_err(|_| MinosError::AcpProtocolError {
                method: method.to_string(),
                message: "acp client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::AcpProtocolError {
            method: method.to_string(),
            message: "acp client dropped the call response".into(),
        })?
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), MinosError> {
        let (ack, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Notification {
                method: method.to_string(),
                params,
                ack,
            })
            .await
            .map_err(|_| MinosError::AcpProtocolError {
                method: method.to_string(),
                message: "acp client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::AcpProtocolError {
            method: method.to_string(),
            message: "acp client dropped the notification ack".into(),
        })?
    }

    pub async fn call_typed<R: AcpClientRequest>(
        &self,
        params: R,
    ) -> Result<R::Response, MinosError> {
        let value = serde_json::to_value(&params).map_err(|e| MinosError::AcpProtocolError {
            method: R::METHOD.into(),
            message: format!("encode params failed: {e}"),
        })?;
        let raw = self.call(R::METHOD, value).await?;
        serde_json::from_value::<R::Response>(raw).map_err(|e| MinosError::AcpProtocolError {
            method: R::METHOD.into(),
            message: format!("decode response failed: {e}"),
        })
    }

    pub async fn notify_typed<N: AcpClientNotification>(
        &self,
        params: N,
    ) -> Result<(), MinosError> {
        let value = serde_json::to_value(&params).map_err(|e| MinosError::AcpProtocolError {
            method: N::METHOD.into(),
            message: format!("encode notification params failed: {e}"),
        })?;
        self.notify(N::METHOD, value).await
    }

    #[allow(dead_code)]
    pub async fn reply(&self, id: Value, result: Value) -> Result<(), MinosError> {
        let (ack, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Reply { id, result, ack })
            .await
            .map_err(|_| MinosError::AcpProtocolError {
                method: "<reply>".into(),
                message: "acp client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::AcpProtocolError {
            method: "<reply>".into(),
            message: "acp client dropped the reply ack".into(),
        })?
    }

    pub async fn reply_error(
        &self,
        id: Value,
        code: i64,
        message: String,
    ) -> Result<(), MinosError> {
        let (ack, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Error {
                id,
                code,
                message,
                ack,
            })
            .await
            .map_err(|_| MinosError::AcpProtocolError {
                method: "<reply_error>".into(),
                message: "acp client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::AcpProtocolError {
            method: "<reply_error>".into(),
            message: "acp client dropped the error reply ack".into(),
        })?
    }

    pub async fn next_inbound(&self) -> Option<Inbound> {
        self.inbound_rx.lock().await.recv().await
    }
}

async fn pump_loop(
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    mut outbound_rx: mpsc::Receiver<Outbound>,
    inbound_tx: mpsc::Sender<Inbound>,
) {
    let mut stdin_writer = stdin;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut pending: HashMap<String, oneshot::Sender<Result<Value, MinosError>>> = HashMap::new();

    loop {
        tokio::select! {
            biased;
            maybe_cmd = outbound_rx.recv() => {
                let Some(cmd) = maybe_cmd else { break };
                match cmd {
                    Outbound::Request { method, params, reply_to } => {
                        let id = Uuid::new_v4().to_string();
                        let frame = minos_acp_protocol::make_request(serde_json::json!(id), &method, params);
                        if let Err(e) = write_frame(&mut stdin_writer, &method, &frame).await {
                            let _ = reply_to.send(Err(MinosError::AcpProtocolError { method, message: format!("stdin write failed: {e}") }));
                        } else {
                            pending.insert(id, reply_to);
                        }
                    }
                    Outbound::Notification { method, params, ack } => {
                        let frame = minos_acp_protocol::make_notification(&method, params);
                        let _ = ack.send(write_frame(&mut stdin_writer, &method, &frame).await);
                    }
                    Outbound::Reply { id, result, ack } => {
                        let frame = minos_acp_protocol::make_response(id, result);
                        let _ = ack.send(write_frame(&mut stdin_writer, "<reply>", &frame).await);
                    }
                    Outbound::Error { id, code, message, ack } => {
                        let frame = minos_acp_protocol::make_error(id, code, message, None);
                        let _ = ack.send(write_frame(&mut stdin_writer, "<reply_error>", &frame).await);
                    }
                }
            }
            maybe_line = lines.next_line() => {
                match maybe_line {
                    Ok(Some(line)) => { handle_inbound_line(&line, &mut pending, &inbound_tx).await; }
                    Ok(None) => {
                        debug!("gemini ACP stdout EOF");
                        for (_id, tx) in pending.drain() {
                            let _ = tx.send(Err(MinosError::AcpProtocolError { method: "<pending>".into(), message: "stdout closed before response".into() }));
                        }
                        let _ = inbound_tx.send(Inbound::Closed).await;
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "gemini ACP stdout read error");
                        for (_id, tx) in pending.drain() {
                            let _ = tx.send(Err(MinosError::AcpProtocolError { method: "<pending>".into(), message: format!("stdout read error: {e}") }));
                        }
                        let _ = inbound_tx.send(Inbound::Closed).await;
                        break;
                    }
                }
            }
        }
    }
}

async fn write_frame(
    stdin: &mut tokio::process::ChildStdin,
    method: &str,
    frame: &Value,
) -> Result<(), MinosError> {
    let mut bytes = serde_json::to_string(frame).map_err(|e| MinosError::AcpProtocolError {
        method: method.to_string(),
        message: format!("serialize frame failed: {e}"),
    })?;
    bytes.push('\n');
    stdin
        .write_all(bytes.as_bytes())
        .await
        .map_err(|e| MinosError::AcpProtocolError {
            method: method.to_string(),
            message: format!("stdin write failed: {e}"),
        })?;
    stdin
        .flush()
        .await
        .map_err(|e| MinosError::AcpProtocolError {
            method: method.to_string(),
            message: format!("stdin flush failed: {e}"),
        })?;
    Ok(())
}

async fn handle_inbound_line(
    line: &str,
    pending: &mut HashMap<String, oneshot::Sender<Result<Value, MinosError>>>,
    inbound_tx: &mpsc::Sender<Inbound>,
) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        warn!(raw = %line, "gemini ACP sent malformed JSON; ignoring");
        return;
    };
    let id = value.get("id").cloned();
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let has_result_or_error = value.get("result").is_some() || value.get("error").is_some();

    match (id, method, has_result_or_error) {
        (Some(id_val), None, true) => {
            let key = match &id_val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => {
                    warn!(id = ?id_val, "response with non-string/non-number id");
                    return;
                }
            };
            let Some(tx) = pending.remove(&key) else {
                warn!(id = ?id_val, "response for unknown request id");
                return;
            };
            if let Some(err) = value.get("error") {
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("ACP error without message")
                    .to_string();
                let _ = tx.send(Err(MinosError::AcpProtocolError {
                    method: "<response>".into(),
                    message,
                }));
            } else {
                let result = value.get("result").cloned().unwrap_or(Value::Null);
                let _ = tx.send(Ok(result));
            }
        }
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
        (None, Some(method), false) => {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = inbound_tx
                .send(Inbound::Notification { method, params })
                .await;
        }
        _ => {
            warn!(raw = %line, "gemini ACP sent ambiguous JSON-RPC frame");
        }
    }
}
