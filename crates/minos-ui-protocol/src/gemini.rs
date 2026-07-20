use crate::error::TranslationError;
use crate::message::{DisplayPayload, MessageRole, ThreadEndReason, UiEventMessage};
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

fn ensure_assistant_message(state: &mut GeminiTranslatorState) -> (String, Vec<UiEventMessage>) {
    let message_id = state.open_assistant_message_id.clone().unwrap_or_else(|| {
        let id = format!("msg_{}", Uuid::new_v4());
        state.open_assistant_message_id = Some(id.clone());
        id
    });

    let mut events = Vec::new();
    if state.emitted_message_ids.insert(message_id.clone()) {
        events.push(UiEventMessage::MessageStarted {
            message_id: message_id.clone(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        });
    }

    (message_id, events)
}

fn complete_open_assistant_message(
    state: &mut GeminiTranslatorState,
    finished_at_ms: i64,
) -> Vec<UiEventMessage> {
    state
        .open_assistant_message_id
        .take()
        .map(|message_id| {
            vec![UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms,
            }]
        })
        .unwrap_or_default()
}

#[allow(clippy::too_many_lines)]
pub fn translate(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    // Minos synthetic envelopes shared with Codex (approval overlay path).
    if let Some(method) = raw.get("method").and_then(Value::as_str) {
        match method {
            "approval/request" | "approval/timeout" => {
                let params = raw.get("params").cloned().unwrap_or(Value::Null);
                return Ok(vec![UiEventMessage::Raw {
                    kind: method.to_string(),
                    payload_json: serde_json::to_string(&params).unwrap_or_default(),
                }]);
            }
            _ => {}
        }
    }

    let kind =
        raw.get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslationError::Malformed {
                reason: "missing kind field".into(),
            })?;

    match kind {
        "user_message" => Ok(translate_user_message(state, raw)),
        "acp_notification" => translate_acp_notification(state, raw),
        "acp_server_request" => translate_acp_server_request(state, raw),
        "acp_prompt_response" => Ok(translate_acp_prompt_response(state, raw)),
        "acp_error" => Ok(vec![UiEventMessage::Error {
            code: raw
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("gemini_acp_error")
                .to_string(),
            message: raw
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Gemini ACP reported an error")
                .to_string(),
            message_id: state
                .open_assistant_message_id
                .clone()
                .or_else(|| state.open_user_message_id.clone()),
        }]),
        "acp_closed" => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let mut events = complete_open_assistant_message(state, now_ms);
            state.tool_calls.clear();
            events.push(UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::AgentDone,
                closed_at_ms: now_ms,
            });
            Ok(events)
        }
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/{other}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

fn translate_user_message(state: &mut GeminiTranslatorState, raw: &Value) -> Vec<UiEventMessage> {
    let message_id = raw
        .get("messageId")
        .and_then(Value::as_str)
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_string);
    if !state.emitted_message_ids.insert(message_id.clone()) {
        return vec![];
    }
    let mut events = complete_open_assistant_message(
        state,
        raw.get("createdAtMs")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
    );
    state.open_user_message_id = Some(message_id.clone());
    let text = raw
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    events.push(UiEventMessage::MessageStarted {
        message_id: message_id.clone(),
        role: MessageRole::User,
        started_at_ms: raw.get("createdAtMs").and_then(Value::as_i64).unwrap_or(0),
    });
    if !text.is_empty() {
        events.push(UiEventMessage::TextDelta {
            message_id,
            text: DisplayPayload::inline(text),
        });
    }
    events
}

fn translate_acp_prompt_response(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let stop_reason = raw
        .get("stopReason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn");
    match stop_reason {
        "end_turn" => {
            state.tool_calls.clear();
            complete_open_assistant_message(state, chrono::Utc::now().timestamp_millis())
        }
        "cancelled" => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let mut events = complete_open_assistant_message(state, now_ms);
            state.tool_calls.clear();
            events.push(UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::UserStopped,
                closed_at_ms: now_ms,
            });
            events
        }
        other => vec![UiEventMessage::Error {
            code: format!("gemini_stop_reason_{other}"),
            message: format!("Gemini stopped with reason: {other}"),
            message_id: state.open_assistant_message_id.clone(),
        }],
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
        "agent_message_chunk" | "agent_thought_chunk" => {
            let content = update.get("content").cloned().unwrap_or(Value::Null);
            let content_type = content
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("text");

            let (mid, mut events) = ensure_assistant_message(state);

            match content_type {
                "text" if session_update == "agent_message_chunk" => {
                    let text = content
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::TextDelta {
                            message_id: mid,
                            text: DisplayPayload::inline(text),
                        });
                    }
                }
                "text" | "thought" => {
                    let text = content
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::ReasoningDelta {
                            message_id: mid,
                            text: DisplayPayload::inline(text),
                        });
                    }
                }
                _ => {
                    events.push(UiEventMessage::Raw {
                        kind: format!("gemini/{session_update}/{content_type}"),
                        payload_json: serde_json::to_string(&update).unwrap_or_default(),
                    });
                }
            }

            Ok(events)
        }
        "tool_call" => {
            let tool_call_id = update
                .get("toolCallId")
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

            if tool_call_id.is_empty() {
                Ok(vec![])
            } else {
                let (mid, mut events) = ensure_assistant_message(state);
                state.tool_calls.insert(
                    tool_call_id.clone(),
                    OpenGeminiToolCall {
                        message_id: mid.clone(),
                        name: title.clone(),
                    },
                );
                events.push(UiEventMessage::ToolCallPlaced {
                    message_id: mid,
                    tool_call_id,
                    name: format!("{kind}: {title}"),
                    args_json: DisplayPayload::inline(
                        serde_json::to_string(&update).unwrap_or_default(),
                    ),
                });
                Ok(events)
            }
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("toolCallId")
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
                let content = crate::strip_ansi_escapes(&content);

                state.tool_calls.remove(&tool_call_id);
                Ok(vec![UiEventMessage::ToolCallCompleted {
                    tool_call_id,
                    output: DisplayPayload::inline(content),
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
            if text.is_empty() {
                Ok(vec![])
            } else {
                let (mid, mut events) = ensure_assistant_message(state);
                events.push(UiEventMessage::ReasoningDelta {
                    message_id: mid,
                    text: DisplayPayload::inline(text),
                });
                Ok(events)
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
    let method = raw.get("method").and_then(Value::as_str).unwrap_or("");

    match method {
        "session/request_permission" => {
            let params = raw.get("params").cloned().unwrap_or(Value::Null);
            let tool_call = params.get("toolCall").cloned().unwrap_or(Value::Null);
            let tool_call_id = tool_call
                .get("toolCallId")
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

            if tool_call_id.is_empty() {
                Ok(vec![UiEventMessage::Raw {
                    kind: "gemini/permission_request".into(),
                    payload_json: serde_json::to_string(raw).unwrap_or_default(),
                }])
            } else {
                let (mid, mut events) = ensure_assistant_message(state);
                state.tool_calls.insert(
                    tool_call_id.clone(),
                    OpenGeminiToolCall {
                        message_id: mid.clone(),
                        name: title.clone(),
                    },
                );
                events.push(UiEventMessage::ToolCallPlaced {
                    message_id: mid,
                    tool_call_id,
                    name: format!("{kind}: {title}"),
                    args_json: DisplayPayload::inline(
                        serde_json::to_string(&tool_call).unwrap_or_default(),
                    ),
                });
                Ok(events)
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
        let raw = val(
            r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}}}"#,
        );
        let out = translate(&mut s, &raw).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::MessageStarted {
                role: MessageRole::Assistant,
                ..
            }
        )));
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::TextDelta { text, .. } if text == "Hello")));
    }

    #[test]
    fn internal_user_message_text() {
        let mut s = GeminiTranslatorState::new("thr_x".into());
        let out = translate(
            &mut s,
            &val(r#"{"kind":"user_message","messageId":"u1","text":"你好","threadId":"thr_x"}"#),
        )
        .unwrap();

        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::MessageStarted {
                message_id,
                role: MessageRole::User,
                ..
            } if message_id == "u1"
        )));
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::TextDelta { message_id, text } if message_id == "u1" && text == "你好")));
    }

    #[test]
    fn codex_json_rpc_event_is_not_accepted_for_gemini() {
        let mut s = GeminiTranslatorState::new("thr_x".into());
        let err = translate(
            &mut s,
            &val(
                r#"{"method":"item/agentMessage/delta","params":{"itemId":"a1","delta":"Hello"}}"#,
            ),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            TranslationError::Malformed { reason } if reason == "missing kind field"
        ));
    }

    #[test]
    fn acp_prompt_response_completes_open_assistant_message() {
        let mut s = GeminiTranslatorState::new("thr_x".into());
        let _ = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}}}"#,
            ),
        )
        .unwrap();

        let out = translate(
            &mut s,
            &val(r#"{"kind":"acp_prompt_response","stopReason":"end_turn"}"#),
        )
        .unwrap();

        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::MessageCompleted { .. })));
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
        assert!(out.iter().any(
            |e| matches!(e, UiEventMessage::ReasoningDelta { text, .. } if text == "thinking...")
        ));
    }

    #[test]
    fn acp_notification_agent_thought_chunk_emits_reasoning_delta() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking from gemini"}}}}"#,
            ),
        )
        .unwrap();

        let assistant_id = out
            .iter()
            .find_map(|event| match event {
                UiEventMessage::MessageStarted {
                    message_id,
                    role: MessageRole::Assistant,
                    ..
                } => Some(message_id.clone()),
                _ => None,
            })
            .unwrap();
        assert!(out.iter().any(
            |event| matches!(event, UiEventMessage::ReasoningDelta { message_id, text } if message_id == &assistant_id && text == "thinking from gemini")
        ));
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
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"Edit file","kind":"edit","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        assert!(
            out.iter()
                .any(|e| matches!(e, UiEventMessage::ToolCallPlaced { tool_call_id, name, .. } if tool_call_id == "tc_1" && name.contains("Edit file")))
        );
    }

    #[test]
    fn acp_notification_tool_call_without_open_message_starts_assistant_message() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"Read file","kind":"read","status":"pending"}}}"#,
            ),
        )
        .unwrap();

        let assistant_id = out
            .iter()
            .find_map(|event| match event {
                UiEventMessage::MessageStarted {
                    message_id,
                    role: MessageRole::Assistant,
                    ..
                } => Some(message_id.clone()),
                _ => None,
            })
            .unwrap();
        assert!(out.iter().any(
            |event| matches!(event, UiEventMessage::ToolCallPlaced { message_id, tool_call_id, .. } if message_id == &assistant_id && tool_call_id == "tc_1")
        ));
    }

    #[test]
    fn cancelled_prompt_then_continue_tool_call_uses_new_assistant_message() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let first = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"partial"}}}}"#,
            ),
        )
        .unwrap();
        let first_assistant_id = first
            .iter()
            .find_map(|event| match event {
                UiEventMessage::MessageStarted {
                    message_id,
                    role: MessageRole::Assistant,
                    ..
                } => Some(message_id.clone()),
                _ => None,
            })
            .unwrap();

        let cancelled = translate(
            &mut s,
            &val(r#"{"kind":"acp_prompt_response","stopReason":"cancelled"}"#),
        )
        .unwrap();
        assert!(cancelled.iter().any(
            |event| matches!(event, UiEventMessage::MessageCompleted { message_id, .. } if message_id == &first_assistant_id)
        ));

        let _ = translate(
            &mut s,
            &val(r#"{"kind":"user_message","messageId":"u_continue","text":"continue"}"#),
        )
        .unwrap();
        let after_continue_tool = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_after_continue","title":"Read file","kind":"read","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        let new_assistant_id = after_continue_tool
            .iter()
            .find_map(|event| match event {
                UiEventMessage::MessageStarted {
                    message_id,
                    role: MessageRole::Assistant,
                    ..
                } => Some(message_id.clone()),
                _ => None,
            })
            .unwrap();

        assert_ne!(new_assistant_id, first_assistant_id);
        assert!(after_continue_tool.iter().any(
            |event| matches!(event, UiEventMessage::ToolCallPlaced { message_id, tool_call_id, .. } if message_id == &new_assistant_id && tool_call_id == "tc_after_continue")
        ));
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
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"Edit","kind":"edit","status":"pending"}}}"#,
            ),
        )
        .unwrap();
        let out = translate(
            &mut s,
            &val(
                r#"{"kind":"acp_notification","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"completed","content":null}}}"#,
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
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::MessageCompleted { .. })));
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ThreadClosed {
                reason: ThreadEndReason::AgentDone,
                ..
            }
        )));
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
        assert!(out
            .iter()
            .any(|e| matches!(e, UiEventMessage::Raw { kind, .. } if kind == "gemini/plan")));
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
                r#"{"kind":"acp_server_request","method":"session/request_permission","params":{"toolCall":{"toolCallId":"tc_perm","title":"Run shell","kind":"terminal","status":"pending"},"options":[]}}"#,
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
        assert!(out.iter().any(
            |e| matches!(e, UiEventMessage::Raw { kind, .. } if kind == "gemini/acp/custom_event")
        ));
    }

    #[test]
    fn approval_request_envelope_becomes_raw_for_overlay() {
        let mut s = GeminiTranslatorState::new("thr_x".into());
        let raw = val(
            r#"{"method":"approval/request","params":{"request_id":"perm-1","thread_id":"thr_x","turn_id":"","method":"session/request_permission","params":{"toolCall":{"title":"ls","kind":"other"}}}}"#,
        );
        let out = translate(&mut s, &raw).unwrap();
        assert!(matches!(
            &out[0],
            UiEventMessage::Raw { kind, payload_json }
                if kind == "approval/request" && payload_json.contains("perm-1")
        ));
    }
}
