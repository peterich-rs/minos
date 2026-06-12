use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde_json::{json, Value};

use crate::mcp_socket::{SocketRequest, SocketResponse};
use crate::teamwork_mcp::{TeamworkMcpToolCatalog, ToolCallContext};

const SERVER_INSTRUCTIONS: &str = "This MCP server exposes the Minos teamwork room bound to the current agent session. Use list_room_messages to read recent room history before answering when room context matters. Use delegate_to_agent for focused work assigned to another Minos agent, then get_delegation_status or cancel_delegation when tracking that work. Use ask_user_question for concise non-blocking user clarification and check_user_feedback before acting on the answer. Use post_room_update only for concise user-visible room updates. Use react_to_message for lightweight emoji acknowledgement on specific room messages.";

pub use crate::teamwork_mcp::TeamworkMcpPermissions as McpToolPermissions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub socket_path: PathBuf,
    pub room_id: String,
    pub source_agent: Option<AgentName>,
    pub permissions: McpToolPermissions,
}

pub async fn serve_stdio(config: McpServerConfig) -> Result<()> {
    let socket_path = config.socket_path.clone();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
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
                )?;
                continue;
            }
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let response = handle_request(&config, &socket_path, id, request).await;
        write_json(&mut stdout, &response?)?;
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
                Err(error) => Ok(error_response(id, -32602, &error.to_string())),
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

async fn handle_tool_call(
    config: &McpServerConfig,
    socket_path: &Path,
    id: Value,
    params: Value,
) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let request = TeamworkMcpToolCatalog::default_catalog().socket_request_for_call(
        config.permissions,
        ToolCallContext {
            room_id: config.room_id.clone(),
            source_agent: config.source_agent,
        },
        name,
        args,
    )?;
    let result = send_socket_request(socket_path, request).await?;
    tool_response(id, result)
}

async fn send_socket_request(
    socket_path: &Path,
    request: SocketRequest,
) -> Result<serde_json::Value> {
    use std::os::unix::net::UnixStream;
    let stream = UnixStream::connect(socket_path).with_context(|| {
        format!(
            "failed to connect to Minos socket at {}",
            socket_path.display()
        )
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let result = tokio::task::spawn_blocking(move || -> Result<SocketResponse> {
        use crate::mcp_socket::read_response_frame;
        use std::io::Write;
        let payload = serde_json::to_vec(&request)?;
        let len = u32::try_from(payload.len()).context("request payload too large")?;
        let mut buf = Vec::with_capacity(4 + payload.len());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&payload);
        {
            let mut stream_ref = &stream;
            stream_ref.write_all(&buf)?;
            stream_ref.flush()?;
        }
        let mut stream_ref = &stream;
        let response = read_response_frame::<&std::os::unix::net::UnixStream>(&mut stream_ref)?;
        match response {
            None => anyhow::bail!("Minos socket closed before response"),
            Some(SocketResponse::Ok { data }) => Ok(SocketResponse::Ok { data }),
            Some(SocketResponse::Error { message }) => {
                anyhow::bail!("Minos socket error: {message}")
            }
            Some(SocketResponse::Pong) => Ok(SocketResponse::Pong),
        }
    })
    .await??;
    match result {
        SocketResponse::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
        SocketResponse::Error { message } => anyhow::bail!("{message}"),
        SocketResponse::Pong => Ok(serde_json::Value::Null),
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

fn write_json(stdout: &mut std::io::Stdout, response: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdout, response)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
