use crate::error::TranslationError;
use crate::message::{MessageRole, UiEventMessage};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct OpencodeTranslatorState {
    thread_id: String,
    opencode_session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    open_user_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    tool_calls: HashMap<String, OpenOpencodeToolCall>,
}

struct OpenOpencodeToolCall {
    message_id: String,
    name: String,
    args_buf: String,
}

impl OpencodeTranslatorState {
    #[must_use]
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            opencode_session_id: None,
            open_assistant_message_id: None,
            open_user_message_id: None,
            emitted_message_ids: HashSet::new(),
            tool_calls: HashMap::new(),
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn translate(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let event_type = raw
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| TranslationError::Malformed {
            reason: "missing type".into(),
        })?;

    match event_type {
        "session.created" => {
            let session = raw.get("session").cloned().unwrap_or(Value::Null);
            if let Some(sid) = session.get("id").and_then(Value::as_str) {
                state.opencode_session_id = Some(sid.to_string());
            }
            let title = session
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok(vec![UiEventMessage::ThreadOpened {
                thread_id: state.thread_id.clone(),
                agent: AgentName::Opencode,
                title,
                opened_at_ms: 0,
            }])
        }
        "message.updated" => {
            let message = raw.get("message").cloned().unwrap_or(Value::Null);
            let msg_id = message
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("");

            let mut events = Vec::new();

            if !msg_id.is_empty() && state.emitted_message_ids.insert(msg_id.clone()) {
                let ui_role = match role {
                    "assistant" => {
                        state.open_assistant_message_id = Some(msg_id.clone());
                        MessageRole::Assistant
                    }
                    "user" => {
                        state.open_user_message_id = Some(msg_id.clone());
                        MessageRole::User
                    }
                    _ => MessageRole::System,
                };
                events.push(UiEventMessage::MessageStarted {
                    message_id: msg_id.clone(),
                    role: ui_role,
                    started_at_ms: 0,
                });
            }

            if let Some(Value::Array(parts)) = message.get("parts") {
                for part in parts {
                    let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
                    match part_type {
                        "text" => {
                            let text = part
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
                        "tool-call" => {
                            let tool_id = part
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let tool_name = part
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let tool_args = part
                                .get("args")
                                .and_then(|v| serde_json::to_string(v).ok())
                                .unwrap_or_default();
                            if !tool_id.is_empty() {
                                state.tool_calls.insert(
                                    tool_id.clone(),
                                    OpenOpencodeToolCall {
                                        message_id: msg_id.clone(),
                                        name: tool_name.clone(),
                                        args_buf: tool_args.clone(),
                                    },
                                );
                                events.push(UiEventMessage::ToolCallPlaced {
                                    message_id: msg_id.clone(),
                                    tool_call_id: tool_id,
                                    name: tool_name,
                                    args_json: tool_args,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            Ok(events)
        }
        "message.part.updated" => {
            let part = raw.get("part").cloned().unwrap_or(Value::Null);
            let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");

            match part_type {
                "text" => {
                    let text = part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let Some(mid) = state.open_assistant_message_id.clone() else {
                        return Ok(vec![]);
                    };
                    Ok(vec![UiEventMessage::TextDelta {
                        message_id: mid,
                        text,
                    }])
                }
                "reasoning" => {
                    let text = part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let Some(mid) = state.open_assistant_message_id.clone() else {
                        return Ok(vec![]);
                    };
                    Ok(vec![UiEventMessage::ReasoningDelta {
                        message_id: mid,
                        text,
                    }])
                }
                "tool-call" => {
                    let tool_id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let tool_state = part
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    match tool_state {
                        "calling" => {
                            let tool_name = part
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let tool_args = part
                                .get("args")
                                .and_then(|v| serde_json::to_string(v).ok())
                                .unwrap_or_default();
                            let mid = state
                                .open_assistant_message_id
                                .clone()
                                .unwrap_or_default();
                            if !tool_id.is_empty() {
                                state.tool_calls.insert(
                                    tool_id.clone(),
                                    OpenOpencodeToolCall {
                                        message_id: mid.clone(),
                                        name: tool_name.clone(),
                                        args_buf: tool_args.clone(),
                                    },
                                );
                                Ok(vec![UiEventMessage::ToolCallPlaced {
                                    message_id: mid,
                                    tool_call_id: tool_id,
                                    name: tool_name,
                                    args_json: tool_args,
                                }])
                            } else {
                                Ok(vec![])
                            }
                        }
                        "complete" => {
                            let output = part
                                .get("output")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let is_error = part
                                .get("is_error")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            if state.tool_calls.remove(&tool_id).is_some() {
                                Ok(vec![UiEventMessage::ToolCallCompleted {
                                    tool_call_id: tool_id,
                                    output,
                                    is_error,
                                }])
                            } else {
                                Ok(vec![UiEventMessage::ToolCallCompleted {
                                    tool_call_id: tool_id,
                                    output,
                                    is_error,
                                }])
                            }
                        }
                        _ => Ok(vec![]),
                    }
                }
                _ => Ok(vec![]),
            }
        }
        "session.idle" => {
            let mut events = Vec::new();
            if let Some(mid) = state.open_assistant_message_id.take() {
                events.push(UiEventMessage::MessageCompleted {
                    message_id: mid,
                    finished_at_ms: 0,
                });
            }
            Ok(events)
        }
        "session.error" => {
            let code = raw
                .get("error")
                .and_then(|e| e.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("opencode_error")
                .to_string();
            let message = raw
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("opencode reported an error")
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
        "permission.updated" => Ok(vec![UiEventMessage::Raw {
            kind: "opencode/permission.updated".into(),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("opencode/{other}"),
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
    fn session_created_emits_thread_opened() {
        let mut state = OpencodeTranslatorState::new("thr_x".into());
        let raw = val(r#"{
            "type":"session.created",
            "session":{"id":"sess_1","title":"My Session"}
        }"#);
        let out = translate(&mut state, &raw).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0] {
            UiEventMessage::ThreadOpened {
                thread_id,
                agent,
                title,
                opened_at_ms,
            } => {
                assert_eq!(thread_id, "thr_x");
                assert_eq!(*agent, AgentName::Opencode);
                assert_eq!(title.as_deref(), Some("My Session"));
                assert_eq!(*opened_at_ms, 0);
            }
            _ => panic!("unexpected {:?}", out[0]),
        }
        assert_eq!(state.opencode_session_id.as_deref(), Some("sess_1"));
    }

    #[test]
    fn message_updated_with_text_part() {
        let mut s = OpencodeTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(r#"{
                "type":"message.updated",
                "message":{
                    "id":"msg_1",
                    "role":"assistant",
                    "parts":[{"type":"text","text":"Hello"}]
                }
            }"#),
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert!(matches!(
            &out[0],
            UiEventMessage::MessageStarted {
                role: MessageRole::Assistant,
                message_id,
                ..
            } if message_id == "msg_1"
        ));
        assert!(matches!(
            &out[1],
            UiEventMessage::TextDelta {
                message_id,
                text,
            } if message_id == "msg_1" && text == "Hello"
        ));
    }

    #[test]
    fn message_part_updated_text_delta() {
        let mut s = OpencodeTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                "type":"message.updated",
                "message":{"id":"msg_1","role":"assistant","parts":[]}
            }"#),
        )
        .unwrap();

        let out = translate(
            &mut s,
            &val(r#"{
                "type":"message.part.updated",
                "part":{"type":"text","text":"world"}
            }"#),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::TextDelta {
                message_id,
                text,
            } if message_id == "msg_1" && text == "world"
        ));
    }

    #[test]
    fn message_part_updated_reasoning_delta() {
        let mut s = OpencodeTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                "type":"message.updated",
                "message":{"id":"msg_1","role":"assistant","parts":[]}
            }"#),
        )
        .unwrap();

        let out = translate(
            &mut s,
            &val(r#"{
                "type":"message.part.updated",
                "part":{"type":"reasoning","text":"thinking..."}
            }"#),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::ReasoningDelta {
                message_id,
                text,
            } if message_id == "msg_1" && text == "thinking..."
        ));
    }

    #[test]
    fn tool_call_placed_and_completed() {
        let mut s = OpencodeTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                "type":"message.updated",
                "message":{"id":"msg_1","role":"assistant","parts":[]}
            }"#),
        )
        .unwrap();

        let placed = translate(
            &mut s,
            &val(r#"{
                "type":"message.part.updated",
                "part":{"type":"tool-call","id":"tc_1","name":"bash","args":{"cmd":"ls"},"state":"calling"}
            }"#),
        )
        .unwrap();
        assert_eq!(placed.len(), 1);
        assert!(matches!(
            &placed[0],
            UiEventMessage::ToolCallPlaced {
                tool_call_id,
                name,
                ..
            } if tool_call_id == "tc_1" && name == "bash"
        ));

        let completed = translate(
            &mut s,
            &val(r#"{
                "type":"message.part.updated",
                "part":{"type":"tool-call","id":"tc_1","output":"file1\nfile2","is_error":false,"state":"complete"}
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
            } if tool_call_id == "tc_1"
        ));
    }

    #[test]
    fn permission_updated_emits_raw_for_now() {
        let mut s = OpencodeTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(r#"{
                "type":"permission.updated",
                "permission":{"tool":"bash","decision":"allowed"}
            }"#),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::Raw { kind, .. } if kind == "opencode/permission.updated"
        ));
    }

    #[test]
    fn session_idle_emits_message_completed() {
        let mut s = OpencodeTranslatorState::new("thr".into());
        let _ = translate(
            &mut s,
            &val(r#"{
                "type":"message.updated",
                "message":{"id":"msg_1","role":"assistant","parts":[]}
            }"#),
        )
        .unwrap();

        let out = translate(&mut s, &val(r#"{"type":"session.idle"}"#)).unwrap();
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
    fn unknown_event_falls_through_to_raw() {
        let mut s = OpencodeTranslatorState::new("thr".into());
        let raw = val(r#"{"type":"custom_event","data":"something"}"#);
        let out = translate(&mut s, &raw).unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::Raw { kind, .. } if kind == "opencode/custom_event"
        ));
    }
}
