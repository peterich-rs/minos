use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::mcp_socket::{SocketRequest, SocketResponse};
use crate::teamwork_mcp::{TeamworkMcpToolCatalog, ToolCallContext};

/// Canonical MCP initialize instructions from `minos-prompt-runtime` package.
use minos_prompt_runtime::TEAMWORK_MCP_SERVER_INSTRUCTIONS as SERVER_INSTRUCTIONS;

/// Margin added on top of `wait_delegation.timeout_ms` so the socket read does
/// not race the daemon assembling the terminal response frame.
pub const WAIT_DELEGATION_SOCKET_MARGIN: Duration = Duration::from_secs(5);
const DEFAULT_SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Exit if the agent process stops speaking on stdio for this long (sidecar leak guard).
const STDIO_IDLE_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

/// JSON-RPC server error: Minos daemon / UDS is not reachable.
pub const MCP_ERR_DAEMON_UNAVAILABLE: i64 = -32001;
/// JSON-RPC server error: daemon closed the socket mid-request.
pub const MCP_ERR_SOCKET_CLOSED: i64 = -32002;
/// JSON-RPC server error: daemon returned an application-level rejection.
pub const MCP_ERR_DAEMON_REJECTED: i64 = -32003;
/// JSON-RPC invalid params (tool schema / arg validation).
pub const MCP_ERR_INVALID_PARAMS: i64 = -32602;

pub use crate::teamwork_mcp::TeamworkMcpPermissions as McpToolPermissions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub socket_path: PathBuf,
    pub conversation_id: String,
    pub source_agent: Option<AgentName>,
    pub source_session_id: Option<String>,
    pub permissions: McpToolPermissions,
}

/// Classified MCP↔daemon socket failures so agents can self-recover.
#[derive(Debug, thiserror::Error)]
pub enum McpSocketClientError {
    #[error(
        "minos daemon unavailable: failed to connect to MCP socket at {path} ({source}). \
         Ensure the Minos host daemon is running."
    )]
    DaemonUnavailable {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "minos daemon closed the MCP socket before responding. \
         The host daemon may have restarted or the session was closed."
    )]
    SocketClosed,
    #[error("minos daemon rejected MCP request: {message}")]
    DaemonRejected { message: String },
    #[error("minos MCP socket I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl McpSocketClientError {
    pub fn mcp_code(&self) -> i64 {
        match self {
            Self::DaemonUnavailable { .. } => MCP_ERR_DAEMON_UNAVAILABLE,
            Self::SocketClosed => MCP_ERR_SOCKET_CLOSED,
            Self::DaemonRejected { .. } => MCP_ERR_DAEMON_REJECTED,
            Self::Io(_) | Self::Other(_) => MCP_ERR_DAEMON_REJECTED,
        }
    }
}

pub async fn serve_stdio(config: McpServerConfig) -> Result<()> {
    let socket_path = config.socket_path.clone();
    let mut reader = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let read = tokio::time::timeout(STDIO_IDLE_TIMEOUT, reader.read_line(&mut line)).await;
        match read {
            Ok(Ok(0)) => break, // EOF — agent closed stdin
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                eprintln!(
                    "minos-teamwork-mcp: stdio idle for {}s; exiting to avoid sidecar leak",
                    STDIO_IDLE_TIMEOUT.as_secs()
                );
                break;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                write_json(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32700, "message": error.to_string()}
                    }),
                )
                .await?;
                continue;
            }
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let response = handle_request(&config, &socket_path, id, request).await;
        write_json(&mut stdout, &response?).await?;
    }
    Ok(())
}

async fn handle_request(
    config: &McpServerConfig,
    socket_path: &Path,
    id: Value,
    request: Value,
) -> Result<Value> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .context("MCP request missing method")?;
    match method {
        "initialize" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": request
                    .get("params")
                    .and_then(|params| params.get("protocolVersion"))
                    .and_then(Value::as_str)
                    .unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "minos-teamwork-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": SERVER_INSTRUCTIONS
            }
        })),
        "tools/list" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": TeamworkMcpToolCatalog::default_catalog().tool_schemas(config.permissions) }
        })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            match handle_tool_call(config, socket_path, id.clone(), params).await {
                Ok(response) => Ok(response),
                Err(ToolCallFailure::InvalidParams(message)) => {
                    Ok(error_response(id, MCP_ERR_INVALID_PARAMS, &message))
                }
                Err(ToolCallFailure::Socket(error)) => {
                    Ok(error_response(id, error.mcp_code(), &error.to_string()))
                }
            }
        }
        "ping" => Ok(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        _ => Ok(error_response(
            id,
            -32601,
            &format!("unsupported MCP method: {method}"),
        )),
    }
}

enum ToolCallFailure {
    InvalidParams(String),
    Socket(McpSocketClientError),
}

async fn handle_tool_call(
    config: &McpServerConfig,
    socket_path: &Path,
    id: Value,
    params: Value,
) -> Result<Value, ToolCallFailure> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let request = TeamworkMcpToolCatalog::default_catalog()
        .socket_request_for_call(
            config.permissions,
            ToolCallContext {
                conversation_id: config.conversation_id.clone(),
                source_agent: config.source_agent,
                source_session_id: config.source_session_id.clone(),
            },
            name,
            args,
        )
        .map_err(|error| ToolCallFailure::InvalidParams(error.to_string()))?;
    let result = send_socket_request(socket_path, request)
        .await
        .map_err(ToolCallFailure::Socket)?;
    tool_response(id, result)
        .map_err(|error| ToolCallFailure::Socket(McpSocketClientError::Other(error)))
}

/// Read timeout for a sidecar→daemon framed request.
///
/// `wait_delegation` holds the connection until the delegation is terminal or
/// `timeout_ms` elapses, so the socket must outlive that bound.
pub fn read_timeout_for_request(request: &SocketRequest) -> Duration {
    match request {
        SocketRequest::WaitDelegation { timeout_ms, .. } => {
            let base_ms = u64::try_from((*timeout_ms).max(0)).unwrap_or(0);
            Duration::from_millis(base_ms).saturating_add(WAIT_DELEGATION_SOCKET_MARGIN)
        }
        _ => DEFAULT_SOCKET_READ_TIMEOUT,
    }
}

async fn send_socket_request(
    socket_path: &Path,
    request: SocketRequest,
) -> Result<serde_json::Value, McpSocketClientError> {
    #[cfg(not(unix))]
    {
        let _ = (socket_path, request);
        return Err(McpSocketClientError::Other(anyhow::anyhow!(
            "MCP Unix-domain sockets are only supported on Unix hosts"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let path_display = socket_path.display().to_string();
        let stream = UnixStream::connect(socket_path).map_err(|source| {
            McpSocketClientError::DaemonUnavailable {
                path: path_display.clone(),
                source,
            }
        })?;
        let read_timeout = read_timeout_for_request(&request);
        stream.set_read_timeout(Some(read_timeout))?;
        stream.set_write_timeout(Some(DEFAULT_SOCKET_WRITE_TIMEOUT))?;
        let result =
            tokio::task::spawn_blocking(move || -> Result<SocketResponse, McpSocketClientError> {
                use crate::mcp_socket::read_response_frame;
                use std::io::Write;
                let payload = serde_json::to_vec(&request)
                    .map_err(|error| McpSocketClientError::Other(error.into()))?;
                let len = u32::try_from(payload.len()).map_err(|_| {
                    McpSocketClientError::Other(anyhow::anyhow!("request payload too large"))
                })?;
                let mut buf = Vec::with_capacity(4 + payload.len());
                buf.extend_from_slice(&len.to_be_bytes());
                buf.extend_from_slice(&payload);
                {
                    let mut stream_ref = &stream;
                    stream_ref.write_all(&buf)?;
                    stream_ref.flush()?;
                }
                let mut stream_ref = &stream;
                let response = match read_response_frame::<&UnixStream>(&mut stream_ref) {
                    Ok(response) => response,
                    Err(error) => {
                        // Map OS read timeouts / unexpected EOF into classified errors.
                        if error_chain_has_io_kind(&error, std::io::ErrorKind::TimedOut)
                            || error_chain_has_io_kind(&error, std::io::ErrorKind::WouldBlock)
                        {
                            return Err(McpSocketClientError::Other(anyhow::anyhow!(
                                "MCP socket read timed out after {}ms waiting for daemon response",
                                read_timeout.as_millis()
                            )));
                        }
                        if error_chain_has_io_kind(&error, std::io::ErrorKind::UnexpectedEof) {
                            return Err(McpSocketClientError::SocketClosed);
                        }
                        return Err(McpSocketClientError::Other(error));
                    }
                };
                match response {
                    None => Err(McpSocketClientError::SocketClosed),
                    Some(SocketResponse::Ok { data }) => Ok(SocketResponse::Ok { data }),
                    Some(SocketResponse::Error { message }) => {
                        Err(McpSocketClientError::DaemonRejected { message })
                    }
                    Some(SocketResponse::Pong) => Ok(SocketResponse::Pong),
                }
            })
            .await
            .map_err(|error| {
                McpSocketClientError::Other(anyhow::anyhow!("socket task join error: {error}"))
            })??;
        match result {
            SocketResponse::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
            SocketResponse::Error { message } => {
                Err(McpSocketClientError::DaemonRejected { message })
            }
            SocketResponse::Pong => Ok(serde_json::Value::Null),
        }
    }
}

fn tool_response(id: Value, payload: Value) -> Result<Value> {
    Ok(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&payload)?
            }],
            "structuredContent": payload,
            "isError": false
        }
    }))
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

async fn write_json(stdout: &mut tokio::io::Stdout, response: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(response)?;
    stdout.write_all(&bytes).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

fn error_chain_has_io_kind(error: &anyhow::Error, kind: std::io::ErrorKind) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_error| io_error.kind() == kind)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_socket::SocketRequest;

    #[test]
    fn wait_delegation_read_timeout_tracks_timeout_ms_plus_margin() {
        let request = SocketRequest::WaitDelegation {
            conversation_id: "c1".into(),
            delegation_id: "d1".into(),
            timeout_ms: 120_000,
        };
        assert_eq!(
            read_timeout_for_request(&request),
            Duration::from_millis(120_000) + WAIT_DELEGATION_SOCKET_MARGIN
        );
    }

    #[test]
    fn wait_delegation_read_timeout_covers_max_allowed_timeout() {
        let request = SocketRequest::WaitDelegation {
            conversation_id: "c1".into(),
            delegation_id: "d1".into(),
            timeout_ms: 600_000,
        };
        assert_eq!(
            read_timeout_for_request(&request),
            Duration::from_millis(600_000) + WAIT_DELEGATION_SOCKET_MARGIN
        );
    }

    #[test]
    fn non_wait_requests_keep_default_read_timeout() {
        let request = SocketRequest::GetDelegationStatus {
            conversation_id: "c1".into(),
            delegation_id: "d1".into(),
        };
        assert_eq!(
            read_timeout_for_request(&request),
            DEFAULT_SOCKET_READ_TIMEOUT
        );
    }

    #[test]
    fn socket_error_codes_are_stable() {
        let unavailable = McpSocketClientError::DaemonUnavailable {
            path: "/tmp/x.sock".into(),
            source: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"),
        };
        assert_eq!(unavailable.mcp_code(), MCP_ERR_DAEMON_UNAVAILABLE);
        assert!(unavailable.to_string().contains("daemon unavailable"));

        let closed = McpSocketClientError::SocketClosed;
        assert_eq!(closed.mcp_code(), MCP_ERR_SOCKET_CLOSED);
        assert!(closed.to_string().contains("closed the MCP socket"));

        let rejected = McpSocketClientError::DaemonRejected {
            message: "session closed".into(),
        };
        assert_eq!(rejected.mcp_code(), MCP_ERR_DAEMON_REJECTED);
        assert!(rejected.to_string().contains("session closed"));
    }
}
