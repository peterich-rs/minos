use crate::error::TranslationError;
use crate::message::{MessageRole, UiEventMessage};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct ClaudeTranslatorState {
    thread_id: String,
    claude_session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    open_user_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    tool_calls: HashMap<String, OpenClaudeToolCall>,
}

struct OpenClaudeToolCall {
    message_id: String,
    name: String,
    args_buf: String,
}

impl ClaudeTranslatorState {
    #[must_use]
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            claude_session_id: None,
            open_assistant_message_id: None,
            open_user_message_id: None,
            emitted_message_ids: HashSet::new(),
            tool_calls: HashMap::new(),
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn translate(
    state: &mut ClaudeTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let event_type = raw
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| TranslationError::Malformed {
            reason: "missing type".into(),
        })?;

    match event_type {
        "system" => {
            let subtype = raw.get("subtype").and_then(Value::as_str).unwrap_or("");
            if subtype == "init" {
                if let Some(sid) = raw.get("session_id").and_then(Value::as_str) {
                    state.claude_session_id = Some(sid.to_string());
                }
                return Ok(vec![UiEventMessage::ThreadOpened {
                    thread_id: state.thread_id.clone(),
                    agent: AgentName::Claude,
                    title: None,
                    opened_at_ms: 0,
                }]);
            }
            Ok(vec![UiEventMessage::Raw {
                kind: format!("claude/system/{subtype}"),
                payload_json: serde_json::to_string(raw).unwrap_or_default(),
            }])
        }
        "assistant" => {
            let message = raw.get("message").cloned().unwrap_or(Value::Null);
            let msg_id = message
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            let mut events = Vec::new();

            if !msg_id.is_empty() && state.emitted_message_ids.insert(msg_id.clone()) {
                state.open_assistant_message_id = Some(msg_id.clone());
                events.push(UiEventMessage::MessageStarted {
                    message_id: msg_id.clone(),
                    role: MessageRole::Assistant,
                    started_at_ms: 0,
                });
            }

            if let Some(Value::Array(content)) = message.get("content") {
                for block in content {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        let tool_id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let tool_name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let tool_input = block
                            .get("input")
                            .and_then(|v| serde_json::to_string(v).ok())
                            .unwrap_or_default();
                        if !tool_id.is_empty() {
                            state.tool_calls.insert(
                                tool_id.clone(),
                                OpenClaudeToolCall {
                                    message_id: msg_id.clone(),
                                    name: tool_name.clone(),
                                    args_buf: tool_input.clone(),
                                },
                            );
                            events.push(UiEventMessage::ToolCallPlaced {
                                message_id: msg_id.clone(),
                                tool_call_id: tool_id,
                                name: tool_name,
                                args_json: tool_input,
                            });
                        }
                    }
                }
            }

            let delta_type = raw
                .get("delta")
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");

            let Some(ref mid) = state.open_assistant_message_id else {
                return Ok(events);
            };

            match delta_type {
                "text_delta" => {
                    let text = raw
                        .get("delta")
                        .and_then(|d| d.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    events.push(UiEventMessage::TextDelta {
                        message_id: mid.clone(),
                        text,
                    });
                }
                "thinking_delta" => {
                    let text = raw
                        .get("delta")
                        .and_then(|d| d.get("thinking"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    events.push(UiEventMessage::ReasoningDelta {
                        message_id: mid.clone(),
                        text,
                    });
                }
                "input_json_delta" => {
                    if let Some(delta) = raw
                        .get("delta")
                        .and_then(|d| d.get("partial_json"))
                        .and_then(Value::as_str)
                    {
                        let tool_use_id = raw
                            .get("delta")
                            .and_then(|d| d.get("tool_use_id"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if let Some(tc) = state.tool_calls.get_mut(tool_use_id) {
                            tc.args_buf.push_str(delta);
                        }
                    }
                }
                _ => {}
            }

            Ok(events)
        }
        "user" => {
            let message = raw.get("message").cloned().unwrap_or(Value::Null);
            let msg_id = message
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            let mut events = Vec::new();

            if !msg_id.is_empty() && state.emitted_message_ids.insert(msg_id.clone()) {
                state.open_user_message_id = Some(msg_id.clone());
                events.push(UiEventMessage::MessageStarted {
                    message_id: msg_id.clone(),
                    role: MessageRole::User,
                    started_at_ms: 0,
                });
            }

            if let Some(Value::Array(content)) = message.get("content") {
                for block in content {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        let text = block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if !text.is_empty() && !msg_id.is_empty() {
                            events.push(UiEventMessage::TextDelta {
                                message_id: msg_id.clone(),
                                text,
                            });
                        }
                    }
                }
            }

            Ok(events)
        }
        "tool_result" => {
            let tool_use_id = raw
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let output = raw
                .get("content")
                .and_then(|c| {
                    if let Some(Value::Array(arr)) = c.get("content") {
                        arr.iter()
                            .filter_map(|b| {
                                if b.get("type").and_then(Value::as_str) == Some("text") {
                                    b.get("text").and_then(Value::as_str)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                            .into()
                    } else if let Some(s) = c.as_str() {
                        Some(s.to_string())
                    } else {
                        c.as_str().map(str::to_string)
                    }
                })
                .or_else(|| {
                    raw.get("content")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();

            let is_error = raw
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if let Some(_tc) = state.tool_calls.get(&tool_use_id) {
                Ok(vec![UiEventMessage::ToolCallCompleted {
                    tool_call_id: tool_use_id,
                    output,
                    is_error,
                }])
            } else {
                Ok(vec![])
            }
        }
        "result" => {
            let mut events = Vec::new();
            if let Some(mid) = state.open_assistant_message_id.take() {
                events.push(UiEventMessage::MessageCompleted {
                    message_id: mid,
                    finished_at_ms: 0,
                });
            }
            let is_error = raw
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_error {
                let code = raw
                    .get("subtype")
                    .and_then(Value::as_str)
                    .unwrap_or("claude_error")
                    .to_string();
                let message = raw
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or("claude reported an error")
                    .to_string();
                events.push(UiEventMessage::Error {
                    code,
                    message,
                    message_id: state.open_assistant_message_id.clone(),
                });
            }
            Ok(events)
        }
        "error" => {
            let error_obj = raw.get("error").cloned().unwrap_or(Value::Null);
            let code = error_obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("claude_error")
                .to_string();
            let message = error_obj
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("claude reported an error")
                .to_string();
            Ok(vec![UiEventMessage::Error {
                code,
                message,
                message_id: state
                    .open_assistant_message_id
                    .clone()
                    .or_else(|| state.open_user_message_id.clone()),
            }])
        }
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("claude/{other}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::*;
    use pretty_assertions::assert_eq;

    fn val(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn system_init_emits_thread_opened() {
        let mut state = ClaudeTranslatorState::new("thr_x".into());
        let raw = val(r#"{
            "type":"system",
            "subtype":"init",
            "session_id":"sess_1",
            "tools":[]
        }"#);
        let out = translate(&mut state, &raw).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0] {
            UiEventMessage::ThreadOpened {
                thread_id,
                agent,
                opened_at_ms,
                ..
            } => {
                assert_eq!(thread_id, "thr_x");
                assert_eq!(*agent, AgentName::Claude);
                assert_eq!(*opened_at_ms, 0);
            }
            _ => panic!("unexpected {:?}", out[0]),
        }
        assert_eq!(state.claude_session_id.as_deref(), Some("sess_1"));
    }

    #[test]
    fn assistant_message_start_and_text_delta() {
        let mut s = ClaudeTranslatorState::new("thr".into());

        let o1 = translate(
            &mut s,
            &val(r#"{
                "type":"assistant",
                "message":{"id":"msg_1","content":[]},
                "delta":{"type":"text_delta","text":"Hello"}
            }"#),
        )
        .unwrap();
        assert_eq!(o1.len(), 2);
        assert!(matches!(
            &o1[0],
            UiEventMessage::MessageStarted {
                role: MessageRole::Assistant,
                message_id,
                ..
            } if message_id == "msg_1"
        ));
        assert!(matches!(
            &o1[1],
            UiEventMessage::TextDelta {
                message_id,
                text,
            } if message_id == "msg_1" && text == "Hello"
        ));
    }

    #[test]
    fn result_emits_message_completed() {
        let mut s = ClaudeTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                "type":"assistant",
                "message":{"id":"msg_1","content":[]},
                "delta":{"type":"text_delta","text":"hi"}
            }"#),
        )
        .unwrap();

        let out = translate(
            &mut s,
            &val(r#"{"type":"result","subtype":"success","result":"done","is_error":false}"#),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms: 0,
            } if message_id == "msg_1"
        ));
    }

    #[test]
    fn tool_use_maps_to_tool_call_placed_and_completed() {
        let mut s = ClaudeTranslatorState::new("thr".into());

        let out = translate(
            &mut s,
            &val(r#"{
                "type":"assistant",
                "message":{
                    "id":"msg_1",
                    "content":[
                        {"type":"tool_use","id":"tu_1","name":"bash","input":{"command":"ls"}}
                    ]
                },
                "delta":{"type":"text_delta","text":""}
            }"#),
        )
        .unwrap();

        let placed = out
            .iter()
            .find(|e| matches!(e, UiEventMessage::ToolCallPlaced { .. }));
        assert!(placed.is_some());
        match placed.unwrap() {
            UiEventMessage::ToolCallPlaced {
                tool_call_id,
                name,
                args_json,
                ..
            } => {
                assert_eq!(tool_call_id, "tu_1");
                assert_eq!(name, "bash");
                assert!(args_json.contains("command"));
            }
            _ => panic!(),
        }

        let completed = translate(
            &mut s,
            &val(r#"{
                "type":"tool_result",
                "tool_use_id":"tu_1",
                "content":"file1\nfile2",
                "is_error":false
            }"#),
        )
        .unwrap();
        assert_eq!(completed.len(), 1);
        assert!(matches!(
            &completed[0],
            UiEventMessage::ToolCallCompleted {
                tool_call_id,
                is_error: false,
                ..
            } if tool_call_id == "tu_1"
        ));
    }

    #[test]
    fn error_event_maps_to_ui_error() {
        let mut s = ClaudeTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(r#"{
                "type":"error",
                "error":{"type":"overloaded_error","message":"Too many requests"}
            }"#),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::Error {
                code,
                message,
                message_id: None,
            } if code == "overloaded_error" && message == "Too many requests"
        ));
    }

    #[test]
    fn unknown_event_falls_through_to_raw() {
        let mut s = ClaudeTranslatorState::new("thr".into());
        let raw = val(r#"{"type":"custom_event","data":"something"}"#);
        let out = translate(&mut s, &raw).unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::Raw { kind, .. } if kind == "claude/custom_event"
        ));
    }
}
