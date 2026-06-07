use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde_json::{json, Map, Value};

use crate::{ChatMcpCommandKind, ChatStore, NewChatMcpCommand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatMcpToolPermissions {
    pub read_chat: bool,
    pub mention_agent: bool,
    pub mention_user: bool,
    pub allow_any_room: bool,
}

impl Default for ChatMcpToolPermissions {
    fn default() -> Self {
        Self {
            read_chat: true,
            mention_agent: true,
            mention_user: true,
            allow_any_room: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatMcpServerConfig {
    pub default_room_id: Option<String>,
    pub source_agent: Option<AgentName>,
    pub permissions: ChatMcpToolPermissions,
}

pub async fn serve_stdio(db_path: Option<PathBuf>, default_room_id: Option<String>) -> Result<()> {
    serve_stdio_with_config(
        db_path,
        ChatMcpServerConfig {
            default_room_id,
            ..ChatMcpServerConfig::default()
        },
    )
    .await
}

pub async fn serve_stdio_with_config(
    db_path: Option<PathBuf>,
    config: ChatMcpServerConfig,
) -> Result<()> {
    let db_path = match db_path {
        Some(path) => path,
        None => crate::default_db_path()?,
    };
    let store = ChatStore::open(&db_path).await?;
    serve_stdio_with_store_config(store, config).await
}

pub async fn serve_stdio_with_store(
    store: ChatStore,
    default_room_id: Option<String>,
) -> Result<()> {
    serve_stdio_with_store_config(
        store,
        ChatMcpServerConfig {
            default_room_id,
            ..ChatMcpServerConfig::default()
        },
    )
    .await
}

pub async fn serve_stdio_with_store_config(
    store: ChatStore,
    config: ChatMcpServerConfig,
) -> Result<()> {
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
                write_message(
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
        let response = handle_request(&store, &config, id, request).await;
        write_message(&mut stdout, &response?)?;
    }
    Ok(())
}

async fn handle_request(
    store: &ChatStore,
    config: &ChatMcpServerConfig,
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
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "minos-chat",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })),
        "tools/list" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tools_for_permissions(config.permissions)
            }
        })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            let error_id = id.clone();
            match handle_tool_call(store, config, id, params).await {
                Ok(response) => Ok(response),
                Err(error) => Ok(error_response(error_id, -32602, &error.to_string())),
            }
        }
        "ping" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),
        _ => Ok(error_response(
            id,
            -32601,
            &format!("unsupported MCP method: {method}"),
        )),
    }
}

async fn handle_tool_call(
    store: &ChatStore,
    config: &ChatMcpServerConfig,
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

    match name {
        "list_chat_messages" => {
            ensure_tool_enabled(config.permissions.read_chat, "list_chat_messages")?;
            let room_id = resolve_room_id(&args, config, "list_chat_messages")?;
            let before_seq = args.get("before_seq").and_then(Value::as_u64);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            let page = store
                .list_messages_desc(&room_id, before_seq, limit)
                .await?;
            tool_response(id, serde_json::to_value(&page)?)
        }
        "request_agent_help" => {
            ensure_tool_enabled(config.permissions.mention_agent, "request_agent_help")?;
            let room_id = resolve_room_id(&args, config, "request_agent_help")?;
            let target_agent = parse_agent_arg(required_string_arg(&args, "target_agent")?)?;
            let prompt = required_string_arg(&args, "prompt")?.trim().to_owned();
            anyhow::ensure!(!prompt.is_empty(), "prompt must not be empty");
            let command = store
                .enqueue_mcp_command(
                    &room_id,
                    NewChatMcpCommand {
                        kind: ChatMcpCommandKind::MentionAgent,
                        source_agent: config.source_agent,
                        target_agent: Some(target_agent),
                        body: prompt,
                    },
                )
                .await?;
            tool_response(id, serde_json::to_value(&command)?)
        }
        "mention_user" => {
            ensure_tool_enabled(config.permissions.mention_user, "mention_user")?;
            let room_id = resolve_room_id(&args, config, "mention_user")?;
            let message = required_string_arg(&args, "message")?.trim().to_owned();
            anyhow::ensure!(!message.is_empty(), "message must not be empty");
            let command = store
                .enqueue_mcp_command(
                    &room_id,
                    NewChatMcpCommand {
                        kind: ChatMcpCommandKind::MentionUser,
                        source_agent: config.source_agent,
                        target_agent: None,
                        body: message,
                    },
                )
                .await?;
            tool_response(id, serde_json::to_value(&command)?)
        }
        _ => anyhow::bail!("unknown Minos chat MCP tool: {name}"),
    }
}

fn tools_for_permissions(permissions: ChatMcpToolPermissions) -> Vec<Value> {
    let mut tools = Vec::new();
    if permissions.read_chat {
        tools.push(list_chat_messages_tool(permissions.allow_any_room));
    }
    if permissions.mention_agent {
        tools.push(request_agent_help_tool(permissions.allow_any_room));
    }
    if permissions.mention_user {
        tools.push(mention_user_tool(permissions.allow_any_room));
    }
    tools
}

fn list_chat_messages_tool(allow_any_room: bool) -> Value {
    let mut properties = pagination_properties();
    insert_room_property(&mut properties, allow_any_room);
    json!({
        "name": "list_chat_messages",
        "description": "Read messages from the Minos chat room bound to this MCP server, newest-first with cursor pagination.",
        "inputSchema": {
            "type": "object",
            "properties": properties
        }
    })
}

fn request_agent_help_tool(allow_any_room: bool) -> Value {
    let mut properties = Map::new();
    insert_room_property(&mut properties, allow_any_room);
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
        "description": "Ask another Minos agent in this chat room to help with a prompt. The request is queued for the TUI to dispatch.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["target_agent", "prompt"]
        }
    })
}

fn mention_user_tool(allow_any_room: bool) -> Value {
    let mut properties = Map::new();
    insert_room_property(&mut properties, allow_any_room);
    properties.insert(
        "message".into(),
        json!({
            "type": "string",
            "description": "A concise user-visible message to post in the room."
        }),
    );
    json!({
        "name": "mention_user",
        "description": "Post a user-visible mention into this Minos chat room. The message is queued for the TUI to display.",
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

fn insert_room_property(properties: &mut Map<String, Value>, allow_any_room: bool) {
    if allow_any_room {
        properties.insert(
            "room_id".into(),
            json!({
                "type": "string",
                "description": "Chat room id. Only available when this MCP server was started with any-room access."
            }),
        );
    }
}

fn resolve_room_id(args: &Value, config: &ChatMcpServerConfig, tool_name: &str) -> Result<String> {
    let explicit_room_id = args.get("room_id").and_then(Value::as_str);
    if config.permissions.allow_any_room {
        return explicit_room_id
            .or(config.default_room_id.as_deref())
            .map(str::to_owned)
            .with_context(|| format!("{tool_name} requires room_id"));
    }

    let bound_room_id = config
        .default_room_id
        .as_deref()
        .with_context(|| format!("{tool_name} requires a bound --default-room-id"))?;
    if let Some(explicit_room_id) = explicit_room_id {
        anyhow::ensure!(
            explicit_room_id == bound_room_id,
            "{tool_name} may only access the bound chat room"
        );
    }
    Ok(bound_room_id.to_owned())
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
    AgentName::all()
        .iter()
        .map(|agent| agent.bin_name())
        .collect()
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
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn write_message(stdout: &mut std::io::Stdout, response: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdout, response)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessageType, NewChatMessage};

    fn bound_config(room_id: &str) -> ChatMcpServerConfig {
        ChatMcpServerConfig {
            default_room_id: Some(room_id.into()),
            source_agent: Some(AgentName::Codex),
            permissions: ChatMcpToolPermissions::default(),
        }
    }

    #[tokio::test]
    async fn tools_call_returns_newest_first_page() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();
        for text in ["old", "new"] {
            store
                .append_message(
                    "room-main",
                    NewChatMessage {
                        message_id: None,
                        created_at_ms: 10,
                        event_type: ChatMessageType::AgentResult,
                        text: text.into(),
                        agent: Some(AgentName::Codex),
                        thread_id: Some("thread-1".into()),
                        thread_short_id: Some("thread-1".into()),
                        workspace_root: Some("/tmp/ws".into()),
                    },
                )
                .await
                .unwrap();
        }

        let response = handle_request(
            &store,
            &bound_config("room-main"),
            json!(1),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "list_chat_messages",
                    "arguments": {"limit": 1}
                }
            }),
        )
        .await
        .unwrap();
        let messages = response["result"]["structuredContent"]["messages"]
            .as_array()
            .unwrap();
        assert_eq!(messages[0]["text"], "new");
        assert_eq!(response["result"]["structuredContent"]["has_more"], true);
    }

    #[tokio::test]
    async fn bounded_room_policy_rejects_other_room_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();
        store
            .ensure_room("room-other", "other", "/tmp/other")
            .await
            .unwrap();

        let response = handle_request(
            &store,
            &bound_config("room-main"),
            json!(1),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "list_chat_messages",
                    "arguments": {"room_id": "room-other"}
                }
            }),
        )
        .await
        .unwrap();

        assert_eq!(response["error"]["code"], json!(-32602));
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bound chat room"));
    }

    #[tokio::test]
    async fn request_agent_help_enqueues_room_command() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();

        let response = handle_request(
            &store,
            &bound_config("room-main"),
            json!(1),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "request_agent_help",
                    "arguments": {
                        "target_agent": "gemini",
                        "prompt": "review this plan"
                    }
                }
            }),
        )
        .await
        .unwrap();

        assert_eq!(
            response["result"]["structuredContent"]["kind"],
            json!("mention_agent")
        );
        assert_eq!(
            response["result"]["structuredContent"]["source_agent"],
            json!("codex")
        );
        assert_eq!(
            response["result"]["structuredContent"]["target_agent"],
            json!("gemini")
        );

        let claimed = store
            .claim_pending_mcp_commands("room-main", Some(10))
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].kind, ChatMcpCommandKind::MentionAgent);
        assert_eq!(claimed[0].target_agent, Some(AgentName::Gemini));
        assert_eq!(claimed[0].body, "review this plan");
    }

    #[tokio::test]
    async fn tools_list_respects_disabled_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        let config = ChatMcpServerConfig {
            default_room_id: Some("room-main".into()),
            source_agent: Some(AgentName::Codex),
            permissions: ChatMcpToolPermissions {
                read_chat: true,
                mention_agent: false,
                mention_user: true,
                allow_any_room: false,
            },
        };

        let response = handle_request(
            &store,
            &config,
            json!(1),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list"
            }),
        )
        .await
        .unwrap();

        let names = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["list_chat_messages", "mention_user"]);
    }
}
