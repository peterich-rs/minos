use crate::error::TranslationError;
use crate::message::{DisplayPayload, MessageRole, SubagentStatus, UiEventMessage};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct OpencodeTranslatorState {
    session_id: String,
    opencode_session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    open_user_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    tool_calls: HashSet<String>,
    /// Maps parent `task` tool call IDs to registered subagent session/session IDs.
    task_tool_subagents: HashMap<String, String>,
    message_roles: HashMap<String, MessageRole>,
    part_kinds: HashMap<String, TrackedPartKind>,
    parts_with_streamed_delta: HashSet<String>,
    pending_synthetic_user_texts: HashSet<String>,
    suppressed_message_ids: HashSet<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackedPartKind {
    Text,
    Reasoning,
    Tool,
    Other,
}

impl OpencodeTranslatorState {
    #[must_use]
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            opencode_session_id: None,
            open_assistant_message_id: None,
            open_user_message_id: None,
            emitted_message_ids: HashSet::new(),
            tool_calls: HashSet::new(),
            task_tool_subagents: HashMap::new(),
            message_roles: HashMap::new(),
            part_kinds: HashMap::new(),
            parts_with_streamed_delta: HashSet::new(),
            pending_synthetic_user_texts: HashSet::new(),
            suppressed_message_ids: HashSet::new(),
        }
    }
}

pub fn translate(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    if let Some(events) = translate_synthetic_user_message(state, raw) {
        return Ok(events);
    }

    let event_type =
        raw.get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslationError::Malformed {
                reason: "missing type".into(),
            })?;

    match event_type {
        "minos.subagent.spawned" => Ok(translate_synthetic_subagent_spawned(state, raw)),
        "session.created" => Ok(translate_session_created(state, raw)),
        "session.updated" => Ok(translate_session_updated(state, raw)),
        "message.updated" => Ok(translate_message_updated(state, raw)),
        "message.part.updated" => Ok(translate_message_part_updated(state, raw)),
        "message.part.delta" => Ok(translate_message_part_delta(state, raw)),
        "session.status" => Ok(translate_session_status(state, raw)),
        "session.idle" => Ok(complete_open_assistant_message(state)),
        "session.error" => Ok(vec![translate_session_error(state, raw)]),
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

fn translate_synthetic_subagent_spawned(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let properties = props(raw);
    let parent_session_id = properties
        .get("parent_session_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let sub_session_id = properties
        .get("sub_session_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if parent_session_id.is_empty() || sub_session_id.is_empty() {
        return Vec::new();
    }
    let tool_call_id = properties
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !tool_call_id.is_empty() {
        state
            .task_tool_subagents
            .insert(tool_call_id.clone(), sub_session_id.clone());
    }
    vec![UiEventMessage::SubagentSpawned {
        parent_session_id,
        sub_session_id,
        tool_call_id,
        agent: AgentName::Opencode,
        model: properties
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        prompt: properties
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::to_string),
        title: properties
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
    }]
}

fn translate_synthetic_user_message(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Option<Vec<UiEventMessage>> {
    if raw.get("method").and_then(Value::as_str) != Some("item/started") {
        return None;
    }

    let item = raw.get("params").and_then(|params| params.get("item"))?;
    if item.get("type").and_then(Value::as_str) != Some("userMessage") {
        return None;
    }

    let message_id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if message_id.is_empty() {
        return Some(Vec::new());
    }

    let mut events = Vec::new();
    if let Some(started) = start_message(state, &message_id, MessageRole::User) {
        events.push(started);
    }

    let text = collect_text_parts(item.get("content"));
    if !text.is_empty() {
        state
            .pending_synthetic_user_texts
            .insert(normalize_user_text(&text));
        events.push(UiEventMessage::TextDelta {
            message_id,
            text: DisplayPayload::inline(text),
        });
    }

    Some(events)
}

fn translate_session_created(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let info = props(raw)
        .get("info")
        .or_else(|| raw.get("session"))
        .cloned()
        .unwrap_or(Value::Null);

    if let Some(session_id) = info.get("id").and_then(Value::as_str) {
        state.opencode_session_id = Some(session_id.to_string());
    }

    vec![UiEventMessage::SessionOpened {
        session_id: state.session_id.clone(),
        agent: AgentName::Opencode,
        title: info
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
        opened_at_ms: 0,
    }]
}

fn translate_session_updated(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let info = props(raw)
        .get("info")
        .or_else(|| raw.get("session"))
        .cloned()
        .unwrap_or(Value::Null);
    let title = info.get("title").and_then(Value::as_str).unwrap_or("");

    if title.is_empty() {
        Vec::new()
    } else {
        vec![UiEventMessage::SessionTitleUpdated {
            session_id: state.session_id.clone(),
            title: title.to_string(),
        }]
    }
}

fn translate_message_updated(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let message = props(raw)
        .get("info")
        .or_else(|| raw.get("message"))
        .cloned()
        .unwrap_or(Value::Null);
    let message_id = message
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let role = message.get("role").and_then(Value::as_str).unwrap_or("");
    let mut events = Vec::new();

    match role {
        "assistant" => {
            state
                .message_roles
                .insert(message_id.clone(), MessageRole::Assistant);
            if let Some(started) = start_message(state, &message_id, MessageRole::Assistant) {
                events.push(started);
            }

            if let Some(parts) = message.get("parts").and_then(Value::as_array) {
                for part in parts {
                    let part_id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let Some(kind) = tracked_part_kind(part) {
                        state.part_kinds.insert(part_id.clone(), kind);
                    }
                    events.extend(translate_inline_part(
                        state,
                        &message_id,
                        &part_id,
                        MessageRole::Assistant,
                        part,
                        part.get("text").and_then(Value::as_str),
                    ));
                }
            }

            if opencode_message_completion_is_terminal(&message) {
                events.extend(complete_specific_assistant_message(state, &message_id));
            }
        }
        "user" => {
            state
                .message_roles
                .insert(message_id.clone(), MessageRole::User);
            if has_inline_parts(&message) {
                if let Some(started) = start_message(state, &message_id, MessageRole::User) {
                    events.push(started);
                }

                if let Some(parts) = message.get("parts").and_then(Value::as_array) {
                    for part in parts {
                        let part_id = part
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if let Some(kind) = tracked_part_kind(part) {
                            state.part_kinds.insert(part_id.clone(), kind);
                        }
                        events.extend(translate_inline_part(
                            state,
                            &message_id,
                            &part_id,
                            MessageRole::User,
                            part,
                            part.get("text").and_then(Value::as_str),
                        ));
                    }
                }

                let content_text = collect_text_parts(message.get("content"));
                if !content_text.is_empty() {
                    events.push(UiEventMessage::TextDelta {
                        message_id,
                        text: DisplayPayload::inline(content_text),
                    });
                }
            }
        }
        _ => {}
    }

    events
}

fn translate_message_part_updated(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let properties = props(raw);
    let part = properties
        .get("part")
        .or_else(|| raw.get("part"))
        .cloned()
        .unwrap_or(Value::Null);
    let part_id = part
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let message_id = part
        .get("messageID")
        .or_else(|| part.get("message_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let role = state
        .message_roles
        .get(&message_id)
        .copied()
        .unwrap_or(MessageRole::Assistant);
    let mut events = Vec::new();

    if state.suppressed_message_ids.contains(&message_id) {
        return Vec::new();
    }

    if role == MessageRole::User
        && part.get("type").and_then(Value::as_str) == Some("text")
        && part
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| {
                state
                    .pending_synthetic_user_texts
                    .remove(&normalize_user_text(text))
            })
    {
        state.suppressed_message_ids.insert(message_id);
        return Vec::new();
    }

    if let Some(kind) = tracked_part_kind(&part) {
        state.part_kinds.insert(part_id.clone(), kind);
    }

    if let Some(started) = start_message(state, &message_id, role) {
        events.push(started);
    }
    events.extend(translate_inline_part(
        state,
        &message_id,
        &part_id,
        role,
        &part,
        None,
    ));
    events
}

fn translate_message_part_delta(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let properties = props(raw);
    let message_id = properties
        .get("messageID")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let part_id = properties
        .get("partID")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let delta = properties
        .get("delta")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let field = properties
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or("");

    if message_id.is_empty()
        || delta.is_empty()
        || state.suppressed_message_ids.contains(&message_id)
    {
        return Vec::new();
    }

    let role = state
        .message_roles
        .get(&message_id)
        .copied()
        .unwrap_or(MessageRole::Assistant);
    let kind = state
        .part_kinds
        .get(&part_id)
        .copied()
        .unwrap_or(TrackedPartKind::Text);

    state.parts_with_streamed_delta.insert(part_id);

    if let Some(started) = start_message(state, &message_id, role) {
        let mut events = vec![started];
        if let Some(delta_event) = delta_to_ui_event(kind, &message_id, field, delta) {
            events.push(delta_event);
        }
        return events;
    }

    delta_to_ui_event(kind, &message_id, field, delta)
        .into_iter()
        .collect()
}

fn translate_session_status(
    state: &mut OpencodeTranslatorState,
    raw: &Value,
) -> Vec<UiEventMessage> {
    let properties = props(raw);
    let status = properties
        .get("status")
        .and_then(|status| {
            status
                .get("type")
                .and_then(Value::as_str)
                .or_else(|| status.as_str())
        })
        .unwrap_or("");

    if status == "idle" {
        complete_open_assistant_message(state)
    } else {
        Vec::new()
    }
}

fn translate_session_error(state: &OpencodeTranslatorState, raw: &Value) -> UiEventMessage {
    let error = props(raw).get("error").or_else(|| raw.get("error"));
    let code = error
        .and_then(|value| {
            value
                .get("name")
                .or_else(|| value.get("type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("opencode_error")
        .to_string();
    let message = error
        .and_then(|value| {
            value
                .get("data")
                .and_then(|data| data.get("message"))
                .and_then(Value::as_str)
                .or_else(|| value.get("message").and_then(Value::as_str))
        })
        .unwrap_or("opencode reported an error")
        .to_string();

    UiEventMessage::Error {
        code,
        message,
        message_id: state
            .open_assistant_message_id
            .clone()
            .or_else(|| state.open_user_message_id.clone()),
    }
}

fn translate_inline_part(
    state: &mut OpencodeTranslatorState,
    message_id: &str,
    part_id: &str,
    role: MessageRole,
    part: &Value,
    delta: Option<&str>,
) -> Vec<UiEventMessage> {
    let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");

    match part_type {
        "text" => {
            let text = delta
                .or_else(|| part.get("text").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();

            if role == MessageRole::User
                && !text.is_empty()
                && state
                    .pending_synthetic_user_texts
                    .remove(&normalize_user_text(&text))
            {
                state.suppressed_message_ids.insert(message_id.to_string());
                return Vec::new();
            }

            if state.parts_with_streamed_delta.contains(part_id) {
                if should_replace_streamed_part(message_id, part, delta, &text) {
                    return vec![UiEventMessage::TextReplace {
                        message_id: message_id.to_string(),
                        text: DisplayPayload::inline(text),
                    }];
                }
                return Vec::new();
            }

            if text.is_empty() || message_id.is_empty() {
                Vec::new()
            } else {
                vec![UiEventMessage::TextDelta {
                    message_id: message_id.to_string(),
                    text: DisplayPayload::inline(text),
                }]
            }
        }
        "reasoning" => {
            let text = delta
                .or_else(|| part.get("text").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();

            if state.parts_with_streamed_delta.contains(part_id) {
                if should_replace_streamed_part(message_id, part, delta, &text) {
                    return vec![UiEventMessage::ReasoningReplace {
                        message_id: message_id.to_string(),
                        text: DisplayPayload::inline(text),
                    }];
                }
                return Vec::new();
            }

            if text.is_empty() || message_id.is_empty() {
                Vec::new()
            } else {
                vec![UiEventMessage::ReasoningDelta {
                    message_id: message_id.to_string(),
                    text: DisplayPayload::inline(text),
                }]
            }
        }
        "tool" | "tool-call" => translate_tool_part(state, message_id, part),
        _ => Vec::new(),
    }
}

fn should_replace_streamed_part(
    message_id: &str,
    part: &Value,
    delta: Option<&str>,
    text: &str,
) -> bool {
    delta.is_none() && !message_id.is_empty() && !text.is_empty() && part_is_finished(part)
}

fn part_is_finished(part: &Value) -> bool {
    let time = part.get("time").unwrap_or(&Value::Null);
    time.get("end").is_some() || time.get("completed").is_some()
}

fn translate_tool_part(
    state: &mut OpencodeTranslatorState,
    message_id: &str,
    part: &Value,
) -> Vec<UiEventMessage> {
    let tool_call_id = part
        .get("callID")
        .or_else(|| part.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if tool_call_id.is_empty() {
        return Vec::new();
    }

    if let Some(legacy_state) = part.get("state").and_then(Value::as_str) {
        return match legacy_state {
            "calling" => {
                if !state.tool_calls.insert(tool_call_id.clone()) || message_id.is_empty() {
                    Vec::new()
                } else {
                    vec![UiEventMessage::ToolCallPlaced {
                        message_id: message_id.to_string(),
                        tool_call_id,
                        name: part
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        args_json: serde_json::to_string(part.get("args").unwrap_or(&Value::Null))
                            .map(DisplayPayload::inline)
                            .unwrap_or_else(|_| DisplayPayload::inline(String::new())),
                    }]
                }
            }
            "complete" => {
                state.tool_calls.remove(&tool_call_id);
                let name = part.get("name").and_then(Value::as_str).unwrap_or("");
                let output = part
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let is_error = part
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                finish_tool_call(state, tool_call_id, name, output, is_error)
            }
            _ => Vec::new(),
        };
    }

    let tool_state = part.get("state").cloned().unwrap_or(Value::Null);
    let status = tool_state
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = part
        .get("tool")
        .or_else(|| part.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");

    match status {
        "pending" | "running" => {
            if !state.tool_calls.insert(tool_call_id.clone()) || message_id.is_empty() {
                Vec::new()
            } else {
                vec![UiEventMessage::ToolCallPlaced {
                    message_id: message_id.to_string(),
                    tool_call_id,
                    name: name.to_string(),
                    args_json: DisplayPayload::inline(tool_args_json(part, &tool_state)),
                }]
            }
        }
        "completed" => {
            state.tool_calls.remove(&tool_call_id);
            let output = tool_state
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            finish_tool_call(state, tool_call_id, name, output, false)
        }
        "error" => {
            state.tool_calls.remove(&tool_call_id);
            let output = tool_state
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            finish_tool_call(state, tool_call_id, name, output, true)
        }
        _ => Vec::new(),
    }
}

fn finish_tool_call(
    state: &mut OpencodeTranslatorState,
    tool_call_id: String,
    name: &str,
    output: String,
    is_error: bool,
) -> Vec<UiEventMessage> {
    let mut events = vec![UiEventMessage::ToolCallCompleted {
        tool_call_id: tool_call_id.clone(),
        output: output.clone().into(),
        is_error,
    }];
    if let Some(status_event) =
        task_tool_subagent_status_event(state, &tool_call_id, name, &output, is_error)
    {
        events.push(status_event);
    }
    events
}

/// When an opencode `task` tool finishes, also close the linked SubagentCall card.
/// Codex does this via collabAgentToolCall status; opencode only had spawn + tool result.
fn task_tool_subagent_status_event(
    state: &mut OpencodeTranslatorState,
    tool_call_id: &str,
    name: &str,
    output: &str,
    is_error: bool,
) -> Option<UiEventMessage> {
    if name != "task" {
        return None;
    }
    let sub_session_id = state
        .task_tool_subagents
        .remove(tool_call_id)
        .or_else(|| sub_session_id_from_task_output(output))?;
    if sub_session_id.is_empty() {
        return None;
    }
    Some(UiEventMessage::SubagentStatusUpdated {
        sub_session_id,
        status: if is_error {
            SubagentStatus::Failed
        } else {
            SubagentStatus::Completed
        },
    })
}

fn sub_session_id_from_task_output(output: &str) -> Option<String> {
    // opencode task results look like: <task id="ses_..." state="completed">
    let task_tag_start = output.find("<task")?;
    let tag = &output[task_tag_start..];
    let tag_end = tag.find('>').unwrap_or(tag.len());
    let tag = &tag[..tag_end];
    let id_key = "id=\"";
    let id_start = tag.find(id_key)? + id_key.len();
    let id_end = tag[id_start..].find('"')? + id_start;
    let id = tag[id_start..id_end].trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn tool_args_json(part: &Value, tool_state: &Value) -> String {
    tool_state
        .get("raw")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| serde_json::to_string(tool_state.get("input").unwrap_or(&Value::Null)).ok())
        .filter(|value| value != "null")
        .or_else(|| serde_json::to_string(part.get("args").unwrap_or(&Value::Null)).ok())
        .filter(|value| value != "null")
        .or_else(|| serde_json::to_string(tool_state).ok())
        .unwrap_or_default()
}

fn tracked_part_kind(part: &Value) -> Option<TrackedPartKind> {
    Some(match part.get("type").and_then(Value::as_str)? {
        "text" => TrackedPartKind::Text,
        "reasoning" => TrackedPartKind::Reasoning,
        "tool" | "tool-call" => TrackedPartKind::Tool,
        _ => TrackedPartKind::Other,
    })
}

fn delta_to_ui_event(
    part_kind: TrackedPartKind,
    message_id: &str,
    field: &str,
    delta: String,
) -> Option<UiEventMessage> {
    if delta.is_empty() || message_id.is_empty() {
        return None;
    }

    match (part_kind, field) {
        (TrackedPartKind::Reasoning, "text" | "") => Some(UiEventMessage::ReasoningDelta {
            message_id: message_id.to_string(),
            text: DisplayPayload::inline(delta),
        }),
        (TrackedPartKind::Text | TrackedPartKind::Other, "text" | "") => {
            Some(UiEventMessage::TextDelta {
                message_id: message_id.to_string(),
                text: DisplayPayload::inline(delta),
            })
        }
        _ => None,
    }
}

fn has_inline_parts(message: &Value) -> bool {
    message
        .get("parts")
        .and_then(Value::as_array)
        .is_some_and(|parts| !parts.is_empty())
        || message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|parts| !parts.is_empty())
}

fn collect_text_parts(content: Option<&Value>) -> String {
    content
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn normalize_user_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn opencode_message_completion_is_terminal(message: &Value) -> bool {
    if message
        .get("time")
        .and_then(|time| time.get("completed"))
        .is_none()
    {
        return false;
    }

    let Some(finish) = message.get("finish").and_then(Value::as_str) else {
        return true;
    };
    !matches!(finish.to_ascii_lowercase().as_str(), "tool-calls")
}

fn start_message(
    state: &mut OpencodeTranslatorState,
    message_id: &str,
    role: MessageRole,
) -> Option<UiEventMessage> {
    if message_id.is_empty() || !state.emitted_message_ids.insert(message_id.to_string()) {
        return None;
    }

    match role {
        MessageRole::Assistant => state.open_assistant_message_id = Some(message_id.to_string()),
        MessageRole::User => state.open_user_message_id = Some(message_id.to_string()),
        MessageRole::System => {}
    }

    Some(UiEventMessage::MessageStarted {
        message_id: message_id.to_string(),
        role,
        started_at_ms: 0,
    })
}

fn complete_open_assistant_message(state: &mut OpencodeTranslatorState) -> Vec<UiEventMessage> {
    state
        .open_assistant_message_id
        .take()
        .map(|message_id| {
            vec![UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms: 0,
            }]
        })
        .unwrap_or_default()
}

fn complete_specific_assistant_message(
    state: &mut OpencodeTranslatorState,
    message_id: &str,
) -> Vec<UiEventMessage> {
    if state.open_assistant_message_id.as_deref() == Some(message_id) {
        complete_open_assistant_message(state)
    } else {
        Vec::new()
    }
}

fn props(raw: &Value) -> &Value {
    raw.get("properties").unwrap_or(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageRole;

    fn val(s: &str) -> Value {
        serde_json::from_str(s).expect("json fixture should parse")
    }

    #[test]
    fn session_created_uses_current_properties_shape() {
        let mut state = OpencodeTranslatorState::new("thr_1".into());
        let events = translate(
            &mut state,
            &val(r#"{
                "type":"session.created",
                "properties":{"info":{"id":"sess_1","title":"Workspace Session"}}
            }"#),
        )
        .expect("translation should succeed");

        assert!(matches!(
            &events[0],
            UiEventMessage::SessionOpened { session_id, agent, title, .. }
                if session_id == "thr_1"
                    && *agent == AgentName::Opencode
                    && title.as_deref() == Some("Workspace Session")
        ));
        assert_eq!(state.opencode_session_id.as_deref(), Some("sess_1"));
    }

    #[test]
    fn synthetic_user_item_started_is_rendered() {
        let mut state = OpencodeTranslatorState::new("thr_user".into());
        let events = translate(
            &mut state,
            &val(r#"{
                "method":"item/started",
                "params":{
                    "item":{
                        "type":"userMessage",
                        "id":"user_1",
                        "content":[{"type":"text","text":"hello opencode"}]
                    }
                }
            }"#),
        )
        .expect("translation should succeed");

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            UiEventMessage::MessageStarted { role: MessageRole::User, message_id, .. }
                if message_id == "user_1"
        ));
        assert!(matches!(
            &events[1],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "user_1" && text == "hello opencode"
        ));
    }

    #[test]
    fn assistant_message_and_stream_delta_use_current_schema() {
        let mut state = OpencodeTranslatorState::new("thr_stream".into());
        let started = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_a1","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &started[0],
            UiEventMessage::MessageStarted { role: MessageRole::Assistant, message_id, .. }
                if message_id == "msg_a1"
        ));

        let part_started = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{"id":"part_1","sessionID":"sess_1","messageID":"msg_a1","type":"text","text":"","time":{"start":1}}
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(part_started.is_empty());

        let delta = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.delta",
                "properties":{
                    "sessionID":"sess_1",
                    "messageID":"msg_a1",
                    "partID":"part_1",
                    "field":"text",
                    "delta":"Hello"
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &delta[0],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "msg_a1" && text == "Hello"
        ));

        let final_part = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{"id":"part_1","sessionID":"sess_1","messageID":"msg_a1","type":"text","text":"Hello","time":{"start":1,"end":2}}
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &final_part[0],
            UiEventMessage::TextReplace { message_id, text }
                if message_id == "msg_a1" && text == "Hello"
        ));
    }

    #[test]
    fn assistant_message_completed_with_tool_calls_is_not_terminal() {
        let mut state = OpencodeTranslatorState::new("thr_tool_calls".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_tool_calls","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");

        let events = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_tool_calls","sessionID":"sess_1","role":"assistant","finish":"tool-calls","time":{"created":1,"completed":2}}}
            }"#),
        )
        .expect("translation should succeed");

        assert!(!events
            .iter()
            .any(|event| matches!(event, UiEventMessage::MessageCompleted { .. })));
    }

    #[test]
    fn assistant_message_completed_with_stop_is_terminal() {
        let mut state = OpencodeTranslatorState::new("thr_stop".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_stop","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");

        let events = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_stop","sessionID":"sess_1","role":"assistant","finish":"stop","time":{"created":1,"completed":2}}}
            }"#),
        )
        .expect("translation should succeed");

        assert!(events.iter().any(|event| matches!(
            event,
            UiEventMessage::MessageCompleted { message_id, .. } if message_id == "msg_stop"
        )));
    }

    #[test]
    fn final_text_part_replaces_streamed_delta() {
        let mut state = OpencodeTranslatorState::new("thr_replace".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_replace","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{"id":"part_replace","sessionID":"sess_1","messageID":"msg_replace","type":"text","text":"","time":{"start":1}}
                }
            }"#),
        )
        .expect("translation should succeed");

        let delta = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.delta",
                "properties":{
                    "sessionID":"sess_1",
                    "messageID":"msg_replace",
                    "partID":"part_replace",
                    "field":"text",
                    "delta":"）.rs了鼠标CP去路 call。"
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &delta[0],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "msg_replace" && text == "）.rs了鼠标CP去路 call。"
        ));

        let final_part = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{"id":"part_replace","sessionID":"sess_1","messageID":"msg_replace","type":"text","text":"我检查了 Minos TUI 的 opencode 翻译路径。","time":{"start":1,"end":2}}
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &final_part[0],
            UiEventMessage::TextReplace { message_id, text }
                if message_id == "msg_replace" && text == "我检查了 Minos TUI 的 opencode 翻译路径。"
        ));
    }

    #[test]
    fn final_reasoning_part_replaces_streamed_delta() {
        let mut state = OpencodeTranslatorState::new("thr_reasoning_replace".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_reasoning","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{"id":"part_reasoning","sessionID":"sess_1","messageID":"msg_reasoning","type":"reasoning","text":"","time":{"start":1}}
                }
            }"#),
        )
        .expect("translation should succeed");
        let delta = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.delta",
                "properties":{
                    "sessionID":"sess_1",
                    "messageID":"msg_reasoning",
                    "partID":"part_reasoning",
                    "field":"text",
                    "delta":"scratch"
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &delta[0],
            UiEventMessage::ReasoningDelta { message_id, text }
                if message_id == "msg_reasoning" && text == "scratch"
        ));

        let final_part = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{"id":"part_reasoning","sessionID":"sess_1","messageID":"msg_reasoning","type":"reasoning","text":"final reasoning","time":{"start":1,"end":2}}
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &final_part[0],
            UiEventMessage::ReasoningReplace { message_id, text }
                if message_id == "msg_reasoning" && text == "final reasoning"
        ));
    }

    #[test]
    fn tool_part_pending_and_completed_translate() {
        let mut state = OpencodeTranslatorState::new("thr_tool".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_tool","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");

        let placed = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{
                        "id":"part_tool",
                        "sessionID":"sess_1",
                        "messageID":"msg_tool",
                        "type":"tool",
                        "callID":"call_1",
                        "tool":"bash",
                        "state":{"status":"pending","input":{"cmd":"ls"},"raw":"{\"cmd\":\"ls\"}"}
                    }
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &placed[0],
            UiEventMessage::ToolCallPlaced { message_id, tool_call_id, name, args_json }
                if message_id == "msg_tool"
                    && tool_call_id == "call_1"
                    && name == "bash"
                    && args_json == "{\"cmd\":\"ls\"}"
        ));

        let completed = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{
                        "id":"part_tool",
                        "sessionID":"sess_1",
                        "messageID":"msg_tool",
                        "type":"tool",
                        "callID":"call_1",
                        "tool":"bash",
                        "state":{"status":"completed","input":{"cmd":"ls"},"output":"file1\nfile2","title":"bash"}
                    }
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &completed[0],
            UiEventMessage::ToolCallCompleted { tool_call_id, output, is_error: false }
                if tool_call_id == "call_1" && output == "file1\nfile2"
        ));
    }

    #[test]
    fn session_status_idle_completes_open_assistant_message() {
        let mut state = OpencodeTranslatorState::new("thr_idle".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_idle","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");

        let completed = translate(
            &mut state,
            &val(r#"{
                "type":"session.status",
                "properties":{"sessionID":"sess_1","status":{"type":"idle"}}
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &completed[0],
            UiEventMessage::MessageCompleted { message_id, .. } if message_id == "msg_idle"
        ));
    }

    #[test]
    fn session_error_extracts_current_error_shape() {
        let mut state = OpencodeTranslatorState::new("thr_error".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_err","sessionID":"sess_1","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");

        let events = translate(
            &mut state,
            &val(r#"{
                "type":"session.error",
                "properties":{
                    "sessionID":"sess_1",
                    "error":{"name":"APIError","data":{"message":"rate limit"}}
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(matches!(
            &events[0],
            UiEventMessage::Error { code, message, message_id }
                if code == "APIError"
                    && message == "rate limit"
                    && message_id.as_deref() == Some("msg_err")
        ));
    }

    #[test]
    fn legacy_shapes_still_translate() {
        let mut state = OpencodeTranslatorState::new("thr_legacy".into());
        let events = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "message":{"id":"msg_legacy","role":"assistant","parts":[{"type":"text","text":"legacy"}]}
            }"#),
        )
        .expect("translation should succeed");

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            UiEventMessage::MessageStarted { role: MessageRole::Assistant, message_id, .. }
                if message_id == "msg_legacy"
        ));
        assert!(matches!(
            &events[1],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "msg_legacy" && text == "legacy"
        ));
    }

    #[test]
    fn real_user_echo_is_suppressed_after_synthetic_message() {
        let mut state = OpencodeTranslatorState::new("thr_user_echo".into());
        let synthetic = translate(
            &mut state,
            &val(r#"{
                "method":"item/started",
                "params":{
                    "item":{
                        "type":"userMessage",
                        "id":"user_synth",
                        "content":[{"type":"text","text":"say hello in two words"}]
                    }
                }
            }"#),
        )
        .expect("translation should succeed");
        assert_eq!(synthetic.len(), 2);

        let user_info = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_real_user","role":"user","sessionID":"sess_1","time":{"created":1}}}
            }"#),
        )
        .expect("translation should succeed");
        assert!(user_info.is_empty());

        let user_part = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{
                        "id":"prt_user_1",
                        "sessionID":"sess_1",
                        "messageID":"msg_real_user",
                        "type":"text",
                        "text":"say hello in two words"
                    }
                }
            }"#),
        )
        .expect("translation should succeed");
        assert!(user_part.is_empty());
    }

    #[test]
    fn synthetic_subagent_spawned_emits_subagent_event() {
        let mut state = OpencodeTranslatorState::new("parent".into());
        let events = translate(
            &mut state,
            &val(r#"{
                "type":"minos.subagent.spawned",
                "properties":{
                    "parent_session_id":"parent",
                    "sub_session_id":"sub",
                    "tool_call_id":"tool-task",
                    "prompt":"inspect"
                }
            }"#),
        )
        .expect("translation should succeed");

        assert!(matches!(
            events.as_slice(),
            [UiEventMessage::SubagentSpawned {
                parent_session_id,
                sub_session_id,
                tool_call_id,
                agent: AgentName::Opencode,
                prompt,
                ..
            }] if parent_session_id == "parent"
                && sub_session_id == "sub"
                && tool_call_id == "tool-task"
                && prompt.as_deref() == Some("inspect")
        ));
    }

    #[test]
    fn task_tool_completed_emits_subagent_status_updated() {
        use crate::message::SubagentStatus;

        let mut state = OpencodeTranslatorState::new("parent".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_task","sessionID":"sess_parent","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("message start");

        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{
                        "id":"part_task",
                        "sessionID":"sess_parent",
                        "messageID":"msg_task",
                        "type":"tool",
                        "callID":"call_task",
                        "tool":"task",
                        "state":{"status":"running","input":{"prompt":"audit unwrap"}}
                    }
                }
            }"#),
        )
        .expect("task placed");

        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"minos.subagent.spawned",
                "properties":{
                    "parent_session_id":"parent",
                    "sub_session_id":"ses_sub_1",
                    "tool_call_id":"call_task",
                    "prompt":"audit unwrap"
                }
            }"#),
        )
        .expect("subagent spawn");

        let completed = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{
                        "id":"part_task",
                        "sessionID":"sess_parent",
                        "messageID":"msg_task",
                        "type":"tool",
                        "callID":"call_task",
                        "tool":"task",
                        "state":{
                            "status":"completed",
                            "input":{"prompt":"audit unwrap"},
                            "output":"<task id=\"ses_sub_1\" state=\"completed\">\n<task_result>\ndone\n</task_result>\n</task>"
                        }
                    }
                }
            }"#),
        )
        .expect("task completed");

        assert!(matches!(
            completed.as_slice(),
            [
                UiEventMessage::ToolCallCompleted {
                    tool_call_id,
                    is_error: false,
                    ..
                },
                UiEventMessage::SubagentStatusUpdated {
                    sub_session_id,
                    status: SubagentStatus::Completed,
                }
            ] if tool_call_id == "call_task" && sub_session_id == "ses_sub_1"
        ));
    }

    #[test]
    fn task_tool_completed_resolves_subagent_from_output_without_spawn() {
        use crate::message::SubagentStatus;

        let mut state = OpencodeTranslatorState::new("parent".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_task","sessionID":"sess_parent","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("message start");

        let completed = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{
                        "id":"part_task",
                        "sessionID":"sess_parent",
                        "messageID":"msg_task",
                        "type":"tool",
                        "callID":"call_orphan",
                        "tool":"task",
                        "state":{
                            "status":"completed",
                            "output":"<task id=\"ses_from_output\" state=\"completed\">ok</task>"
                        }
                    }
                }
            }"#),
        )
        .expect("task completed");

        assert!(matches!(
            completed.as_slice(),
            [
                UiEventMessage::ToolCallCompleted { .. },
                UiEventMessage::SubagentStatusUpdated {
                    sub_session_id,
                    status: SubagentStatus::Completed,
                }
            ] if sub_session_id == "ses_from_output"
        ));
    }

    #[test]
    fn task_tool_error_emits_subagent_failed() {
        use crate::message::SubagentStatus;

        let mut state = OpencodeTranslatorState::new("parent".into());
        let _ = translate(
            &mut state,
            &val(r#"{
                "type":"message.updated",
                "properties":{"info":{"id":"msg_task","sessionID":"sess_parent","role":"assistant","time":{"created":1}}}
            }"#),
        )
        .expect("message start");
        state
            .task_tool_subagents
            .insert("call_err".into(), "ses_err".into());

        let completed = translate(
            &mut state,
            &val(r#"{
                "type":"message.part.updated",
                "properties":{
                    "part":{
                        "id":"part_task",
                        "sessionID":"sess_parent",
                        "messageID":"msg_task",
                        "type":"tool",
                        "callID":"call_err",
                        "tool":"task",
                        "state":{"status":"error","error":"subagent crashed"}
                    }
                }
            }"#),
        )
        .expect("task error");

        assert!(matches!(
            completed.as_slice(),
            [
                UiEventMessage::ToolCallCompleted {
                    is_error: true,
                    ..
                },
                UiEventMessage::SubagentStatusUpdated {
                    sub_session_id,
                    status: SubagentStatus::Failed,
                }
            ] if sub_session_id == "ses_err"
        ));
    }
}
