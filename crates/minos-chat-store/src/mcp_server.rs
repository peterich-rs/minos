use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde_json::{json, Map, Value};

use crate::mcp_socket::{SocketRequest, SocketResponse};
use crate::ChatStore;

const SERVER_INSTRUCTIONS: &str = "This MCP server exposes the Minos teamwork chat room bound to the current agent session. Use list_chat_messages to read recent room history before answering when the user or another agent refers to chat context, coordination, previous replies, or current room state. Use request_agent_help to ask another Minos agent in the same room for focused help. Use mention_user only for concise user-visible updates that should appear in the room.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpToolPermissions {
    pub read_chat: bool,
    pub mention_agent: bool,
    pub mention_user: bool,
}

impl Default for McpToolPermissions {
    fn default() -> Self {
        Self {
            read_chat: true,
            mention_agent: true,
            mention_user: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub socket_path: PathBuf,
    pub db_path: PathBuf,
    pub room_id: String,
    pub source_agent: Option<AgentName>,
    pub permissions: McpToolPermissions,
}

pub async fn serve_stdio(config: McpServerConfig) -> Result<()> {
    let store = ChatStore::open(&config.db_path).await?;
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
        let response = handle_request(&store, &config, &socket_path, id, request).await;
        write_json(&mut stdout, &response?)?;
    }
    Ok(())
}

async fn handle_request(
    store: &ChatStore,
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
                    "name": "minos-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": SERVER_INSTRUCTIONS
            }
        })),
        "tools/list" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tools_for_permissions(config.permissions) }
        })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            match handle_tool_call(store, config, socket_path, id.clone(), params).await {
                Ok(response) => Ok(response),
                Err(error) => Ok(error_response(id, -32602, &error.to_string())),
            }
        }
        "ping" => Ok(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        _ => Ok(error_response(id, -32601, &format!("unsupported MCP method: {method}"))),
    }
}

async fn handle_tool_call(
    store: &ChatStore,
    config: &McpServerConfig,
    socket_path: &Path,
    id: Value,
    params: Value,
) -> Result<Value> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    match name {
        "list_chat_messages" => {
            ensure_tool_enabled(config.permissions.read_chat, "list_chat_messages")?;
            let room_id = bound_room_id(&args, config, "list_chat_messages")?;
            let before_seq = args.get("before_seq").and_then(Value::as_u64);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            let page = store.list_messages_desc(room_id, before_seq, limit).await?;
            tool_response(id, serde_json::to_value(&page)?)
        }
        "request_agent_help" => {
            ensure_tool_enabled(config.permissions.mention_agent, "request_agent_help")?;
            let room_id = bound_room_id(&args, config, "request_agent_help")?;
            let target_agent = parse_agent_arg(required_string_arg(&args, "target_agent")?)?;
            let prompt = required_string_arg(&args, "prompt")?.trim().to_owned();
            anyhow::ensure!(!prompt.is_empty(), "prompt must not be empty");
            let request = SocketRequest::RequestAgentHelp {
                room_id: room_id.to_owned(),
                source_agent: config.source_agent.map(|a| a.bin_name().to_owned()),
                target_agent: target_agent.bin_name().to_owned(),
                prompt,
            };
            let result = send_socket_request(socket_path, request).await?;
            tool_response(id, result)
        }
        "mention_user" => {
            ensure_tool_enabled(config.permissions.mention_user, "mention_user")?;
            let room_id = bound_room_id(&args, config, "mention_user")?;
            let message = required_string_arg(&args, "message")?.trim().to_owned();
            anyhow::ensure!(!message.is_empty(), "message must not be empty");
            let request = SocketRequest::MentionUser {
                room_id: room_id.to_owned(),
                source_agent: config.source_agent.map(|a| a.bin_name().to_owned()),
                message,
            };
            let result = send_socket_request(socket_path, request).await?;
            tool_response(id, result)
        }
        _ => anyhow::bail!("unknown Minos MCP tool: {name}"),
    }
}

async fn send_socket_request(
    socket_path: &Path,
    request: SocketRequest,
) -> Result<serde_json::Value> {
    use std::os::unix::net::UnixStream;
    let stream = UnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to Minos socket at {}", socket_path.display()))?;
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
            Some(SocketResponse::Error { message }) => anyhow::bail!("Minos socket error: {message}"),
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

fn tools_for_permissions(permissions: McpToolPermissions) -> Vec<Value> {
    let mut tools = Vec::new();
    if permissions.read_chat {
        tools.push(list_chat_messages_tool());
    }
    if permissions.mention_agent {
        tools.push(request_agent_help_tool());
    }
    if permissions.mention_user {
        tools.push(mention_user_tool());
    }
    tools
}

fn list_chat_messages_tool() -> Value {
    json!({
        "name": "list_chat_messages",
        "description": "Read messages from the Minos chat room bound to this MCP server, newest-first with cursor pagination.",
        "inputSchema": {
            "type": "object",
            "properties": pagination_properties()
        }
    })
}

fn request_agent_help_tool() -> Value {
    let mut properties = Map::new();
    properties.insert(
        "target_agent".into(),
        json!({
            "type": "string",
            "enum": agent_name_values(),
            "description": "The Minos agent to mention in this room."
        }),
    );
    properties.insert(
        "prompt".into(),
        json!({
            "type": "string",
            "description": "The prompt to send to the target agent."
        }),
    );
    json!({
        "name": "request_agent_help",
        "description": "Ask another Minos agent in this chat room to help with a prompt.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["target_agent", "prompt"]
        }
    })
}

fn mention_user_tool() -> Value {
    let mut properties = Map::new();
    properties.insert(
        "message".into(),
        json!({
            "type": "string",
            "description": "A concise user-visible message to post in the room."
        }),
    );
    json!({
        "name": "mention_user",
        "description": "Post a user-visible mention into this Minos chat room.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["message"]
        }
    })
}

fn pagination_properties() -> Map<String, Value> {
    Map::from_iter([
        (
            "before_seq".into(),
            json!({
                "type": "integer",
                "minimum": 1,
                "description": "Return messages with seq lower than this cursor."
            }),
        ),
        (
            "limit".into(),
            json!({
                "type": "integer",
                "minimum": 1,
                "maximum": 500,
                "description": "Maximum messages to return. Defaults to 100."
            }),
        ),
    ])
}

fn bound_room_id<'a>(
    args: &Value,
    config: &'a McpServerConfig,
    tool_name: &str,
) -> Result<&'a str> {
    anyhow::ensure!(
        args.get("room_id").is_none(),
        "{tool_name} does not accept room_id; this MCP server is bound to a single room at startup"
    );
    Ok(config.room_id.as_str())
}

fn required_string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .with_context(|| format!("{name} is required"))
}

fn parse_agent_arg(value: &str) -> Result<AgentName> {
    let normalized = value.trim().to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
        .with_context(|| format!("unknown agent: {value}"))
}

fn agent_name_values() -> Vec<&'static str> {
    AgentName::all().iter().map(|agent| agent.bin_name()).collect()
}

fn ensure_tool_enabled(enabled: bool, name: &str) -> Result<()> {
    anyhow::ensure!(enabled, "{name} is disabled by this MCP server policy");
    Ok(())
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
