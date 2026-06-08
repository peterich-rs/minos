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
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Per-thread state the translator accumulates while streaming raw codex
/// notifications. Not thread-safe; one instance per `thread_id`.
pub struct CodexTranslatorState {
    thread_id: String,
    /// Currently-open assistant message (only one at a time for codex).
    /// Populated when `item/started` carries `item.type == "agentMessage"`;
    /// reset when the corresponding `turn/completed` lands.
    open_assistant_message_id: Option<String>,
    /// Currently-open user message id. Set when an `item/started` with
    /// `item.type == "userMessage"` arrives — either echoed by codex
    /// itself, or synthesised by the daemon ahead of `turn/start` (see
    /// `minos-agent-runtime::manager::send_user_message`). Used so the
    /// `error` translator can target the user bubble when the failure
    /// races a still-open user item.
    open_user_message_id: Option<String>,
    /// Already-emitted message ids — used to dedupe a `MessageStarted` for
    /// the same `item.id` if the daemon's synthesised echo races with a
    /// real codex `item/started` carrying the same id. Lookup is bounded
    /// to the current thread's lifetime; entries never get pruned because
    /// codex item ids are turn-scoped and one thread's `turn` count is
    /// bounded in practice.
    emitted_message_ids: std::collections::HashSet<String>,
    /// Normalized user texts already seen in the current in-flight turn.
    /// The daemon synthesizes a durable user item before `turn/start`; some
    /// app-server versions can also echo that same user item with a distinct
    /// id. Message ids alone cannot collapse that case, so we suppress a
    /// second identical user text until the turn completes.
    pending_user_message_texts: HashSet<String>,
    /// CLI tool-call-id → buffered state (args JSON fragments, stable UUID
    /// the translator assigned when `toolCall/started` was seen, plus the
    /// message id the tool call belongs to).
    tool_calls: HashMap<String, OpenToolCall>,
    /// Completed thread items can arrive after later assistant content has
    /// started. Remember the assistant message that was open when each item
    /// started so completed tool rows do not attach to a newer answer.
    item_message_ids: HashMap<String, String>,
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
            emitted_message_ids: std::collections::HashSet::new(),
            pending_user_message_texts: HashSet::new(),
            tool_calls: HashMap::new(),
            item_message_ids: HashMap::new(),
        }
    }
}

/// Concatenate the `text` field of every `Text` variant in a codex
/// `userMessage.content` array. Non-text inputs (image, mention, skill,
/// localImage) are not yet rendered as text — they fall through silently
/// and the resulting string may be empty. Newline-joined so multi-segment
/// pastes stay readable in the bubble.
fn collect_user_input_text(content: Option<&Value>) -> String {
    let Some(Value::Array(arr)) = content else {
        return String::new();
    };
    arr.iter()
        .filter_map(|el| {
            let kind = el.get("type").and_then(Value::as_str)?;
            if kind == "text" {
                el.get("text").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_user_input_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
            // Real codex 2026-04 shape (`crates/minos-codex-protocol`
            // `ItemStartedNotification`): `params: {item: ThreadItem (tagged
            // by "type"), threadId, turnId}`. ThreadItem variants we render
            // in the chat timeline are `userMessage` and `agentMessage`;
            // anything else (plan, reasoning, hookPrompt, …) flows through
            // as a `Raw` event so the system surface can still see it
            // without producing a misleading bubble.
            //
            // Item id is supplied by codex (or the daemon when it
            // synthesises a user echo ahead of `turn/start`); we do NOT
            // mint a fresh UUID here so that re-translation is idempotent
            // and the daemon's synth + a hypothetical codex echo of the
            // same id collapse into a single bubble.
            let item = params.get("item").cloned().unwrap_or(Value::Null);
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            let item_id = item
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| Uuid::new_v4().to_string(), str::to_string);

            match item_type {
                "userMessage" => {
                    if !state.emitted_message_ids.insert(item_id.clone()) {
                        // Duplicate — already emitted MessageStarted for
                        // this id (synth + codex echo race). Drop silently.
                        return Ok(vec![]);
                    }
                    let text = collect_user_input_text(item.get("content"));
                    let normalized_text = normalize_user_input_text(&text);
                    if !normalized_text.is_empty()
                        && !state.pending_user_message_texts.insert(normalized_text)
                    {
                        // Same turn, same user text, different item id:
                        // daemon synth + app-server echo. Keep the durable
                        // first row and suppress the transport echo.
                        return Ok(vec![]);
                    }
                    state.open_user_message_id = Some(item_id.clone());
                    let mut events = vec![UiEventMessage::MessageStarted {
                        message_id: item_id.clone(),
                        role: MessageRole::User,
                        started_at_ms: 0,
                    }];
                    if !text.is_empty() {
                        events.push(UiEventMessage::TextDelta {
                            message_id: item_id,
                            text,
                        });
                    }
                    Ok(events)
                }
                "agentMessage" => {
                    if !state.emitted_message_ids.insert(item_id.clone()) {
                        return Ok(vec![]);
                    }
                    state.open_assistant_message_id = Some(item_id.clone());
                    Ok(vec![UiEventMessage::MessageStarted {
                        message_id: item_id,
                        role: MessageRole::Assistant,
                        started_at_ms: 0,
                    }])
                }
                "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall" => {
                    if let Some(msg_id) = state.open_assistant_message_id.clone() {
                        state.item_message_ids.insert(item_id.clone(), msg_id);
                    }
                    Ok(vec![])
                }
                _ => Ok(vec![UiEventMessage::Raw {
                    kind: format!("item/started:{item_type}"),
                    payload_json: serde_json::to_string(&params).unwrap_or_default(),
                }]),
            }
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
        "item/completed" => Ok(translate_item_completed(state, &params)),
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
            state.pending_user_message_texts.clear();
            state.item_message_ids.clear();
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

fn translate_item_completed(
    state: &mut CodexTranslatorState,
    params: &Value,
) -> Vec<UiEventMessage> {
    let item = params.get("item").unwrap_or(&Value::Null);
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
    let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
    match item_type {
        "agentMessage" => {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if item_id.is_empty() {
                return Vec::new();
            }
            let mut events = Vec::new();
            if state.open_assistant_message_id.as_deref() != Some(item_id)
                && state.emitted_message_ids.insert(item_id.to_string())
            {
                state.open_assistant_message_id = Some(item_id.to_string());
                events.push(UiEventMessage::MessageStarted {
                    message_id: item_id.to_string(),
                    role: MessageRole::Assistant,
                    started_at_ms: 0,
                });
            }
            events.push(UiEventMessage::TextReplace {
                message_id: item_id.to_string(),
                text,
            });
            events
        }
        "reasoning" => {
            let Some(msg_id) = state.open_assistant_message_id.clone() else {
                return Vec::new();
            };
            let content = string_array(item.get("content"));
            let summary = string_array(item.get("summary"));
            let text = if !summary.is_empty() {
                summary.join("\n")
            } else {
                content.join("\n")
            };
            if text.is_empty() {
                Vec::new()
            } else {
                vec![UiEventMessage::ReasoningReplace {
                    message_id: msg_id,
                    text,
                }]
            }
        }
        "commandExecution" => {
            let Some(msg_id) = completed_item_message_id(state, item_id) else {
                return Vec::new();
            };
            let tool_call_id = stable_item_tool_call_id(item_id, item_type);
            let name = "commandExecution".to_string();
            let args_json = serde_json::json!({
                "command": item.get("command").cloned().unwrap_or(Value::Null),
                "cwd": item.get("cwd").cloned().unwrap_or(Value::Null),
                "status": item.get("status").cloned().unwrap_or(Value::Null),
                "exitCode": item.get("exitCode").cloned().unwrap_or(Value::Null),
            })
            .to_string();
            let output = item
                .get("aggregatedOutput")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let is_error = !matches!(
                item.get("status").and_then(Value::as_str),
                Some("completed" | "succeeded")
            ) || item.get("exitCode").and_then(Value::as_i64).unwrap_or(0) != 0;
            vec![
                UiEventMessage::ToolCallPlaced {
                    message_id: msg_id,
                    tool_call_id: tool_call_id.clone(),
                    name,
                    args_json,
                },
                UiEventMessage::ToolCallCompleted {
                    tool_call_id,
                    output,
                    is_error,
                },
            ]
        }
        "fileChange" | "mcpToolCall" | "dynamicToolCall" => {
            let Some(msg_id) = completed_item_message_id(state, item_id) else {
                return Vec::new();
            };
            let tool_call_id = stable_item_tool_call_id(item_id, item_type);
            let name = item
                .get("tool")
                .or_else(|| item.get("server"))
                .and_then(Value::as_str)
                .map_or_else(|| item_type.to_string(), str::to_string);
            let args_json = serde_json::to_string(item).unwrap_or_default();
            let output = summarize_completed_tool_item(item_type, item);
            let is_error = completed_tool_item_is_error(item);
            vec![
                UiEventMessage::ToolCallPlaced {
                    message_id: msg_id,
                    tool_call_id: tool_call_id.clone(),
                    name,
                    args_json,
                },
                UiEventMessage::ToolCallCompleted {
                    tool_call_id,
                    output,
                    is_error,
                },
            ]
        }
        _ => Vec::new(),
    }
}

fn completed_item_message_id(state: &mut CodexTranslatorState, item_id: &str) -> Option<String> {
    state
        .item_message_ids
        .remove(item_id)
        .or_else(|| state.open_assistant_message_id.clone())
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn stable_item_tool_call_id(item_id: &str, item_type: &str) -> String {
    if item_id.is_empty() {
        format!("{item_type}:unknown")
    } else {
        format!("{item_type}:{item_id}")
    }
}

fn completed_tool_item_is_error(item: &Value) -> bool {
    matches!(
        item.get("status").and_then(Value::as_str),
        Some("failed" | "errored" | "cancelled" | "denied")
    ) || item.get("error").is_some()
        || item.get("success").and_then(Value::as_bool) == Some(false)
}

fn summarize_completed_tool_item(item_type: &str, item: &Value) -> String {
    match item_type {
        "fileChange" => item
            .get("changes")
            .map(|changes| serde_json::to_string_pretty(changes).unwrap_or_default())
            .unwrap_or_default(),
        "mcpToolCall" => item
            .get("result")
            .or_else(|| item.get("error"))
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_default())
            .unwrap_or_default(),
        "dynamicToolCall" => item
            .get("contentItems")
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_default())
            .unwrap_or_default(),
        _ => String::new(),
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
            &val(r#"{"method":"item/started","params":{
                    "item":{"type":"agentMessage","id":"i1","text":""},
                    "threadId":"thr","turnId":"t1"
                }}"#),
        )
        .unwrap();
        assert!(matches!(
            o1.as_slice(),
            [UiEventMessage::MessageStarted {
                role: MessageRole::Assistant,
                message_id,
                ..
            }] if message_id == "i1"
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
    fn item_completed_replaces_incomplete_agent_message_text() {
        let mut s = CodexTranslatorState::new("thr".into());

        let _ = translate(
            &mut s,
            &val(r#"{"method":"item/started","params":{
                    "item":{"type":"agentMessage","id":"i1","text":""},
                    "threadId":"thr","turnId":"t1"
                }}"#),
        )
        .unwrap();
        let _ = translate(
            &mut s,
            &val(r#"{"method":"item/agentMessage/delta","params":{"itemId":"i1","delta":"Partial"}}"#),
        )
        .unwrap();

        let completed = translate(
            &mut s,
            &val(r#"{"method":"item/completed","params":{
                    "item":{"type":"agentMessage","id":"i1","text":"Partial final answer"},
                    "threadId":"thr","turnId":"t1","completedAtMs":2
                }}"#),
        )
        .unwrap();

        assert!(matches!(
            completed.as_slice(),
            [UiEventMessage::TextReplace { message_id, text }]
                if message_id == "i1" && text == "Partial final answer"
        ));
    }

    #[test]
    fn completed_tool_item_uses_message_open_when_item_started() {
        let mut s = CodexTranslatorState::new("thr".into());

        let _ = translate(
            &mut s,
            &val(r#"{"method":"item/started","params":{
                    "item":{"type":"agentMessage","id":"a1","text":""},
                    "threadId":"thr","turnId":"t1"
                }}"#),
        )
        .unwrap();
        let _ = translate(
            &mut s,
            &val(r#"{"method":"item/started","params":{
                    "item":{"type":"commandExecution","id":"cmd1","command":"ls","commandActions":[],"cwd":"/tmp","status":"inProgress"},
                    "threadId":"thr","turnId":"t1"
                }}"#),
        )
        .unwrap();
        let _ = translate(
            &mut s,
            &val(r#"{"method":"item/started","params":{
                    "item":{"type":"agentMessage","id":"a2","text":""},
                    "threadId":"thr","turnId":"t1"
                }}"#),
        )
        .unwrap();

        let completed = translate(
            &mut s,
            &val(r#"{"method":"item/completed","params":{
                    "item":{"type":"commandExecution","id":"cmd1","command":"ls","commandActions":[],"cwd":"/tmp","status":"completed","aggregatedOutput":"ok","exitCode":0},
                    "threadId":"thr","turnId":"t1","completedAtMs":2
                }}"#),
        )
        .unwrap();

        assert!(matches!(
            completed.as_slice(),
            [UiEventMessage::ToolCallPlaced { message_id, tool_call_id, .. }, UiEventMessage::ToolCallCompleted { tool_call_id: done_id, output, is_error: false }]
                if message_id == "a1" && tool_call_id == done_id && output == "ok"
        ));
    }

    #[test]
    fn user_message_emits_started_and_text_in_one_step() {
        // Real codex (and the daemon's synth) put the user text inside
        // `item.content[*].text`; there is no `item/userMessage/delta` in
        // the 2026-04 protocol.
        let mut s = CodexTranslatorState::new("thr".into());

        let out = translate(
            &mut s,
            &val(r#"{"method":"item/started","params":{
                    "item":{
                        "type":"userMessage",
                        "id":"u1",
                        "content":[{"type":"text","text":"hello"}]
                    },
                    "threadId":"thr","turnId":"t1"
                }}"#),
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert!(matches!(
            &out[0],
            UiEventMessage::MessageStarted {
                role: MessageRole::User,
                message_id,
                ..
            } if message_id == "u1"
        ));
        assert!(matches!(
            &out[1],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "u1" && text == "hello"
        ));
    }

    #[test]
    fn duplicate_user_item_started_is_deduped() {
        // Daemon synth + codex echo can race on the same item.id. The
        // translator must emit MessageStarted exactly once.
        let mut s = CodexTranslatorState::new("thr".into());
        let raw = val(r#"{"method":"item/started","params":{
                "item":{"type":"userMessage","id":"u1","content":[{"type":"text","text":"hi"}]},
                "threadId":"thr","turnId":"t1"
            }}"#);
        let first = translate(&mut s, &raw).unwrap();
        assert_eq!(first.len(), 2);
        let second = translate(&mut s, &raw).unwrap();
        assert!(second.is_empty(), "second emission must dedupe: {second:?}");
    }

    #[test]
    fn duplicate_user_text_in_same_turn_is_deduped_across_item_ids() {
        let mut s = CodexTranslatorState::new("thr".into());
        let synth = val(r#"{"method":"item/started","params":{
                "item":{"type":"userMessage","id":"synth-1","content":[{"type":"text","text":"hi there"}]},
                "threadId":"thr","turnId":""
            }}"#);
        let echo = val(r#"{"method":"item/started","params":{
                "item":{"type":"userMessage","id":"echo-1","content":[{"type":"text","text":"hi  there"}]},
                "threadId":"thr","turnId":"t1"
            }}"#);

        let first = translate(&mut s, &synth).unwrap();
        assert_eq!(first.len(), 2);
        let second = translate(&mut s, &echo).unwrap();
        assert!(
            second.is_empty(),
            "same-turn user echo with a new id must dedupe: {second:?}"
        );
    }

    #[test]
    fn same_user_text_is_allowed_after_turn_completed() {
        let mut s = CodexTranslatorState::new("thr".into());
        let first_user = val(r#"{"method":"item/started","params":{
                "item":{"type":"userMessage","id":"u1","content":[{"type":"text","text":"repeat"}]},
                "threadId":"thr","turnId":"t1"
            }}"#);
        let assistant = val(r#"{"method":"item/started","params":{
                "item":{"type":"agentMessage","id":"a1","text":""},
                "threadId":"thr","turnId":"t1"
            }}"#);
        let completed = val(r#"{"method":"turn/completed","params":{"finishedAtMs":2}}"#);
        let second_user = val(r#"{"method":"item/started","params":{
                "item":{"type":"userMessage","id":"u2","content":[{"type":"text","text":"repeat"}]},
                "threadId":"thr","turnId":"t2"
            }}"#);

        assert_eq!(translate(&mut s, &first_user).unwrap().len(), 2);
        assert_eq!(translate(&mut s, &assistant).unwrap().len(), 1);
        assert_eq!(translate(&mut s, &completed).unwrap().len(), 1);
        let second = translate(&mut s, &second_user).unwrap();
        assert_eq!(second.len(), 2);
        assert!(matches!(
            &second[0],
            UiEventMessage::MessageStarted {
                role: MessageRole::User,
                message_id,
                ..
            } if message_id == "u2"
        ));
    }

    #[test]
    fn user_message_with_empty_content_emits_started_only() {
        let mut s = CodexTranslatorState::new("thr".into());
        let out = translate(
            &mut s,
            &val(r#"{"method":"item/started","params":{
                    "item":{"type":"userMessage","id":"u_empty","content":[]},
                    "threadId":"thr","turnId":"t1"
                }}"#),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            UiEventMessage::MessageStarted {
                role: MessageRole::User,
                ..
            }
        ));
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
            &val(r#"{"method":"item/started","params":{
                    "item":{"type":"agentMessage","id":"i1","text":""},
                    "threadId":"thr","turnId":"t1"
                }}"#),
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
