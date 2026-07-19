use crate::error::TranslationError;
use crate::message::{DisplayPayload, MessageRole, UiEventMessage};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const TOOL_ARGS_DISPLAY_LIMIT: usize = 64 * 1024;

pub struct ClaudeTranslatorState {
    thread_id: String,
    claude_session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    emitted_tool_call_ids: HashSet<String>,
    streamed_message_ids: HashSet<String>,
    blocks: HashMap<usize, StreamBlockState>,
}

enum StreamBlockState {
    Text,
    Thinking,
    ToolUse {
        tool_call_id: String,
        name: String,
        args_json: String,
    },
    Other,
}

impl ClaudeTranslatorState {
    #[must_use]
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            claude_session_id: None,
            open_assistant_message_id: None,
            emitted_message_ids: HashSet::new(),
            emitted_tool_call_ids: HashSet::new(),
            streamed_message_ids: HashSet::new(),
            blocks: HashMap::new(),
        }
    }
}

pub fn translate(
    state: &mut ClaudeTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    if let Some(events) = translate_synthetic_user_message(raw) {
        return Ok(events);
    }

    if let Some(session_id) = raw.get("session_id").and_then(Value::as_str) {
        state.claude_session_id = Some(session_id.to_string());
    }

    let event_type =
        raw.get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslationError::Malformed {
                reason: "missing type".into(),
            })?;

    match event_type {
        "system" => Ok(translate_system(state, raw)),
        "stream_event" => translate_stream_event(state, raw),
        "assistant" => Ok(translate_assistant_message(state, raw)),
        "result" => Ok(translate_result(state, raw)),
        "error" => Ok(vec![translate_error(state, raw)]),
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("claude/{other}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

fn translate_synthetic_user_message(raw: &Value) -> Option<Vec<UiEventMessage>> {
    if raw.get("method").and_then(Value::as_str) != Some("item/started") {
        return None;
    }

    let item = raw.get("params").and_then(|params| params.get("item"))?;
    if item.get("type").and_then(Value::as_str) != Some("userMessage") {
        return None;
    }

    let message_id = item.get("id").and_then(Value::as_str)?.to_string();
    let text = item
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let mut events = vec![UiEventMessage::MessageStarted {
        message_id: message_id.clone(),
        role: MessageRole::User,
        started_at_ms: 0,
    }];
    if !text.is_empty() {
        events.push(UiEventMessage::TextDelta {
            message_id,
            text: DisplayPayload::inline(text),
        });
    }
    Some(events)
}

fn translate_system(state: &mut ClaudeTranslatorState, raw: &Value) -> Vec<UiEventMessage> {
    let subtype = raw.get("subtype").and_then(Value::as_str).unwrap_or("");
    if subtype == "init" {
        return vec![UiEventMessage::ThreadOpened {
            thread_id: state.thread_id.clone(),
            agent: AgentName::Claude,
            title: None,
            opened_at_ms: 0,
        }];
    }

    vec![UiEventMessage::Raw {
        kind: format!("claude/system/{subtype}"),
        payload_json: serde_json::to_string(raw).unwrap_or_default(),
    }]
}

fn translate_stream_event(
    state: &mut ClaudeTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let event = raw
        .get("event")
        .ok_or_else(|| TranslationError::Malformed {
            reason: "stream_event missing event".into(),
        })?;
    let event_type =
        event
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslationError::Malformed {
                reason: "stream_event.event missing type".into(),
            })?;

    match event_type {
        "message_start" => Ok(translate_message_start(state, event)),
        "content_block_start" => Ok(translate_content_block_start(state, event)),
        "content_block_delta" => Ok(translate_content_block_delta(state, event)),
        "content_block_stop" => Ok(translate_content_block_stop(state, event)),
        "message_delta" | "message_stop" => Ok(Vec::new()),
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("claude/stream_event/{other}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

fn translate_message_start(
    state: &mut ClaudeTranslatorState,
    event: &Value,
) -> Vec<UiEventMessage> {
    let message = event.get("message").cloned().unwrap_or(Value::Null);
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }

    let message_id = message
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    start_assistant_message(state, &message_id)
        .into_iter()
        .collect()
}

fn translate_content_block_start(
    state: &mut ClaudeTranslatorState,
    event: &Value,
) -> Vec<UiEventMessage> {
    let index = content_block_index(event);
    let block = event.get("content_block").cloned().unwrap_or(Value::Null);
    let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");

    let state_block = match block_type {
        "text" => StreamBlockState::Text,
        "thinking" | "redacted_thinking" => StreamBlockState::Thinking,
        "tool_use" => StreamBlockState::ToolUse {
            tool_call_id: block
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            name: block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            args_json: String::new(),
        },
        _ => StreamBlockState::Other,
    };

    state.blocks.insert(index, state_block);
    Vec::new()
}

fn translate_content_block_delta(
    state: &mut ClaudeTranslatorState,
    event: &Value,
) -> Vec<UiEventMessage> {
    let index = content_block_index(event);
    let delta = event.get("delta").cloned().unwrap_or(Value::Null);
    let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");

    match (state.blocks.get_mut(&index), delta_type) {
        (Some(StreamBlockState::Text), "text_delta") => {
            let Some(message_id) = state.open_assistant_message_id.clone() else {
                return Vec::new();
            };
            let text = delta
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                return Vec::new();
            }
            state.streamed_message_ids.insert(message_id.clone());
            vec![UiEventMessage::TextDelta {
                message_id,
                text: DisplayPayload::inline(text),
            }]
        }
        (Some(StreamBlockState::Thinking), "thinking_delta") => {
            let Some(message_id) = state.open_assistant_message_id.clone() else {
                return Vec::new();
            };
            let text = delta
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                return Vec::new();
            }
            vec![UiEventMessage::ReasoningDelta {
                message_id,
                text: DisplayPayload::inline(text),
            }]
        }
        (Some(StreamBlockState::ToolUse { args_json, .. }), "input_json_delta") => {
            if let Some(partial_json) = delta.get("partial_json").and_then(Value::as_str) {
                push_bounded_display(args_json, partial_json, TOOL_ARGS_DISPLAY_LIMIT);
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn translate_content_block_stop(
    state: &mut ClaudeTranslatorState,
    event: &Value,
) -> Vec<UiEventMessage> {
    let index = content_block_index(event);

    match state.blocks.remove(&index) {
        Some(StreamBlockState::ToolUse {
            tool_call_id,
            name,
            args_json,
        }) => {
            if tool_call_id.is_empty() || !state.emitted_tool_call_ids.insert(tool_call_id.clone())
            {
                return Vec::new();
            }
            let Some(message_id) = state.open_assistant_message_id.clone() else {
                return Vec::new();
            };
            vec![UiEventMessage::ToolCallPlaced {
                message_id,
                tool_call_id,
                name,
                args_json: DisplayPayload::inline(args_json),
            }]
        }
        _ => Vec::new(),
    }
}

fn content_block_index(event: &Value) -> usize {
    event
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(usize::MAX)
}

fn translate_assistant_message(
    state: &mut ClaudeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let message = raw.get("message").cloned().unwrap_or(Value::Null);
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }

    let message_id = message
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut events = start_assistant_message(state, &message_id)
        .into_iter()
        .collect::<Vec<_>>();

    if let Some(Value::Array(content)) = message.get("content") {
        for block in content {
            match block.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => {
                    if state.streamed_message_ids.contains(&message_id) {
                        continue;
                    }
                    let text = block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::TextDelta {
                            message_id: message_id.clone(),
                            text: DisplayPayload::inline(text),
                        });
                    }
                }
                "thinking" | "redacted_thinking" => {
                    let text = block
                        .get("thinking")
                        .or_else(|| block.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::ReasoningDelta {
                            message_id: message_id.clone(),
                            text: DisplayPayload::inline(text),
                        });
                    }
                }
                "tool_use" => {
                    let tool_call_id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if tool_call_id.is_empty()
                        || !state.emitted_tool_call_ids.insert(tool_call_id.clone())
                    {
                        continue;
                    }
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let args_json =
                        serde_json::to_string(block.get("input").unwrap_or(&Value::Null))
                            .unwrap_or_default();
                    events.push(UiEventMessage::ToolCallPlaced {
                        message_id: message_id.clone(),
                        tool_call_id,
                        name,
                        args_json: DisplayPayload::inline(args_json),
                    });
                }
                _ => {}
            }
        }
    }

    events
}

fn translate_result(state: &mut ClaudeTranslatorState, raw: &Value) -> Vec<UiEventMessage> {
    let mut events = Vec::new();
    let open_message_id = state.open_assistant_message_id.clone();
    if let Some(message_id) = state.open_assistant_message_id.take() {
        events.push(UiEventMessage::MessageCompleted {
            message_id,
            finished_at_ms: 0,
        });
    }

    if raw
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
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
            message_id: open_message_id,
        });
    }

    events
}

fn translate_error(state: &ClaudeTranslatorState, raw: &Value) -> UiEventMessage {
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
    UiEventMessage::Error {
        code,
        message,
        message_id: state.open_assistant_message_id.clone(),
    }
}

fn push_bounded_display(buf: &mut String, delta: &str, limit: usize) {
    if buf.len() >= limit {
        return;
    }
    let remaining = limit - buf.len();
    if delta.len() <= remaining {
        buf.push_str(delta);
        return;
    }
    let mut end = remaining;
    while end > 0 && !delta.is_char_boundary(end) {
        end -= 1;
    }
    buf.push_str(&delta[..end]);
    buf.push_str("\n[truncated display buffer; full raw event is stored separately]");
}

fn start_assistant_message(
    state: &mut ClaudeTranslatorState,
    message_id: &str,
) -> Option<UiEventMessage> {
    if message_id.is_empty() || !state.emitted_message_ids.insert(message_id.to_string()) {
        return None;
    }

    state.open_assistant_message_id = Some(message_id.to_string());
    Some(UiEventMessage::MessageStarted {
        message_id: message_id.to_string(),
        role: MessageRole::Assistant,
        started_at_ms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(s: &str) -> Value {
        serde_json::from_str(s).expect("json fixture should parse")
    }

    #[test]
    fn system_init_emits_thread_opened() {
        let mut state = ClaudeTranslatorState::new("thr_x".into());
        let out = translate(
            &mut state,
            &val(r#"{"type":"system","subtype":"init","session_id":"sess_1","tools":[]}"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &out[0],
            UiEventMessage::ThreadOpened { thread_id, agent, .. }
                if thread_id == "thr_x" && *agent == AgentName::Claude
        ));
        assert_eq!(state.claude_session_id.as_deref(), Some("sess_1"));
    }

    #[test]
    fn synthetic_user_message_is_rendered() {
        let out = translate(
            &mut ClaudeTranslatorState::new("thr_user".into()),
            &val(r#"{
                "method":"item/started",
                "params":{"item":{"type":"userMessage","id":"user_1","content":[{"type":"text","text":"hello claude"}]}}
            }"#),
        )
        .expect("translation should succeed");

        assert_eq!(out.len(), 2);
        assert!(matches!(
            &out[0],
            UiEventMessage::MessageStarted { role: MessageRole::User, message_id, .. }
                if message_id == "user_1"
        ));
        assert!(matches!(
            &out[1],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "user_1" && text == "hello claude"
        ));
    }

    #[test]
    fn stream_event_text_flow_matches_docs() {
        let mut state = ClaudeTranslatorState::new("thr_stream".into());

        let started = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[]}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &started[0],
            UiEventMessage::MessageStarted { role: MessageRole::Assistant, message_id, .. }
                if message_id == "msg_1"
        ));

        let no_output = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        assert!(no_output.is_empty());

        let delta = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello there"}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &delta[0],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "msg_1" && text == "Hello there"
        ));
    }

    #[test]
    fn assistant_message_does_not_duplicate_streamed_text() {
        let mut state = ClaudeTranslatorState::new("thr_stream".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[]}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello there!"}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");

        let out = translate(
            &mut state,
            &val(r#"{
                "type":"assistant",
                "message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"Hello there!"}]},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        assert!(out.is_empty());
    }

    #[test]
    fn result_emits_message_completed() {
        let mut state = ClaudeTranslatorState::new("thr_result".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"message_start","message":{"id":"msg_r1","role":"assistant","content":[]}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");

        let out = translate(
            &mut state,
            &val(r#"{"type":"result","subtype":"success","result":"done","is_error":false}"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &out[0],
            UiEventMessage::MessageCompleted { message_id, .. } if message_id == "msg_r1"
        ));
    }

    #[test]
    fn tool_use_stream_emits_tool_call() {
        let mut state = ClaudeTranslatorState::new("thr_tool".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"message_start","message":{"id":"msg_t1","role":"assistant","content":[]}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool_1","name":"Read"}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"/tmp/a.rs\"}"}},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");

        let out = translate(
            &mut state,
            &val(r#"{
                "type":"stream_event",
                "event":{"type":"content_block_stop","index":1},
                "session_id":"sess_1"
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &out[0],
            UiEventMessage::ToolCallPlaced { message_id, tool_call_id, name, args_json }
                if message_id == "msg_t1"
                    && tool_call_id == "tool_1"
                    && name == "Read"
                    && args_json == "{\"file_path\":\"/tmp/a.rs\"}"
        ));
    }
}
