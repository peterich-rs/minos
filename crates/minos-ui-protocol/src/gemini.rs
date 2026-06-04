use crate::error::TranslationError;
use crate::message::{MessageRole, ThreadEndReason, UiEventMessage};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[allow(dead_code)]
pub struct GeminiTranslatorState {
    thread_id: String,
    session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    open_user_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    tool_calls: HashMap<String, OpenGeminiToolCall>,
}

#[allow(dead_code)]
struct OpenGeminiToolCall {
    message_id: String,
    name: String,
}

impl GeminiTranslatorState {
    #[must_use]
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            session_id: None,
            open_assistant_message_id: None,
            open_user_message_id: None,
            emitted_message_ids: HashSet::new(),
            tool_calls: HashMap::new(),
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn translate(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let kind = raw
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| TranslationError::Malformed {
            reason: "missing kind field".into(),
        })?;

    match kind {
        "acp_notification" => translate_acp_notification(state, raw),
        "acp_server_request" => translate_acp_server_request(state, raw),
        "acp_closed" => {
            let mut events = Vec::new();
            if let Some(mid) = state.open_assistant_message_id.take() {
                events.push(UiEventMessage::MessageCompleted {
                    message_id: mid,
                    finished_at_ms: chrono::Utc::now().timestamp_millis(),
                });
            }
            events.push(UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::AgentDone,
                closed_at_ms: chrono::Utc::now().timestamp_millis(),
            });
            Ok(events)
        }
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/{other}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::unnecessary_wraps)]
fn translate_acp_notification(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let params = raw.get("params").cloned().unwrap_or(Value::Null);
    let update = params.get("update").cloned().unwrap_or(Value::Null);
    let session_update = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("");

    match session_update {
        "agent_message_chunk" => {
            let content = update.get("content").cloned().unwrap_or(Value::Null);
            let content_type = content.get("type").and_then(Value::as_str).unwrap_or("text");

            let mid = state
                .open_assistant_message_id
                .clone()
                .unwrap_or_else(|| {
                    let id = format!("msg_{}", Uuid::new_v4());
                    state.open_assistant_message_id = Some(id.clone());
                    id
                });

            let mut events = Vec::new();

            if state.emitted_message_ids.insert(mid.clone()) {
                events.push(UiEventMessage::MessageStarted {
                    message_id: mid.clone(),
                    role: MessageRole::Assistant,
                    started_at_ms: 0,
                });
            }

            match content_type {
                "text" => {
                    let text = content
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::TextDelta {
                            message_id: mid,
                            text,
                        });
                    }
                }
                "thought" => {
                    let text = content
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::ReasoningDelta {
                            message_id: mid,
                            text,
                        });
                    }
                }
                _ => {
                    events.push(UiEventMessage::Raw {
                        kind: format!("gemini/agent_message_chunk/{content_type}"),
                        payload_json: serde_json::to_string(&update).unwrap_or_default(),
                    });
                }
            }

            Ok(events)
        }
        "tool_call" => {
            let tool_call_id = update
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let title = update
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let kind = update
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("other")
                .to_string();

            let mid = state
                .open_assistant_message_id
                .clone()
                .unwrap_or_default();

            if tool_call_id.is_empty() {
                Ok(vec![])
            } else {
                state.tool_calls.insert(
                    tool_call_id.clone(),
                    OpenGeminiToolCall {
                        message_id: mid.clone(),
                        name: title.clone(),
                    },
                );
                Ok(vec![UiEventMessage::ToolCallPlaced {
                    message_id: mid,
                    tool_call_id,
                    name: format!("{kind}: {title}"),
                    args_json: String::new(),
                }])
            }
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let status = update
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            if status == "completed" {
                let content = update
                    .get("content")
                    .and_then(|c| {
                        if let Some(arr) = c.as_array() {
                            arr.iter()
                                .filter_map(|item| {
                                    item.get("content")
                                        .and_then(|inner| inner.get("text"))
                                        .and_then(Value::as_str)
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                                .into()
                        } else {
                            c.as_str().map(str::to_string)
                        }
                    })
                    .unwrap_or_default();

                state.tool_calls.remove(&tool_call_id);
                Ok(vec![UiEventMessage::ToolCallCompleted {
                    tool_call_id,
                    output: content,
                    is_error: false,
                }])
            } else {
                Ok(vec![])
            }
        }
        "plan" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/plan".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "thought" => {
            let content = update.get("content").cloned().unwrap_or(Value::Null);
            let text = content
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mid = state
                .open_assistant_message_id
                .clone()
                .unwrap_or_default();
            if text.is_empty() {
                Ok(vec![])
            } else {
                Ok(vec![UiEventMessage::ReasoningDelta {
                    message_id: mid,
                    text,
                }])
            }
        }
        "current_mode_update" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/mode_change".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "available_commands_update" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/commands_update".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "session_info_update" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/session_info".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "" => Ok(vec![]),
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/acp/{other}"),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn translate_acp_server_request(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let method = raw
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("");

    match method {
        "session/request_permission" => {
            let params = raw.get("params").cloned().unwrap_or(Value::Null);
            let tool_call = params.get("tool_call").cloned().unwrap_or(Value::Null);
            let tool_call_id = tool_call
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let title = tool_call
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Tool execution")
                .to_string();
            let kind = tool_call
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("other")
                .to_string();

            let mid = state
                .open_assistant_message_id
                .clone()
                .unwrap_or_default();

            if tool_call_id.is_empty() {
                Ok(vec![UiEventMessage::Raw {
                    kind: "gemini/permission_request".into(),
                    payload_json: serde_json::to_string(raw).unwrap_or_default(),
                }])
            } else {
                state.tool_calls.insert(
                    tool_call_id.clone(),
                    OpenGeminiToolCall {
                        message_id: mid.clone(),
                        name: title.clone(),
                    },
                );
                Ok(vec![UiEventMessage::ToolCallPlaced {
                    message_id: mid,
                    tool_call_id,
                    name: format!("{kind}: {title}"),
                    args_json: String::new(),
                }])
            }
        }
        "fs/read_text_file" | "fs/write_text_file" => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/{method}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
        _ => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/server_request/{method}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::*;

    fn val(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn acp_notification_agent_message_chunk_text() {
        let mut s = GeminiTranslatorState::new("thr_x".into());
        let raw = val(r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}}}"#);
        let out = translate(&mut s, &raw).unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::MessageStarted { role: MessageRole::Assistant, .. }))
        );
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::TextDelta { text, .. } if text == "Hello"))
        );
    }

    #[test]
    fn acp_notification_thought_emits_reasoning_delta() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":""}}}}"#,
            ),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"thought","content":{"type":"text","text":"thinking..."}}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ReasoningDelta { text, .. } if text == "thinking..."))
        );
    }

    #[test]
    fn acp_notification_tool_call_placed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":""}}}}"#,
            ),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","tool_call_id":"tc_1","title":"Edit file","kind":"edit","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ToolCallPlaced { tool_call_id, name, .. } if tool_call_id == "tc_1" && name.contains("Edit file")))
        );
    }

    #[test]
    fn acp_notification_tool_call_update_completed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":""}}}}"#,
            ),
        )
        .unwrap();
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","tool_call_id":"tc_1","title":"Edit","kind":"edit","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call_update","tool_call_id":"tc_1","status":"completed","content":null}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ToolCallCompleted { tool_call_id, is_error: false, .. } if tool_call_id == "tc_1"))
        );
    }

    #[test]
    fn acp_closed_emits_thread_closed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}}}"#,
            ),
        )
        .unwrap();
        let out = translate(&mut s, &val(r#"{"kind":"acp_closed"}"#)).unwrap();
        assert!(out.iter().any(|e| matches!(e, UiEventMessage::MessageCompleted { .. })));
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ThreadClosed { reason: ThreadEndReason::AgentDone, .. }))
        );
    }

    #[test]
    fn acp_notification_plan_emits_raw() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"plan","entries":[{"content":"Step 1","priority":"high","status":"pending"}]}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::Raw { kind, .. } if kind == "gemini/plan"))
        );
    }

    #[test]
    fn acp_server_request_permission_emits_tool_call_placed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":""}}}}"#,
            ),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_server_request","method":"session/request_permission","params":{"tool_call":{"tool_call_id":"tc_perm","title":"Run shell","kind":"terminal","status":"pending"},"options":[]}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ToolCallPlaced { tool_call_id, name, .. } if tool_call_id == "tc_perm" && name.contains("Run shell")))
        );
    }

    #[test]
    fn unknown_session_update_falls_through_to_raw() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"custom_event","data":"something"}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::Raw { kind, .. } if kind == "gemini/acp/custom_event"))
        );
    }
}
