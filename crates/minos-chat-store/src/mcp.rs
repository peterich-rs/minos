use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::ChatStore;

pub async fn serve_stdio(db_path: Option<PathBuf>, default_room_id: Option<String>) -> Result<()> {
    let db_path = match db_path {
        Some(path) => path,
        None => crate::default_db_path()?,
    };
    let store = ChatStore::open(&db_path).await?;
    serve_stdio_with_store(store, default_room_id).await
}

pub async fn serve_stdio_with_store(
    store: ChatStore,
    default_room_id: Option<String>,
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
        let response = handle_request(&store, default_room_id.as_deref(), id, request).await;
        write_message(&mut stdout, &response?)?;
    }
    Ok(())
}

async fn handle_request(
    store: &ChatStore,
    default_room_id: Option<&str>,
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
                "tools": [{
                    "name": "list_chat_messages",
                    "description": "Read Minos chat room messages newest-first with cursor pagination.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "room_id": {
                                "type": "string",
                                "description": "Chat room id. Optional when this MCP server was started with --default-room-id."
                            },
                            "before_seq": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Return messages with seq lower than this cursor."
                            },
                            "limit": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 500,
                                "description": "Maximum messages to return. Defaults to 100."
                            }
                        }
                    }
                }]
            }
        })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if name != "list_chat_messages" {
                return Ok(error_response(
                    id,
                    -32602,
                    &format!("unknown Minos chat MCP tool: {name}"),
                ));
            }
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let explicit_room_id = args.get("room_id").and_then(Value::as_str);
            let room_id = explicit_room_id
                .or(default_room_id)
                .context("list_chat_messages requires room_id")?;
            let before_seq = args.get("before_seq").and_then(Value::as_u64);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            let mut page = store.list_messages_desc(room_id, before_seq, limit).await?;
            if explicit_room_id.is_none() && before_seq.is_none() && page.messages.is_empty() {
                if let Some(fallback_room_id) = store.most_recent_non_empty_room_id().await? {
                    if fallback_room_id != room_id {
                        page = store
                            .list_messages_desc(&fallback_room_id, before_seq, limit)
                            .await?;
                    }
                }
            }
            let payload = serde_json::to_value(&page)?;
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
    use minos_domain::AgentName;

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
            Some("room-main"),
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
    async fn tools_call_without_explicit_room_falls_back_to_recent_non_empty_room() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChatStore::open(&tmp.path().join("chat.sqlite"))
            .await
            .unwrap();
        store
            .ensure_room("room-empty", "empty", "/tmp/empty")
            .await
            .unwrap();
        store
            .ensure_room("room-main", "main", "/tmp/ws")
            .await
            .unwrap();
        store
            .append_message(
                "room-main",
                NewChatMessage {
                    message_id: None,
                    created_at_ms: 10,
                    event_type: ChatMessageType::UserMessage,
                    text: "fallback message".into(),
                    agent: Some(AgentName::Codex),
                    thread_id: None,
                    thread_short_id: None,
                    workspace_root: Some("/tmp/ws".into()),
                },
            )
            .await
            .unwrap();

        let response = handle_request(
            &store,
            Some("room-empty"),
            json!(1),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "list_chat_messages",
                    "arguments": {}
                }
            }),
        )
        .await
        .unwrap();

        assert_eq!(
            response["result"]["structuredContent"]["room_id"],
            json!("room-main")
        );
        assert_eq!(
            response["result"]["structuredContent"]["messages"][0]["text"],
            json!("fallback message")
        );
    }
}
