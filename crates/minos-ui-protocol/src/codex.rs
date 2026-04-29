//! Codex CLI → `UiEventMessage` translator.
//!
//! Each codex WebSocket notification flows through `translate`, accumulating
//! per-thread state (currently-open assistant message id, buffered tool-call
//! arguments) in `CodexTranslatorState`. State is NOT thread-safe by design:
//! the backend owns one instance per live thread and reconstructs a fresh
//! state for history reads so translations are deterministic.

use crate::error::TranslationError;
use crate::message::{MessageRole, ThreadEndReason, UiEventMessage};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Per-thread state the translator accumulates while streaming raw codex
/// notifications. Not thread-safe; one instance per `thread_id`.
pub struct CodexTranslatorState {
    thread_id: String,
    /// Currently-open assistant message (only one at a time for codex).
    open_assistant_message_id: Option<String>,
    /// Currently-open user message. The exec/jsonl route synthesizes
    /// `item/userMessage/delta` so confirmed user messages can replace the
    /// optimistic bubble in the mobile UI.
    open_user_message_id: Option<String>,
    /// CLI tool-call-id → buffered state (args JSON fragments, stable UUID
    /// the translator assigned when `toolCall/started` was seen, plus the
    /// message id the tool call belongs to).
    tool_calls: HashMap<String, OpenToolCall>,
}

struct OpenToolCall {
    message_id: String,
    tool_call_id_stable: String,
    name: String,
    args_buf: String,
}

impl CodexTranslatorState {
    #[must_use]
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            open_assistant_message_id: None,
            open_user_message_id: None,
            tool_calls: HashMap::new(),
        }
    }
}

/// Translate one raw codex WS notification (or response) into zero or more
/// UI events. State is threaded through `state`.
#[allow(clippy::too_many_lines)] // Single-source dispatch over ~10 codex methods; splitting would obscure the protocol mapping.
pub fn translate(
    state: &mut CodexTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let method =
        raw.get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslationError::Malformed {
                reason: "missing method".into(),
            })?;
    let params = raw.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "thread/started" => {
            let thread_id = params
                .get("threadId")
                .and_then(Value::as_str)
                .unwrap_or(&state.thread_id)
                .to_string();
            let opened_at_ms = params
                .get("createdAtMs")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            Ok(vec![UiEventMessage::ThreadOpened {
                thread_id,
                agent: AgentName::Codex,
                title: None,
                opened_at_ms,
            }])
        }
        "thread/archived" => {
            let closed_at_ms = params
                .get("archivedAtMs")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            Ok(vec![UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::AgentDone,
                closed_at_ms,
            }])
        }
        "item/started" => {
            let role_raw = params
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("agent");
            let started_at_ms = params
                .get("startedAtMs")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let role = match role_raw {
                "user" => MessageRole::User,
                "agent" | "assistant" => MessageRole::Assistant,
                _ => MessageRole::System,
            };
            let message_id = Uuid::new_v4().to_string();
            if matches!(role, MessageRole::Assistant) {
                state.open_assistant_message_id = Some(message_id.clone());
            } else if matches!(role, MessageRole::User) {
                state.open_user_message_id = Some(message_id.clone());
            }
            Ok(vec![UiEventMessage::MessageStarted {
                message_id,
                role,
                started_at_ms,
            }])
        }
        "item/userMessage/delta" => {
            let text = params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let Some(msg_id) = state.open_user_message_id.clone() else {
                return Ok(vec![]);
            };
            Ok(vec![UiEventMessage::TextDelta {
                message_id: msg_id,
                text,
            }])
        }
        "item/agentMessage/delta" => {
            let text = params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let Some(msg_id) = state.open_assistant_message_id.clone() else {
                // Delta without an open assistant message — drop silently.
                return Ok(vec![]);
            };
            Ok(vec![UiEventMessage::TextDelta {
                message_id: msg_id,
                text,
            }])
        }
        // Note: spec §12.1 (2026-04 codex app-server) canonicalises reasoning
        // deltas on `item/reasoning/delta`. Older codex releases exposed
        // `item/reasoning/textDelta` and `item/reasoning/summaryTextDelta`
        // as separate notifications; those names are no longer emitted.
        "item/reasoning/delta" => {
            let text = params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let Some(msg_id) = state.open_assistant_message_id.clone() else {
                return Ok(vec![]);
            };
            Ok(vec![UiEventMessage::ReasoningDelta {
                message_id: msg_id,
                text,
            }])
        }
        // `*/completed` markers are signal-absorbed per spec §12.1: the
        // `MessageCompleted` UI event awaits `turn/completed`, not the
        // per-item completion. Returning `vec![]` keeps these off the mobile
        // timeline without falling through to the Raw escape hatch.
        "item/agentMessage/completed" | "item/reasoning/completed" => Ok(vec![]),
        "item/toolCall/started" => {
            let cli_id = params
                .get("toolCallId")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed {
                    reason: "toolCallId missing".into(),
                })?
                .to_string();
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let Some(msg_id) = state.open_assistant_message_id.clone() else {
                return Ok(vec![]);
            };
            let stable_id = Uuid::new_v4().to_string();
            state.tool_calls.insert(
                cli_id,
                OpenToolCall {
                    message_id: msg_id,
                    tool_call_id_stable: stable_id,
                    name,
                    args_buf: String::new(),
                },
            );
            Ok(vec![])
        }
        "item/toolCall/arguments" => {
            let cli_id = params
                .get("toolCallId")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed {
                    reason: "toolCallId missing".into(),
                })?;
            if let Some(tc) = state.tool_calls.get_mut(cli_id) {
                if let Some(delta) = params.get("argumentsDelta").and_then(Value::as_str) {
                    tc.args_buf.push_str(delta);
                }
            }
            Ok(vec![])
        }
        "item/toolCall/argumentsCompleted" => {
            let cli_id = params
                .get("toolCallId")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed {
                    reason: "toolCallId missing".into(),
                })?;
            if let Some(tc) = state.tool_calls.get(cli_id) {
                Ok(vec![UiEventMessage::ToolCallPlaced {
                    message_id: tc.message_id.clone(),
                    tool_call_id: tc.tool_call_id_stable.clone(),
                    name: tc.name.clone(),
                    args_json: tc.args_buf.clone(),
                }])
            } else {
                Ok(vec![])
            }
        }
        "item/toolCall/completed" => {
            let cli_id = params
                .get("toolCallId")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslationError::Malformed {
                    reason: "toolCallId missing".into(),
                })?;
            let output = params
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let is_error = params
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(tc) = state.tool_calls.remove(cli_id) {
                Ok(vec![UiEventMessage::ToolCallCompleted {
                    tool_call_id: tc.tool_call_id_stable,
                    output,
                    is_error,
                }])
            } else {
                Ok(vec![])
            }
        }
        "turn/completed" => {
            let finished_at_ms = params
                .get("finishedAtMs")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let Some(msg_id) = state.open_assistant_message_id.take() else {
                return Ok(vec![]);
            };
            Ok(vec![UiEventMessage::MessageCompleted {
                message_id: msg_id,
                finished_at_ms,
            }])
        }
        "error" => {
            let code = params
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("codex_error")
                .to_string();
            let message = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("codex reported an error")
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
            kind: other.to_string(),
            payload_json: serde_json::to_string(&params).unwrap_or_default(),
        }]),
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::message::*;
    use minos_domain::AgentName;
    use pretty_assertions::assert_eq;

    fn val(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn thread_started_emits_thread_opened() {
        let mut state = CodexTranslatorState::new("thr_x".into());
        let raw = val(r#"{
            "method":"thread/started",
            "params":{"threadId":"thr_x","createdAtMs":1714000000000}
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
                assert_eq!(*agent, AgentName::Codex);
                assert_eq!(*opened_at_ms, 1_714_000_000_000);
            }
            _ => panic!("unexpected {:?}", out[0]),
        }
    }

    #[test]
    fn unknown_method_falls_through_to_raw() {
        let mut state = CodexTranslatorState::new("thr_x".into());
        let raw = val(r#"{"method":"item/plan/delta","params":{"step":"compile"}}"#);
        let out = translate(&mut state, &raw).unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], UiEventMessage::Raw { kind, .. } if kind == "item/plan/delta"));
    }

    #[test]
    fn agent_message_sequence() {
        let mut s = CodexTranslatorState::new("thr".into());

        let o1 = translate(
            &mut s,
            &val(
                r#"{"method":"item/started","params":{"itemId":"i1","role":"agent","startedAtMs":1}}"#,
            ),
        )
        .unwrap();
        assert!(matches!(
            o1.as_slice(),
            [UiEventMessage::MessageStarted {
                role: MessageRole::Assistant,
                ..
            }]
        ));

        let o2 = translate(
            &mut s,
            &val(r#"{"method":"item/agentMessage/delta","params":{"itemId":"i1","delta":"Hel"}}"#),
        )
        .unwrap();
        assert!(matches!(o2.as_slice(), [UiEventMessage::TextDelta { text, .. }] if text == "Hel"));

        let o3 = translate(
            &mut s,
            &val(r#"{"method":"item/agentMessage/delta","params":{"itemId":"i1","delta":"lo"}}"#),
        )
        .unwrap();
        assert!(matches!(o3.as_slice(), [UiEventMessage::TextDelta { text, .. }] if text == "lo"));

        let o4 = translate(
            &mut s,
            &val(r#"{"method":"turn/completed","params":{"finishedAtMs":2}}"#),
        )
        .unwrap();
        assert!(matches!(
            o4.as_slice(),
            [UiEventMessage::MessageCompleted {
                finished_at_ms: 2,
                ..
            }]
        ));
    }

    #[test]
    fn user_message_delta_sequence() {
        let mut s = CodexTranslatorState::new("thr".into());

        let o1 = translate(
            &mut s,
            &val(
                r#"{"method":"item/started","params":{"itemId":"u1","role":"user","startedAtMs":1}}"#,
            ),
        )
        .unwrap();
        assert!(matches!(
            o1.as_slice(),
            [UiEventMessage::MessageStarted {
                role: MessageRole::User,
                ..
            }]
        ));

        let o2 = translate(
            &mut s,
            &val(r#"{"method":"item/userMessage/delta","params":{"itemId":"u1","delta":"hello"}}"#),
        )
        .unwrap();
        assert!(
            matches!(o2.as_slice(), [UiEventMessage::TextDelta { text, .. }] if text == "hello")
        );
    }

    #[test]
    fn raw_error_maps_to_ui_error() {
        let mut s = CodexTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(r#"{"method":"error","params":{"code":"exec_exit_nonzero","message":"boom"}}"#),
        )
        .unwrap();
        assert_eq!(
            out,
            vec![UiEventMessage::Error {
                code: "exec_exit_nonzero".into(),
                message: "boom".into(),
                message_id: None,
            }]
        );
    }

    #[test]
    fn tool_call_buffers_args_then_emits_placed() {
        let mut s = CodexTranslatorState::new("thr".into());

        // Bracket with a MessageStarted so the tool is associated.
        let _ = translate(
            &mut s,
            &val(
                r#"{"method":"item/started","params":{"itemId":"i1","role":"agent","startedAtMs":1}}"#,
            ),
        )
        .unwrap();

        let o1 = translate(
            &mut s,
            &val(
                r#"{"method":"item/toolCall/started","params":{"itemId":"i1","toolCallId":"tc_1","name":"run_command"}}"#,
            ),
        )
        .unwrap();
        assert!(o1.is_empty(), "emitted too early: {o1:?}");

        let o2 = translate(
            &mut s,
            &val(
                r#"{"method":"item/toolCall/arguments","params":{"toolCallId":"tc_1","argumentsDelta":"{\"cmd\":\"ls"}}"#,
            ),
        )
        .unwrap();
        assert!(o2.is_empty());

        let o3 = translate(
            &mut s,
            &val(
                r#"{"method":"item/toolCall/arguments","params":{"toolCallId":"tc_1","argumentsDelta":"\"}"}}"#,
            ),
        )
        .unwrap();
        assert!(o3.is_empty());

        let o4 = translate(
            &mut s,
            &val(r#"{"method":"item/toolCall/argumentsCompleted","params":{"toolCallId":"tc_1"}}"#),
        )
        .unwrap();
        assert_eq!(o4.len(), 1);
        match &o4[0] {
            UiEventMessage::ToolCallPlaced {
                tool_call_id,
                name,
                args_json,
                ..
            } => {
                assert_eq!(name, "run_command");
                assert_eq!(args_json, r#"{"cmd":"ls"}"#);
                // tool_call_id is translator-assigned (UUID); just assert non-empty.
                assert!(!tool_call_id.is_empty());
            }
            _ => panic!(),
        }

        let o5 = translate(
            &mut s,
            &val(
                r#"{"method":"item/toolCall/completed","params":{"toolCallId":"tc_1","output":"file1\nfile2","isError":false}}"#,
            ),
        )
        .unwrap();
        assert!(matches!(
            o5.as_slice(),
            [UiEventMessage::ToolCallCompleted {
                output,
                is_error: false,
                ..
            }] if output == "file1\nfile2"
        ));
    }
}
