use minos_domain::AgentName;
use minos_ui_protocol::{
    ClaudeTranslatorState, CodexTranslatorState, GeminiTranslatorState, MessageRole,
    OpencodeTranslatorState, UiEventMessage,
};
use std::collections::{HashMap, HashSet};
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
    pub items: Vec<ChatItem>,
    pub pending_requests: Vec<PendingAgentRequest>,
    open_message_ids: HashSet<String>,
    open_message_roles: HashMap<String, MessageRole>,
    completed_assistant_message_ids: HashSet<String>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
    pub selection: Option<ChatSelection>,
    pub version: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChatItem {
    UserMessage {
        message_id: String,
        text_parts: Vec<TextPart>,
        is_streaming: bool,
    },
    AssistantText {
        message_id: String,
        text_parts: Vec<TextPart>,
        is_streaming: bool,
    },
    Reasoning {
        message_id: String,
        text: String,
        is_streaming: bool,
    },
    ToolCall {
        message_id: String,
        tool_call_id: String,
        name: String,
        args_summary: String,
        args_detail: Option<String>,
        output_summary: Option<String>,
        output_detail: Option<String>,
        is_error: bool,
        is_expanded: bool,
        is_streaming: bool,
    },
    SystemMessage {
        text: String,
    },
    Error {
        message_id: Option<String>,
        text: String,
    },
}

impl ChatState {
    pub fn new(thread_id: String, agent: AgentName) -> Self {
        Self {
            translation_state: AgentTranslationState::new(agent, thread_id.clone()),
            thread_id,
            agent,
            items: Vec::new(),
            pending_requests: Vec::new(),
            open_message_ids: HashSet::new(),
            open_message_roles: HashMap::new(),
            completed_assistant_message_ids: HashSet::new(),
            scroll_offset: 0,
            auto_scroll: true,
            max_scroll: 0,
            selection: None,
            version: 0,
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
        for (idx, item) in self.items.iter().enumerate().rev() {
            match item {
                ChatItem::AssistantText {
                    message_id,
                    text_parts,
                    is_streaming: false,
                } if self.completed_assistant_message_ids.contains(message_id) => {
                    if let Some(text) = text_parts_to_string(text_parts) {
                        let key = if message_id.is_empty() {
                            format!("text:{text}")
                        } else {
                            message_id.clone()
                        };
                        return Some((key, text));
                    }
                }
                ChatItem::Error { message_id, text } => {
                    let text = text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    if let Some(message_id) = message_id {
                        if let Some(result) =
                            assistant_text_before_error(&self.items[..idx], message_id)
                        {
                            return Some(result);
                        }
                        return Some((format!("error:{message_id}"), text.to_owned()));
                    }
                    return Some((format!("error:{text}"), text.to_owned()));
                }
                _ => {}
            }
        }
        None
    }

    pub fn finish_all_streaming(&mut self) {
        for item in &mut self.items {
            item.set_streaming(false);
        }
        self.open_message_ids.clear();
        self.open_message_roles.clear();
        self.version += 1;
    }

    pub fn toggle_tool_expansion(&mut self) {
        for item in &mut self.items {
            if let ChatItem::ToolCall { is_expanded, .. } = item {
                *is_expanded = !*is_expanded;
            }
        }
        self.version += 1;
    }

    pub fn active_pending_request(&self) -> Option<&PendingAgentRequest> {
        self.pending_requests.last()
    }

    pub fn resolve_pending_request(&mut self, request_id: &str) -> bool {
        let before = self.pending_requests.len();
        self.pending_requests
            .retain(|request| request.id() != request_id);
        self.pending_requests.len() != before
    }

    fn apply_ui_event(&mut self, event: UiEventMessage) {
        self.version += 1;
        match event {
            UiEventMessage::MessageStarted {
                message_id,
                role,
                started_at_ms: _,
            } => {
                if matches!(role, MessageRole::Assistant) {
                    self.finish_all_streaming();
                }
                self.open_message_ids.insert(message_id.clone());
                self.open_message_roles.insert(message_id, role);
            }
            UiEventMessage::TextDelta { message_id, text } => {
                let text = text.render_preview();
                if text.is_empty() {
                    return;
                }
                if let Some(item) = self.find_text_item_mut(&message_id) {
                    append_text_to_item(item, text);
                } else {
                    self.push_text_item(message_id, text, true);
                }
            }
            UiEventMessage::TextReplace { message_id, text } => {
                let text = text.render_preview();
                if let Some(item) = self.find_text_item_mut(&message_id) {
                    let replacement = if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![TextPart::Plain(text)]
                    };
                    match item {
                        ChatItem::UserMessage { text_parts, .. }
                        | ChatItem::AssistantText { text_parts, .. } => {
                            *text_parts = replacement;
                        }
                        _ => {}
                    }
                } else if !text.is_empty() {
                    let is_streaming = self.open_message_ids.contains(&message_id);
                    self.push_text_item(message_id, text, is_streaming);
                }
            }
            UiEventMessage::ReasoningDelta { message_id, text } => {
                let text = text.render_preview();
                if text.is_empty() {
                    return;
                }
                if let Some(ChatItem::Reasoning { text: existing, .. }) =
                    self.find_reasoning_item_mut(&message_id)
                {
                    existing.push_str(&text);
                } else {
                    self.items.push(ChatItem::Reasoning {
                        message_id,
                        text,
                        is_streaming: true,
                    });
                }
            }
            UiEventMessage::ReasoningReplace { message_id, text } => {
                let text = text.render_preview();
                if text.is_empty() {
                    self.items.retain(|item| {
                        !matches!(
                            item,
                            ChatItem::Reasoning {
                                message_id: item_message_id,
                                ..
                            } if item_message_id == &message_id
                        )
                    });
                } else if let Some(ChatItem::Reasoning { text: existing, .. }) =
                    self.find_reasoning_item_mut(&message_id)
                {
                    *existing = text;
                } else {
                    let is_streaming = self.open_message_ids.contains(&message_id);
                    self.items.push(ChatItem::Reasoning {
                        message_id,
                        text,
                        is_streaming,
                    });
                }
            }
            UiEventMessage::ToolCallPlaced {
                message_id,
                tool_call_id,
                name,
                args_json,
            } => {
                let args_json = args_json.render_preview();
                let args_summary = summarize_tool_args(&name, &args_json);
                let args_detail = compact_tool_args(&args_json)
                    .filter(|detail| !detail.is_empty() && detail != &args_summary);
                let is_expanded = is_diff_like(&args_json);
                if let Some(ChatItem::ToolCall {
                    name: existing_name,
                    args_summary: existing_summary,
                    args_detail: existing_detail,
                    is_expanded: existing_expanded,
                    is_streaming,
                    ..
                }) = self.find_tool_call_item_mut(&tool_call_id)
                {
                    *existing_name = name;
                    *existing_summary = args_summary;
                    *existing_detail = args_detail;
                    *existing_expanded |= is_expanded;
                    *is_streaming = true;
                } else {
                    self.items.push(ChatItem::ToolCall {
                        message_id,
                        tool_call_id,
                        name,
                        args_summary,
                        args_detail,
                        output_summary: None,
                        output_detail: None,
                        is_error: false,
                        is_expanded,
                        is_streaming: true,
                    });
                }
            }
            UiEventMessage::ToolCallCompleted {
                tool_call_id,
                output,
                is_error,
            } => {
                let output = output.render_preview();
                if let Some(ChatItem::ToolCall {
                    output_summary,
                    output_detail,
                    is_error: existing_error,
                    is_expanded,
                    is_streaming,
                    ..
                }) = self.find_tool_call_item_mut(&tool_call_id)
                {
                    *output_summary = Some(summarize_tool_output(&output));
                    *output_detail = tool_output_detail(&output);
                    if is_diff_like(&output) {
                        *is_expanded = true;
                    }
                    *existing_error = is_error;
                    *is_streaming = false;
                }
            }
            UiEventMessage::MessageCompleted { message_id, .. } => {
                if self.message_is_assistant(&message_id) {
                    self.completed_assistant_message_ids
                        .insert(message_id.clone());
                }
                self.open_message_ids.remove(&message_id);
                self.open_message_roles.remove(&message_id);
                for item in &mut self.items {
                    if item.message_id() == Some(message_id.as_str()) {
                        item.set_streaming(false);
                    }
                }
            }
            UiEventMessage::Error {
                message,
                message_id,
                ..
            } => {
                self.finish_all_streaming();
                self.items.push(ChatItem::Error {
                    message_id,
                    text: message,
                });
            }
            UiEventMessage::Raw { kind, payload_json } => {
                if self.apply_raw_request_event(&kind, &payload_json) {
                    return;
                }
                debug!(
                    raw_kind = %kind,
                    payload_bytes = payload_json.len(),
                    "raw ui event suppressed from chat"
                );
            }
            UiEventMessage::ThreadOpened { .. } | UiEventMessage::ThreadTitleUpdated { .. } => {}
            UiEventMessage::ThreadClosed { reason, .. } => {
                self.finish_all_streaming();
                self.items.push(ChatItem::SystemMessage {
                    text: format!("Thread closed: {reason:?}"),
                });
            }
        }
    }

    fn push_text_item(&mut self, message_id: String, text: String, is_streaming: bool) {
        let role = self.infer_role(&message_id);
        let item = match role {
            MessageRole::User => ChatItem::UserMessage {
                message_id: message_id.clone(),
                text_parts: vec![TextPart::Plain(text)],
                is_streaming,
            },
            MessageRole::Assistant | MessageRole::System => ChatItem::AssistantText {
                message_id: message_id.clone(),
                text_parts: vec![TextPart::Plain(text)],
                is_streaming,
            },
        };
        self.items.push(item);
    }

    fn find_text_item_mut(&mut self, message_id: &str) -> Option<&mut ChatItem> {
        self.items.iter_mut().rev().find(|item| {
            matches!(
                item,
                ChatItem::UserMessage { .. } | ChatItem::AssistantText { .. }
            ) && item.message_id() == Some(message_id)
        })
    }

    fn find_reasoning_item_mut(&mut self, message_id: &str) -> Option<&mut ChatItem> {
        self.items.iter_mut().rev().find(|item| {
            matches!(item, ChatItem::Reasoning { .. }) && item.message_id() == Some(message_id)
        })
    }

    fn find_tool_call_item_mut(&mut self, tool_call_id: &str) -> Option<&mut ChatItem> {
        self.items.iter_mut().rev().find(|item| {
            matches!(item, ChatItem::ToolCall { tool_call_id: id, .. } if id == tool_call_id)
        })
    }

    fn infer_role(&self, message_id: &str) -> MessageRole {
        self.open_message_roles
            .get(message_id)
            .copied()
            .unwrap_or_else(|| {
                warn!(
                    target: "minos_tui::translation",
                    message_id,
                    "message role missing; defaulting to assistant"
                );
                MessageRole::Assistant
            })
    }

    fn message_is_assistant(&self, message_id: &str) -> bool {
        self.open_message_roles
            .get(message_id)
            .is_some_and(|role| matches!(role, MessageRole::Assistant | MessageRole::System))
            || self.items.iter().any(|item| {
                matches!(item, ChatItem::AssistantText { message_id: item_message_id, .. } if item_message_id.as_str() == message_id)
            })
    }

    fn apply_raw_request_event(&mut self, kind: &str, payload_json: &str) -> bool {
        match kind {
            "approval/request" => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) else {
                    return false;
                };
                let Some(request) = PendingAgentRequest::from_approval_request(&value) else {
                    return false;
                };
                self.push_pending_request_message(request);
                true
            }
            "approval/timeout" => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) {
                    if let Some(request_id) =
                        value.get("request_id").and_then(serde_json::Value::as_str)
                    {
                        self.resolve_pending_request(request_id);
                        self.items.push(ChatItem::SystemMessage {
                            text: format!("Request timed out: {request_id}"),
                        });
                        return true;
                    }
                }
                false
            }
            "opencode/permission.updated" => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) else {
                    return false;
                };
                if opencode_permission_is_completed(&value) {
                    if let Some(permission_id) = opencode_permission_id(&value) {
                        self.resolve_pending_request(&permission_id);
                    }
                    return true;
                }
                let Some(request) = PendingAgentRequest::from_opencode_permission(&value) else {
                    return false;
                };
                self.push_pending_request_message(request);
                true
            }
            "opencode/question.asked" => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) else {
                    return false;
                };
                let Some(request) = PendingAgentRequest::from_opencode_question(&value) else {
                    return false;
                };
                self.push_pending_request_message(request);
                true
            }
            "opencode/question.replied" | "opencode/question.rejected" => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) else {
                    return false;
                };
                if let Some(question_id) = opencode_question_reply_id(&value) {
                    self.resolve_pending_request(&question_id);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn push_pending_request_message(&mut self, request: PendingAgentRequest) {
        if self
            .pending_requests
            .iter()
            .any(|pending| pending.id() == request.id())
        {
            return;
        }
        let prompt = request.prompt.clone();
        self.pending_requests.push(request);
        self.items.push(ChatItem::SystemMessage { text: prompt });
    }
}

impl ChatItem {
    fn message_id(&self) -> Option<&str> {
        match self {
            ChatItem::UserMessage { message_id, .. }
            | ChatItem::AssistantText { message_id, .. }
            | ChatItem::Reasoning { message_id, .. }
            | ChatItem::ToolCall { message_id, .. } => Some(message_id),
            ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => None,
        }
    }

    fn set_streaming(&mut self, value: bool) {
        match self {
            ChatItem::UserMessage { is_streaming, .. }
            | ChatItem::AssistantText { is_streaming, .. }
            | ChatItem::Reasoning { is_streaming, .. }
            | ChatItem::ToolCall { is_streaming, .. } => *is_streaming = value,
            ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => {}
        }
    }
}

fn assistant_text_before_error(items: &[ChatItem], message_id: &str) -> Option<(String, String)> {
    items.iter().rev().find_map(|item| {
        let ChatItem::AssistantText {
            message_id: item_message_id,
            text_parts,
            is_streaming: false,
        } = item
        else {
            return None;
        };
        (item_message_id == message_id)
            .then(|| text_parts_to_string(text_parts).map(|text| (item_message_id.clone(), text)))
            .flatten()
    })
}

fn text_parts_to_string(parts: &[TextPart]) -> Option<String> {
    let mut result = Vec::new();
    for part in parts {
        match part {
            TextPart::Plain(text) => {
                if !text.trim().is_empty() {
                    result.push(text.trim().to_owned());
                }
            }
            TextPart::Code { code, .. } => {
                if !code.trim().is_empty() {
                    result.push(code.trim().to_owned());
                }
            }
        }
    }

    (!result.is_empty()).then(|| result.join("\n"))
}

fn append_text_to_item(item: &mut ChatItem, text: String) {
    match item {
        ChatItem::UserMessage { text_parts, .. } | ChatItem::AssistantText { text_parts, .. } => {
            if let Some(TextPart::Plain(last)) = text_parts.last_mut() {
                last.push_str(&text);
            } else {
                text_parts.push(TextPart::Plain(text));
            }
        }
        _ => {}
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

fn summarize_tool_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if is_diff_like(trimmed) {
        let add = trimmed
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count();
        let del = trimmed
            .lines()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count();
        return format!("diff +{add} -{del}");
    }
    truncate_str(&one_line(trimmed), 220)
}

fn tool_output_detail(output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_diff_like(trimmed) || trimmed.len() > 220 || trimmed.contains('\n') {
        return Some(truncate_str(trimmed, 6000));
    }
    None
}

fn is_diff_like(text: &str) -> bool {
    text.contains("diff --git")
        || text.contains("\n@@")
        || text.starts_with("@@")
        || text.contains("*** Begin Patch")
        || text.contains("*** Update File:")
        || text.contains("*** Add File:")
        || text.contains("*** Delete File:")
        || text
            .lines()
            .any(|line| line.starts_with("+++ ") || line.starts_with("--- "))
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAgentRequest {
    pub prompt: String,
    pub kind: PendingAgentRequestKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAgentRequestKind {
    CodexUserInput {
        request_id: String,
        question_ids: Vec<String>,
    },
    CodexApproval {
        request_id: String,
        method: String,
    },
    OpencodePermission {
        permission_id: String,
        approve_response: String,
        decline_response: String,
    },
    OpencodeQuestion {
        question_id: String,
        questions: Vec<PendingQuestionSpec>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingQuestionSpec {
    pub header: String,
    pub question: String,
    pub options: Vec<PendingQuestionOption>,
    pub multiple: bool,
    pub custom: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingQuestionOption {
    pub label: String,
    pub description: String,
}

impl PendingAgentRequest {
    pub fn id(&self) -> &str {
        match &self.kind {
            PendingAgentRequestKind::CodexUserInput { request_id, .. }
            | PendingAgentRequestKind::CodexApproval { request_id, .. } => request_id,
            PendingAgentRequestKind::OpencodePermission { permission_id, .. } => permission_id,
            PendingAgentRequestKind::OpencodeQuestion { question_id, .. } => question_id,
        }
    }

    fn from_approval_request(value: &serde_json::Value) -> Option<Self> {
        let request_id = value.get("request_id")?.as_str()?.to_owned();
        let method = value.get("method")?.as_str()?.to_owned();
        let params = value.get("params").unwrap_or(&serde_json::Value::Null);

        if method == "item/tool/requestUserInput" {
            let questions = params
                .get("questions")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let question_ids = questions
                .iter()
                .filter_map(|question| question.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let prompt = format_user_input_prompt(&questions);
            return Some(Self {
                prompt,
                kind: PendingAgentRequestKind::CodexUserInput {
                    request_id,
                    question_ids,
                },
            });
        }

        Some(Self {
            prompt: format_approval_prompt(&method, params),
            kind: PendingAgentRequestKind::CodexApproval { request_id, method },
        })
    }

    fn from_opencode_permission(value: &serde_json::Value) -> Option<Self> {
        if opencode_permission_is_completed(value) {
            return None;
        }

        let permission_id = opencode_permission_id(value)?;
        let title = find_string_by_keys(value, &["title", "name", "tool", "action"])
            .unwrap_or_else(|| "permission request".to_owned());
        let description =
            find_string_by_keys(value, &["description", "message", "reason"]).unwrap_or_default();
        let prompt = if description.is_empty() {
            format!("Opencode asks for permission: {title}")
        } else {
            format!("Opencode asks for permission: {title}\n{description}")
        };
        Some(Self {
            prompt,
            kind: PendingAgentRequestKind::OpencodePermission {
                permission_id,
                approve_response: find_permission_option_response(value, true)
                    .unwrap_or_else(|| "accept".to_owned()),
                decline_response: find_permission_option_response(value, false)
                    .unwrap_or_else(|| "reject".to_owned()),
            },
        })
    }

    fn from_opencode_question(value: &serde_json::Value) -> Option<Self> {
        let properties = value.get("properties").unwrap_or(value);
        let question_id = properties
            .get("id")
            .or_else(|| properties.get("requestID"))
            .or_else(|| value.get("id"))
            .and_then(serde_json::Value::as_str)?
            .to_owned();
        let raw_questions = properties
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let questions = parse_pending_questions(&raw_questions);
        let prompt = format_pending_question_prompt("Opencode asks:", &questions);
        Some(Self {
            prompt,
            kind: PendingAgentRequestKind::OpencodeQuestion {
                question_id,
                questions,
            },
        })
    }
}

fn find_permission_option_response(value: &serde_json::Value, approve: bool) -> Option<String> {
    let options = find_array_by_key(value, "options")?;
    for option in options {
        let label = find_string_by_keys(
            option,
            &[
                "kind",
                "name",
                "label",
                "title",
                "description",
                "optionId",
                "id",
            ],
        )
        .unwrap_or_default()
        .to_ascii_lowercase();
        let is_match = if approve {
            label.contains("allow")
                || label.contains("approve")
                || label.contains("accept")
                || label.contains("yes")
                || label.contains("proceed")
        } else {
            label.contains("reject")
                || label.contains("deny")
                || label.contains("decline")
                || label.contains("cancel")
                || label.contains("no")
        };
        if !is_match {
            continue;
        }
        if let Some(response) =
            find_string_by_keys(option, &["optionId", "optionID", "id", "value"])
        {
            return Some(response);
        }
    }
    None
}

fn opencode_permission_id(value: &serde_json::Value) -> Option<String> {
    let keys = ["permissionID", "permissionId", "permission_id", "id"];
    direct_string_by_keys(value, &keys)
        .or_else(|| {
            value
                .get("properties")
                .and_then(|properties| direct_string_by_keys(properties, &keys))
        })
        .or_else(|| {
            value
                .get("permission")
                .and_then(|permission| direct_string_by_keys(permission, &keys))
                .or_else(|| {
                    value
                        .get("properties")
                        .and_then(|properties| properties.get("permission"))
                        .and_then(|permission| direct_string_by_keys(permission, &keys))
                })
        })
        .or_else(|| {
            value
                .get("permission")
                .filter(|permission| !permission.is_object())
                .and_then(json_value_summary)
                .or_else(|| {
                    value
                        .get("properties")
                        .and_then(|properties| properties.get("permission"))
                        .filter(|permission| !permission.is_object())
                        .and_then(json_value_summary)
                })
        })
        .or_else(|| find_string_by_keys(value, &["permissionID", "permissionId", "permission_id"]))
}

fn opencode_permission_is_completed(value: &serde_json::Value) -> bool {
    let Some(status) = find_permission_status(value) else {
        return false;
    };
    matches!(
        status.to_ascii_lowercase().as_str(),
        "approved" | "accepted" | "rejected" | "declined" | "denied" | "completed"
    )
}

fn find_permission_status(value: &serde_json::Value) -> Option<String> {
    value
        .get("permission")
        .or_else(|| {
            value
                .get("properties")
                .and_then(|props| props.get("permission"))
        })
        .and_then(|permission| find_string_by_keys(permission, &["status", "state"]))
        .or_else(|| {
            value
                .get("status")
                .or_else(|| value.get("state"))
                .and_then(json_value_summary)
        })
        .or_else(|| find_string_by_keys(value, &["status", "state"]))
}

fn find_array_by_key<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Option<&'a Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(array) = map.get(key).and_then(serde_json::Value::as_array) {
                return Some(array);
            }
            map.values().find_map(|child| find_array_by_key(child, key))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|child| find_array_by_key(child, key)),
        _ => None,
    }
}

fn format_user_input_prompt(questions: &[serde_json::Value]) -> String {
    if questions.is_empty() {
        return "Agent asks for input. Type your answer in Agent Input.".into();
    }

    let parsed = parse_pending_questions(questions);
    format_pending_question_prompt("Agent asks for input:", &parsed)
}

fn format_approval_prompt(method: &str, params: &serde_json::Value) -> String {
    let summary = match method {
        "item/commandExecution/requestApproval" => {
            find_string_by_keys(params, &["command", "cmd", "script"]).unwrap_or_default()
        }
        "item/fileChange/requestApproval" => {
            find_string_by_keys(params, &["file", "path", "file_path", "filePath"])
                .unwrap_or_default()
        }
        _ => find_string_by_keys(params, &["reason", "message", "title"]).unwrap_or_default(),
    };
    if summary.is_empty() {
        format!("Approval required: {method}\nType yes to approve, anything else to decline.")
    } else {
        format!("Approval required: {method}\n{summary}\nType yes to approve, anything else to decline.")
    }
}

fn parse_pending_questions(questions: &[serde_json::Value]) -> Vec<PendingQuestionSpec> {
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let header = question
                .get("header")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned();
            let text = question
                .get("question")
                .or_else(|| question.get("text"))
                .or_else(|| question.get("prompt"))
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Question {}", index + 1));
            let options = question
                .get("options")
                .and_then(serde_json::Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| {
                            let label = option
                                .get("label")
                                .or_else(|| option.get("value"))
                                .or_else(|| option.get("id"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .trim()
                                .to_owned();
                            if label.is_empty() {
                                return None;
                            }
                            let description = option
                                .get("description")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .trim()
                                .to_owned();
                            Some(PendingQuestionOption { label, description })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            PendingQuestionSpec {
                header,
                question: text,
                options,
                multiple: question
                    .get("multiple")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                custom: question
                    .get("custom")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            }
        })
        .collect()
}

fn format_pending_question_prompt(prefix: &str, questions: &[PendingQuestionSpec]) -> String {
    if questions.is_empty() {
        return format!("{prefix}\nReply in Agent Input; Shift+Enter inserts a newline.");
    }

    let mut lines = vec![prefix.to_owned()];
    for (question_index, question) in questions.iter().enumerate() {
        let label = if question.header.is_empty() {
            format!("Question {}", question_index + 1)
        } else {
            question.header.clone()
        };
        lines.push(format!("- {label}: {}", question.question));
        for (option_index, option) in question.options.iter().enumerate() {
            if option.description.is_empty() {
                lines.push(format!("  {}. {}", option_index + 1, option.label));
            } else {
                lines.push(format!(
                    "  {}. {}: {}",
                    option_index + 1,
                    option.label,
                    option.description
                ));
            }
        }
        if question.multiple {
            lines.push("  Select multiple with comma-separated numbers or labels.".into());
        }
        if question.custom {
            lines.push("  Custom text is allowed.".into());
        }
    }
    lines.push("Reply in Agent Input; use one line per question.".into());
    lines.join("\n")
}

fn opencode_question_reply_id(value: &serde_json::Value) -> Option<String> {
    let properties = value.get("properties").unwrap_or(value);
    properties
        .get("requestID")
        .or_else(|| properties.get("request_id"))
        .or_else(|| properties.get("id"))
        .or_else(|| value.get("requestID"))
        .or_else(|| value.get("request_id"))
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn find_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    find_string_by_keys_inner(value, keys, 0)
}

fn direct_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let serde_json::Value::Object(map) = value else {
        return None;
    };
    keys.iter()
        .find_map(|key| map.get(*key).and_then(json_value_summary))
}

fn find_string_by_keys_inner(
    value: &serde_json::Value,
    keys: &[&str],
    depth: usize,
) -> Option<String> {
    if depth > 5 {
        return None;
    }
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(text) = map.get(*key).and_then(json_value_summary) {
                    return Some(text);
                }
            }
            map.values()
                .find_map(|child| find_string_by_keys_inner(child, keys, depth + 1))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|child| find_string_by_keys_inner(child, keys, depth + 1)),
        _ => None,
    }
}

fn json_value_summary(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => (!text.trim().is_empty()).then(|| text.to_owned()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Array(values) => {
            let parts = values
                .iter()
                .filter_map(json_value_summary)
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        serde_json::Value::Object(map) => {
            for key in [
                "title", "name", "path", "command", "cmd", "message", "reason",
            ] {
                if let Some(text) = map.get(key).and_then(json_value_summary) {
                    return Some(text);
                }
            }
            None
        }
        serde_json::Value::Null => None,
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPart {
    Plain(String),
    #[allow(dead_code)]
    Code {
        lang: String,
        code: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;

    fn plain_parts(text: &str) -> Vec<TextPart> {
        vec![TextPart::Plain(text.into())]
    }

    #[test]
    fn chat_state_message_started_then_text_delta() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::User,
            started_at_ms: 0,
        }]);
        assert!(cs.items.is_empty());
        assert!(cs.open_message_ids.contains("m1"));

        cs.apply_ui_events(vec![UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "hello ".into(),
        }]);
        cs.apply_ui_events(vec![UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "world".into(),
        }]);
        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::UserMessage {
                text_parts,
                is_streaming,
                ..
            } => {
                assert_eq!(*text_parts, plain_parts("hello world"));
                assert!(*is_streaming);
            }
            other => panic!("expected UserMessage, got {other:?}"),
        }
        assert!(cs.open_message_ids.contains("m1"));

        cs.apply_ui_events(vec![UiEventMessage::MessageCompleted {
            message_id: "m1".into(),
            finished_at_ms: 1,
        }]);
        match &cs.items[0] {
            ChatItem::UserMessage { is_streaming, .. } => assert!(!*is_streaming),
            other => panic!("expected UserMessage, got {other:?}"),
        }
        assert!(!cs.open_message_ids.contains("m1"));
    }

    #[test]
    fn assistant_text_reasoning_and_tool_appear_in_arrival_order() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ReasoningDelta {
                message_id: "m1".into(),
                text: "let me think".into(),
            },
            UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "read_file".into(),
                args_json: r#"{"path":"foo.rs"}"#.into(),
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "answer".into(),
            },
            UiEventMessage::MessageCompleted {
                message_id: "m1".into(),
                finished_at_ms: 1,
            },
        ]);

        assert!(matches!(cs.items[0], ChatItem::Reasoning { .. }));
        assert!(matches!(cs.items[1], ChatItem::ToolCall { .. }));
        assert!(matches!(cs.items[2], ChatItem::AssistantText { .. }));
        for item in &cs.items {
            match item {
                ChatItem::Reasoning { is_streaming, .. }
                | ChatItem::ToolCall { is_streaming, .. }
                | ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
                other => panic!("unexpected item {other:?}"),
            }
        }
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
        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::ToolCall {
                name,
                args_summary,
                is_streaming,
                ..
            } => {
                assert_eq!(name, "write_file");
                assert_eq!(args_summary, "file=foo.rs");
                assert!(*is_streaming);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }

        cs.apply_ui_events(vec![UiEventMessage::ToolCallCompleted {
            tool_call_id: "tc1".into(),
            output: "ok".into(),
            is_error: false,
        }]);
        match &cs.items[0] {
            ChatItem::ToolCall {
                output_summary,
                is_error,
                is_streaming,
                ..
            } => {
                assert_eq!(output_summary.as_deref(), Some("ok"));
                assert!(!*is_error);
                assert!(!*is_streaming);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_tool_call_placed_updates_existing_tool_block() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        }]);
        cs.apply_ui_events(vec![UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tool-1".into(),
            name: "commandExecution".into(),
            args_json: r#"{"command":"ls"}"#.into(),
        }]);
        cs.apply_ui_events(vec![UiEventMessage::ToolCallPlaced {
            message_id: "m1".into(),
            tool_call_id: "tool-1".into(),
            name: "commandExecution".into(),
            args_json: r#"{"command":"ls -la"}"#.into(),
        }]);

        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::ToolCall { args_summary, .. } => assert!(args_summary.contains("ls -la")),
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn codex_raw_events_render_final_text_and_tool_as_separate_items() {
        let mut cs = ChatState::new("thr".into(), AgentName::Codex);
        for raw in [
            serde_json::json!({"method":"item/started","params":{
                "item":{"type":"agentMessage","id":"a1","text":""},
                "threadId":"thr","turnId":"t1"
            }}),
            serde_json::json!({"method":"item/agentMessage/delta","params":{
                "itemId":"a1","delta":"partial"
            }}),
            serde_json::json!({"method":"item/started","params":{
                "item":{"type":"commandExecution","id":"cmd1","command":"ls","commandActions":[],"cwd":"/tmp","status":"inProgress"},
                "threadId":"thr","turnId":"t1"
            }}),
            serde_json::json!({"method":"item/started","params":{
                "item":{"type":"agentMessage","id":"a2","text":""},
                "threadId":"thr","turnId":"t1"
            }}),
            serde_json::json!({"method":"item/completed","params":{
                "item":{"type":"agentMessage","id":"a1","text":"partial final answer"},
                "threadId":"thr","turnId":"t1","completedAtMs":2
            }}),
            serde_json::json!({"method":"item/completed","params":{
                "item":{"type":"commandExecution","id":"cmd1","command":"ls","commandActions":[],"cwd":"/tmp","status":"completed","aggregatedOutput":"ok","exitCode":0},
                "threadId":"thr","turnId":"t1","completedAtMs":3
            }}),
        ] {
            let events = cs.translation_state.translate(&raw);
            cs.apply_ui_events(events);
        }

        assert_eq!(cs.items.len(), 2);
        match &cs.items[0] {
            ChatItem::AssistantText { text_parts, .. } => {
                assert_eq!(*text_parts, plain_parts("partial final answer"));
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
        match &cs.items[1] {
            ChatItem::ToolCall { output_summary, .. } => {
                assert_eq!(output_summary.as_deref(), Some("ok"));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn text_replace_uses_completed_agent_message_as_authoritative() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        }]);
        cs.apply_ui_events(vec![UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "partial ans".into(),
        }]);
        cs.apply_ui_events(vec![UiEventMessage::TextReplace {
            message_id: "m1".into(),
            text: "partial answer with final sentence".into(),
        }]);

        match &cs.items[0] {
            ChatItem::AssistantText { text_parts, .. } => {
                assert_eq!(
                    *text_parts,
                    plain_parts("partial answer with final sentence")
                );
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn text_replace_without_delta_creates_streaming_item_for_open_message() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextReplace {
                message_id: "m1".into(),
                text: "authoritative answer".into(),
            },
        ]);

        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::AssistantText {
                text_parts,
                is_streaming,
                ..
            } => {
                assert_eq!(*text_parts, plain_parts("authoritative answer"));
                assert!(*is_streaming);
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
        assert!(cs.open_message_ids.contains("m1"));
    }

    #[test]
    fn reasoning_replace_uses_completed_reasoning_as_authoritative() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        }]);
        cs.apply_ui_events(vec![UiEventMessage::ReasoningDelta {
            message_id: "m1".into(),
            text: "old".into(),
        }]);
        cs.apply_ui_events(vec![UiEventMessage::ReasoningReplace {
            message_id: "m1".into(),
            text: "final thinking".into(),
        }]);

        match &cs.items[0] {
            ChatItem::Reasoning { text, .. } => assert_eq!(text, "final thinking"),
            other => panic!("expected Reasoning, got {other:?}"),
        }
    }

    #[test]
    fn reasoning_replace_without_delta_creates_streaming_item_for_open_message() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ReasoningReplace {
                message_id: "m1".into(),
                text: "final thinking".into(),
            },
        ]);

        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::Reasoning {
                text, is_streaming, ..
            } => {
                assert_eq!(text, "final thinking");
                assert!(*is_streaming);
            }
            other => panic!("expected Reasoning, got {other:?}"),
        }
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
    fn markdown_list_tool_output_is_not_summarized_as_diff() {
        let output = "- first item\n- second item";

        assert!(!is_diff_like(output));
        assert_eq!(summarize_tool_output(output), "- first item - second item");
    }

    #[test]
    fn diff_tool_output_summarizes_changed_lines() {
        let output = "@@ -1 +1\n-old\n+new";

        assert!(is_diff_like(output));
        assert_eq!(summarize_tool_output(output), "diff +1 -1");
    }

    #[test]
    fn raw_events_do_not_render_large_payloads_into_chat() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);

        cs.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "tool/output".into(),
            payload_json: r#"{"content":"fn main() { println!(\"large source\"); }"}"#.into(),
        }]);

        assert!(cs.items.is_empty());
    }

    #[test]
    fn opencode_permission_update_creates_pending_request() {
        let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

        cs.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "opencode/permission.updated".into(),
            payload_json: serde_json::json!({
                "type": "permission.updated",
                "properties": {
                    "permission": {
                        "id": "perm-1",
                        "title": "Run shell",
                        "options": [
                            {"optionId": "allow_once", "kind": "allow"},
                            {"optionId": "reject_once", "kind": "reject"}
                        ]
                    }
                }
            })
            .to_string(),
        }]);

        assert_eq!(cs.pending_requests.len(), 1);
        assert_eq!(cs.pending_requests[0].id(), "perm-1");
        assert!(cs.pending_requests[0].prompt.contains("Run shell"));
        assert_eq!(
            cs.pending_requests[0].kind,
            PendingAgentRequestKind::OpencodePermission {
                permission_id: "perm-1".into(),
                approve_response: "allow_once".into(),
                decline_response: "reject_once".into()
            }
        );
        assert_eq!(cs.items.len(), 1);
        assert!(matches!(cs.items[0], ChatItem::SystemMessage { .. }));
    }

    #[test]
    fn opencode_permission_completion_clears_pending_request() {
        let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

        cs.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "opencode/permission.updated".into(),
            payload_json: serde_json::json!({
                "permissionID": "perm-1",
                "title": "Run shell",
                "options": [
                    {"optionId": "allow_once", "kind": "allow"},
                    {"optionId": "reject_once", "kind": "reject"}
                ]
            })
            .to_string(),
        }]);
        assert_eq!(cs.pending_requests.len(), 1);

        cs.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "opencode/permission.updated".into(),
            payload_json: serde_json::json!({
                "type": "permission.updated",
                "properties": {
                    "permission": {
                        "id": "perm-1",
                        "status": "rejected"
                    }
                }
            })
            .to_string(),
        }]);

        assert!(cs.pending_requests.is_empty());
        assert_eq!(cs.items.len(), 1);
    }

    #[test]
    fn opencode_question_asked_creates_pending_request_with_options() {
        let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

        cs.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "opencode/question.asked".into(),
            payload_json: serde_json::json!({
                "type": "question.asked",
                "properties": {
                    "id": "que-1",
                    "questions": [{
                        "header": "Core",
                        "question": "Pick a direction",
                        "options": [
                            {"label": "Fast", "description": "Ship quickly"},
                            {"label": "Robust", "description": "Prefer durability"}
                        ]
                    }]
                }
            })
            .to_string(),
        }]);

        assert_eq!(cs.pending_requests.len(), 1);
        assert_eq!(cs.pending_requests[0].id(), "que-1");
        assert!(cs.pending_requests[0].prompt.contains("1. Fast"));
        assert_eq!(
            cs.pending_requests[0].kind,
            PendingAgentRequestKind::OpencodeQuestion {
                question_id: "que-1".into(),
                questions: vec![PendingQuestionSpec {
                    header: "Core".into(),
                    question: "Pick a direction".into(),
                    options: vec![
                        PendingQuestionOption {
                            label: "Fast".into(),
                            description: "Ship quickly".into(),
                        },
                        PendingQuestionOption {
                            label: "Robust".into(),
                            description: "Prefer durability".into(),
                        },
                    ],
                    multiple: false,
                    custom: false,
                }]
            }
        );
        assert!(matches!(cs.items[0], ChatItem::SystemMessage { .. }));
    }

    #[test]
    fn new_assistant_message_finishes_previous_streaming_assistant() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);

        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "first".into(),
            },
            UiEventMessage::MessageStarted {
                message_id: "m2".into(),
                role: MessageRole::Assistant,
                started_at_ms: 1,
            },
        ]);

        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
            other => panic!("expected AssistantText, got {other:?}"),
        }
        assert!(cs.open_message_ids.contains("m2"));
    }

    #[test]
    fn last_completed_assistant_text_ignores_text_finished_only_by_next_message_start() {
        let mut cs = ChatState::new("t1".into(), AgentName::Opencode);

        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "intermediate".into(),
            },
            UiEventMessage::MessageStarted {
                message_id: "m2".into(),
                role: MessageRole::Assistant,
                started_at_ms: 1,
            },
        ]);

        assert_eq!(cs.last_completed_assistant_text(), None);
    }

    #[test]
    fn error_pushes_error_item_and_finishes_streaming() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);

        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "partial".into(),
            },
            UiEventMessage::Error {
                code: "failed".into(),
                message: "tool failed".into(),
                message_id: Some("m1".into()),
            },
        ]);

        assert_eq!(cs.items.len(), 2);
        match &cs.items[0] {
            ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
            other => panic!("expected AssistantText, got {other:?}"),
        }
        match &cs.items[1] {
            ChatItem::Error { message_id, text } => {
                assert_eq!(message_id.as_deref(), Some("m1"));
                assert_eq!(text, "tool failed");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn thread_closed_pushes_system_message_and_finishes_streaming() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);

        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "partial".into(),
            },
            UiEventMessage::ThreadClosed {
                thread_id: "t1".into(),
                reason: minos_ui_protocol::ThreadEndReason::UserStopped,
                closed_at_ms: 1,
            },
        ]);

        match &cs.items[0] {
            ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
            other => panic!("expected AssistantText, got {other:?}"),
        }
        match &cs.items[1] {
            ChatItem::SystemMessage { text } => assert!(text.contains("Thread closed")),
            other => panic!("expected SystemMessage, got {other:?}"),
        }
    }

    #[test]
    fn last_completed_assistant_text_ignores_streaming_items() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "first".into(),
            },
            UiEventMessage::MessageCompleted {
                message_id: "m1".into(),
                finished_at_ms: 1,
            },
            UiEventMessage::MessageStarted {
                message_id: "m2".into(),
                role: MessageRole::Assistant,
                started_at_ms: 2,
            },
            UiEventMessage::TextDelta {
                message_id: "m2".into(),
                text: "second streaming".into(),
            },
        ]);

        assert_eq!(
            cs.last_completed_assistant_text(),
            Some(("m1".into(), "first".into()))
        );
    }

    #[test]
    fn last_completed_assistant_text_falls_back_to_targeted_error_without_text() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::Error {
                code: "failed".into(),
                message: "tool failed".into(),
                message_id: Some("m1".into()),
            },
        ]);

        assert_eq!(
            cs.last_completed_assistant_text(),
            Some(("error:m1".into(), "tool failed".into()))
        );
    }

    #[test]
    fn last_completed_assistant_text_prefers_text_before_targeted_error() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "partial answer".into(),
            },
            UiEventMessage::Error {
                code: "failed".into(),
                message: "tool failed".into(),
                message_id: Some("m1".into()),
            },
        ]);

        assert_eq!(
            cs.last_completed_assistant_text(),
            Some(("m1".into(), "partial answer".into()))
        );
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

    #[test]
    fn toggle_tool_expansion_bumps_version() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        cs.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "bash".into(),
                args_json: r#"{"command":"ls"}"#.into(),
            },
        ]);
        let version_before = cs.version;

        cs.toggle_tool_expansion();
        assert!(cs.version > version_before);

        match &cs.items[0] {
            ChatItem::ToolCall { is_expanded, .. } => assert!(*is_expanded),
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }
}
