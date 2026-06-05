use minos_domain::AgentName;
use minos_ui_protocol::{
    ClaudeTranslatorState, CodexTranslatorState, GeminiTranslatorState, MessageRole,
    OpencodeTranslatorState, UiEventMessage,
};
use tracing::{debug, warn};

pub enum AgentTranslationState {
    Codex(CodexTranslatorState),
    Claude(ClaudeTranslatorState),
    Gemini(GeminiTranslatorState),
    Opencode(OpencodeTranslatorState),
}

impl AgentTranslationState {
    pub fn new(agent: AgentName, thread_id: String) -> Self {
        match agent {
            AgentName::Codex => Self::Codex(CodexTranslatorState::new(thread_id)),
            AgentName::Claude => Self::Claude(ClaudeTranslatorState::new(thread_id)),
            AgentName::Gemini => Self::Gemini(GeminiTranslatorState::new(thread_id)),
            AgentName::Opencode => Self::Opencode(OpencodeTranslatorState::new(thread_id)),
        }
    }

    pub fn translate(&mut self, payload: &serde_json::Value) -> Vec<UiEventMessage> {
        match self {
            Self::Codex(s) => translate_with_log("codex", payload, || {
                minos_ui_protocol::translate_codex(s, payload)
            }),
            Self::Claude(s) => translate_with_log("claude", payload, || {
                minos_ui_protocol::translate_claude(s, payload)
            }),
            Self::Gemini(s) => translate_with_log("gemini", payload, || {
                minos_ui_protocol::translate_gemini(s, payload)
            }),
            Self::Opencode(s) => translate_with_log("opencode", payload, || {
                minos_ui_protocol::translate_opencode(s, payload)
            }),
        }
    }
}

fn translate_with_log<F>(agent: &str, payload: &serde_json::Value, f: F) -> Vec<UiEventMessage>
where
    F: FnOnce() -> Result<Vec<UiEventMessage>, minos_ui_protocol::TranslationError>,
{
    match f() {
        Ok(events) => events,
        Err(error) => {
            warn!(
                target: "minos_tui::translation",
                agent,
                error = %error,
                payload = %payload,
                "ui translation failed"
            );
            Vec::new()
        }
    }
}

pub struct ChatState {
    pub thread_id: String,
    pub agent: AgentName,
    pub translation_state: AgentTranslationState,
    pub messages: Vec<RenderedMessage>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
    pub selection: Option<ChatSelection>,
}

impl ChatState {
    pub fn new(thread_id: String, agent: AgentName) -> Self {
        Self {
            translation_state: AgentTranslationState::new(agent, thread_id.clone()),
            thread_id,
            agent,
            messages: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
            max_scroll: 0,
            selection: None,
        }
    }

    pub fn update_max_scroll(&mut self, max_scroll: u16) {
        self.max_scroll = max_scroll;
        if !self.auto_scroll {
            self.scroll_offset = self.scroll_offset.min(self.max_scroll);
        }
    }

    pub fn active_scroll(&self) -> u16 {
        if self.auto_scroll {
            self.max_scroll
        } else {
            self.scroll_offset.min(self.max_scroll)
        }
    }

    pub fn scroll_up(&mut self, lines: u16) {
        if self.auto_scroll {
            self.scroll_offset = self.max_scroll;
            self.auto_scroll = false;
        }
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        if self.auto_scroll {
            return;
        }

        self.scroll_offset = self
            .scroll_offset
            .saturating_add(lines)
            .min(self.max_scroll);
        if self.scroll_offset >= self.max_scroll {
            self.scroll_to_bottom();
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.auto_scroll = false;
        self.scroll_offset = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        self.scroll_offset = 0;
    }

    pub fn begin_selection(&mut self, point: ChatSelectionPoint) {
        self.selection = Some(ChatSelection {
            anchor: point,
            focus: point,
        });
    }

    pub fn update_selection(&mut self, point: ChatSelectionPoint) {
        if let Some(selection) = self.selection.as_mut() {
            selection.focus = point;
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn apply_ui_events(&mut self, events: Vec<UiEventMessage>) {
        for event in events {
            self.apply_ui_event(event);
        }
    }

    pub fn last_completed_assistant_text(&self) -> Option<(String, String)> {
        self.messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant) && !message.is_streaming)
            .and_then(|message| {
                rendered_message_text(message).map(|text| {
                    let key = if message.message_id.is_empty() {
                        format!("text:{text}")
                    } else {
                        message.message_id.clone()
                    };
                    (key, text)
                })
            })
    }

    fn apply_ui_event(&mut self, event: UiEventMessage) {
        match event {
            UiEventMessage::MessageStarted {
                message_id,
                role,
                started_at_ms: _,
            } => {
                self.messages.push(RenderedMessage {
                    message_id,
                    role,
                    text_parts: Vec::new(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                    is_streaming: true,
                    error: None,
                });
            }
            UiEventMessage::TextDelta { message_id, text } => {
                if let Some(msg) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.message_id == message_id)
                {
                    append_text(msg, text);
                }
            }
            UiEventMessage::ReasoningDelta { message_id, text } => {
                if let Some(msg) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.message_id == message_id)
                {
                    msg.reasoning = Some(match msg.reasoning.take() {
                        Some(existing) => existing + &text,
                        None => text,
                    });
                }
            }
            UiEventMessage::ToolCallPlaced {
                message_id,
                tool_call_id,
                name,
                args_json,
            } => {
                if let Some(msg) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.message_id == message_id)
                {
                    let args_summary = summarize_tool_args(&name, &args_json);
                    let args_detail = compact_tool_args(&args_json)
                        .filter(|detail| !detail.is_empty() && detail != &args_summary);
                    msg.tool_calls.push(ToolCallBlock {
                        tool_call_id,
                        name,
                        args_summary,
                        args_detail,
                        output_summary: None,
                        is_error: false,
                        is_expanded: false,
                    });
                }
            }
            UiEventMessage::ToolCallCompleted {
                tool_call_id,
                output,
                is_error,
            } => {
                for msg in self.messages.iter_mut().rev() {
                    if let Some(tc) = msg
                        .tool_calls
                        .iter_mut()
                        .find(|tc| tc.tool_call_id == tool_call_id)
                    {
                        tc.output_summary = Some(truncate_str(&output, 200));
                        tc.is_error = is_error;
                        break;
                    }
                }
            }
            UiEventMessage::MessageCompleted { message_id, .. } => {
                if let Some(msg) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.message_id == message_id)
                {
                    msg.is_streaming = false;
                }
            }
            UiEventMessage::Error {
                message,
                message_id,
                ..
            } => {
                if let Some(msg) = message_id
                    .and_then(|mid| self.messages.iter_mut().rev().find(|m| m.message_id == mid))
                {
                    msg.error = Some(message);
                } else {
                    self.messages.push(RenderedMessage {
                        message_id: String::new(),
                        role: MessageRole::System,
                        text_parts: vec![TextPart::Plain(message)],
                        tool_calls: Vec::new(),
                        reasoning: None,
                        is_streaming: false,
                        error: Some(String::from("error")),
                    });
                }
            }
            UiEventMessage::Raw { kind, payload_json } => {
                debug!(
                    raw_kind = %kind,
                    payload_bytes = payload_json.len(),
                    "raw ui event suppressed from chat"
                );
            }
            UiEventMessage::ThreadOpened { .. } | UiEventMessage::ThreadTitleUpdated { .. } => {}
            UiEventMessage::ThreadClosed { reason, .. } => {
                self.messages.push(RenderedMessage {
                    message_id: String::new(),
                    role: MessageRole::System,
                    text_parts: vec![TextPart::Plain(format!("Thread closed: {reason:?}"))],
                    tool_calls: Vec::new(),
                    reasoning: None,
                    is_streaming: false,
                    error: None,
                });
            }
        }
    }
}

fn rendered_message_text(message: &RenderedMessage) -> Option<String> {
    let mut parts = Vec::new();
    for part in &message.text_parts {
        match part {
            TextPart::Plain(text) => {
                if !text.trim().is_empty() {
                    parts.push(text.trim().to_owned());
                }
            }
            TextPart::Code { code, .. } => {
                if !code.trim().is_empty() {
                    parts.push(code.trim().to_owned());
                }
            }
        }
    }

    if parts.is_empty() {
        message
            .error
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    } else {
        Some(parts.join("\n"))
    }
}

fn append_text(msg: &mut RenderedMessage, text: String) {
    if let Some(TextPart::Plain(last)) = msg.text_parts.last_mut() {
        last.push_str(&text);
    } else {
        msg.text_parts.push(TextPart::Plain(text));
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

fn summarize_tool_args(tool_name: &str, args_json: &str) -> String {
    let Some(value) = parse_tool_args(args_json) else {
        return truncate_str(&one_line(args_json), 180);
    };

    if value.is_null() {
        return String::new();
    }

    let lower_name = tool_name.to_ascii_lowercase();
    let mut pieces = Vec::new();

    if let Some(value) = find_stringish(
        &value,
        &[
            "file_path",
            "filePath",
            "filepath",
            "path",
            "absolute_path",
            "absolutePath",
            "relative_path",
            "relativePath",
            "target_file",
            "targetFile",
            "file",
            "uri",
        ],
    ) {
        pieces.push(summary_piece("file", &value, 90));
    }

    if let Some(value) = find_stringish(&value, &["cmd", "command", "script", "shell"]) {
        pieces.push(summary_piece("cmd", &value, 90));
    }

    if lower_name.contains("task")
        || lower_name == "todo"
        || lower_name == "todowrite"
        || lower_name == "todo_write"
    {
        if let Some(value) = find_stringish(
            &value,
            &[
                "task",
                "description",
                "prompt",
                "instructions",
                "question",
                "subagent_type",
                "subagentType",
            ],
        ) {
            pieces.push(summary_piece("task", &value, 110));
        }
    } else if let Some(value) = find_stringish(&value, &["task", "description"]) {
        pieces.push(summary_piece("task", &value, 110));
    }

    if lower_name.contains("skill") {
        if let Some(value) = find_stringish(
            &value,
            &[
                "skill",
                "skill_name",
                "skillName",
                "name",
                "skill_path",
                "skillPath",
            ],
        ) {
            pieces.push(summary_piece("skill", &value, 90));
        }
    } else if let Some(value) = find_stringish(&value, &["skill", "skill_name", "skillName"]) {
        pieces.push(summary_piece("skill", &value, 90));
    }

    if let Some(count) = array_len_for_keys(&value, &["todos", "todo", "items"]) {
        pieces.push(format!("items={count}"));
    }

    if pieces.is_empty() {
        compact_tool_args(args_json).unwrap_or_default()
    } else {
        truncate_str(&pieces.join(" "), 180)
    }
}

fn compact_tool_args(args_json: &str) -> Option<String> {
    let value = parse_tool_args(args_json)?;
    if value.is_null() {
        return Some(String::new());
    }
    serde_json::to_string(&value)
        .ok()
        .map(|text| truncate_str(&one_line(&text), 500))
}

fn parse_tool_args(args_json: &str) -> Option<serde_json::Value> {
    let trimmed = args_json.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn summary_piece(label: &str, value: &str, max_len: usize) -> String {
    format!("{label}={}", truncate_str(&one_line(value), max_len))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_stringish(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    find_stringish_inner(value, keys, 0)
}

fn find_stringish_inner(value: &serde_json::Value, keys: &[&str], depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }

    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_to_summary_text) {
                    return Some(found);
                }
            }
            for child_key in [
                "input",
                "args",
                "arguments",
                "params",
                "tool_input",
                "toolInput",
                "metadata",
                "state",
            ] {
                if let Some(found) = map
                    .get(child_key)
                    .and_then(|child| find_stringish_inner(child, keys, depth + 1))
                {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|child| find_stringish_inner(child, keys, depth + 1))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_stringish_inner(child, keys, depth + 1)),
        _ => None,
    }
}

fn value_to_summary_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => (!text.trim().is_empty()).then(|| text.to_owned()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
        serde_json::Value::Array(items) => {
            let values = items
                .iter()
                .filter_map(value_to_summary_text)
                .collect::<Vec<_>>();
            (!values.is_empty()).then(|| values.join(","))
        }
        serde_json::Value::Object(map) => {
            for key in [
                "name",
                "path",
                "file_path",
                "filePath",
                "description",
                "task",
                "prompt",
            ] {
                if let Some(text) = map.get(key).and_then(value_to_summary_text) {
                    return Some(text);
                }
            }
            None
        }
        serde_json::Value::Null => None,
    }
}

fn array_len_for_keys(value: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(len) = map
                    .get(*key)
                    .and_then(|value| value.as_array().map(Vec::len))
                {
                    return Some(len);
                }
            }
            map.values()
                .find_map(|child| array_len_for_keys(child, keys))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| array_len_for_keys(child, keys)),
        _ => None,
    }
}

pub struct RenderedMessage {
    pub message_id: String,
    pub role: MessageRole,
    pub text_parts: Vec<TextPart>,
    pub tool_calls: Vec<ToolCallBlock>,
    pub reasoning: Option<String>,
    pub is_streaming: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatSelectionPoint {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSelection {
    pub anchor: ChatSelectionPoint,
    pub focus: ChatSelectionPoint,
}

impl ChatSelection {
    pub fn normalized(&self) -> (ChatSelectionPoint, ChatSelectionPoint) {
        if (self.anchor.row, self.anchor.col) <= (self.focus.row, self.focus.col) {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }
}

#[derive(Debug, PartialEq)]
pub enum TextPart {
    Plain(String),
    Code { lang: String, code: String },
}

pub struct ToolCallBlock {
    pub tool_call_id: String,
    pub name: String,
    pub args_summary: String,
    pub args_detail: Option<String>,
    pub output_summary: Option<String>,
    pub is_error: bool,
    pub is_expanded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;

    #[test]
    fn chat_state_message_started_then_text_delta() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::User,
            started_at_ms: 0,
        }]);
        assert_eq!(cs.messages.len(), 1);
        assert!(cs.messages[0].is_streaming);

        cs.apply_ui_events(vec![UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "hello ".into(),
        }]);
        cs.apply_ui_events(vec![UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "world".into(),
        }]);
        assert_eq!(
            cs.messages[0].text_parts,
            vec![TextPart::Plain("hello world".into())]
        );

        cs.apply_ui_events(vec![UiEventMessage::MessageCompleted {
            message_id: "m1".into(),
            finished_at_ms: 1,
        }]);
        assert!(!cs.messages[0].is_streaming);
    }

    #[test]
    fn tool_call_placed_then_completed() {
        let mut cs = ChatState::new("t1".into(), AgentName::Claude);
        cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        }]);
        cs.apply_ui_events(vec![UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "write_file".into(),
            args_json: r#"{"path":"foo.rs"}"#.into(),
        }]);
        assert_eq!(cs.messages[0].tool_calls.len(), 1);
        assert_eq!(cs.messages[0].tool_calls[0].name, "write_file");
        assert_eq!(cs.messages[0].tool_calls[0].args_summary, "file=foo.rs");

        cs.apply_ui_events(vec![UiEventMessage::ToolCallCompleted {
            tool_call_id: "tc1".into(),
            output: "ok".into(),
            is_error: false,
        }]);
        assert_eq!(
            cs.messages[0].tool_calls[0].output_summary.as_deref(),
            Some("ok")
        );
        assert!(!cs.messages[0].tool_calls[0].is_error);
    }

    #[test]
    fn tool_arg_summary_highlights_task_and_skill_details() {
        assert_eq!(
            summarize_tool_args(
                "Task",
                r#"{"description":"inspect parser","prompt":"find the failing branch"}"#
            ),
            "task=inspect parser"
        );
        assert_eq!(
            summarize_tool_args("skill", r#"{"skillName":"openai-docs"}"#),
            "skill=openai-docs"
        );
    }

    #[test]
    fn raw_events_do_not_render_large_payloads_into_chat() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);

        cs.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "tool/output".into(),
            payload_json: r#"{"content":"fn main() { println!(\"large source\"); }"}"#.into(),
        }]);

        assert!(cs.messages.is_empty());
    }

    #[test]
    fn scroll_state_tracks_manual_navigation_and_bottom_following() {
        let mut cs = ChatState::new("t1".into(), AgentName::Gemini);
        cs.update_max_scroll(40);

        assert_eq!(cs.active_scroll(), 40);

        cs.scroll_up(5);
        assert!(!cs.auto_scroll);
        assert_eq!(cs.active_scroll(), 35);

        cs.scroll_down(3);
        assert_eq!(cs.active_scroll(), 38);

        cs.scroll_down(10);
        assert!(cs.auto_scroll);
        assert_eq!(cs.active_scroll(), 40);
    }
}
