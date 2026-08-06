use std::collections::{HashMap, HashSet};

use minos_domain::AgentName;
use minos_ui_protocol::{MessageRole, SubagentStatus, UiEventMessage};
use tracing::{debug, warn};

use super::chat_item::{ChatItem, TextPart};
use super::pending_request::{
    opencode_permission_id, opencode_permission_is_completed, opencode_question_reply_id,
    PendingAgentRequest,
};
use super::selection::{ChatSelection, ChatSelectionPoint};
use super::tool_summary::{
    compact_tool_args, is_diff_like, summarize_tool_args, summarize_tool_output, tool_output_detail,
};
use super::verb_group::{run_anchor_id, run_containing};

pub struct ChatState {
    pub session_id: String,
    pub agent: AgentName,
    pub items: Vec<ChatItem>,
    pub pending_requests: Vec<PendingAgentRequest>,
    pub(super) open_message_ids: HashSet<String>,
    open_message_roles: HashMap<String, MessageRole>,
    completed_assistant_message_ids: HashSet<String>,
    pub scroll_offset: u32,
    pub auto_scroll: bool,
    pub max_scroll: u32,
    pub selection: Option<ChatSelection>,
    /// Anchor tool_call_ids of user-expanded verb-group runs.
    pub verb_group_expanded: HashSet<String>,
    /// Bumps on any visual change (content or structure). Cache invalidation key.
    pub version: u64,
    /// Bumps when the item list structure changes (push/remove/reorder).
    /// Streaming text into an existing item leaves this unchanged so the render
    /// cache can rebuild only dirty tail segments.
    pub structure_version: u64,
}

impl ChatState {
    pub fn new(session_id: String, agent: AgentName) -> Self {
        Self {
            session_id,
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
            verb_group_expanded: HashSet::new(),
            version: 0,
            structure_version: 0,
        }
    }

    fn bump_content(&mut self) {
        self.version = self.version.saturating_add(1);
    }

    fn bump_structure(&mut self) {
        self.structure_version = self.structure_version.saturating_add(1);
        self.bump_content();
    }

    pub fn update_max_scroll(&mut self, max_scroll: u32) {
        self.max_scroll = max_scroll;
        if !self.auto_scroll {
            self.scroll_offset = self.scroll_offset.min(self.max_scroll);
        }
    }

    pub fn active_scroll(&self) -> u32 {
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
        self.scroll_offset = self.scroll_offset.saturating_sub(u32::from(lines));
    }

    pub fn scroll_down(&mut self, lines: u16) {
        if self.auto_scroll {
            return;
        }

        self.scroll_offset = self
            .scroll_offset
            .saturating_add(u32::from(lines))
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

    #[allow(dead_code)] // exercised by unit tests; used for result-capture helpers
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
        let mut changed = false;
        for item in &mut self.items {
            if item.is_streaming() {
                item.set_streaming(false);
                changed = true;
            }
        }
        if !self.open_message_ids.is_empty() || !self.open_message_roles.is_empty() {
            changed = true;
        }
        self.open_message_ids.clear();
        self.open_message_roles.clear();
        if changed {
            // Streaming flags change fingerprints of existing items only.
            self.bump_content();
        }
    }

    /// Toggle fold on the most recent tool, thinking, or verb-group (`e` key).
    pub fn toggle_tool_expansion(&mut self) -> bool {
        for index in (0..self.items.len()).rev() {
            if self.items[index].is_foldable()
                || matches!(self.items[index], ChatItem::SubagentCall { .. })
            {
                return self.toggle_fold_at(index);
            }
        }
        false
    }

    /// Toggle fold for a tool/thinking item, or expand/collapse its verb-group.
    ///
    /// When `index` is the start of a folding verb-group run, toggles the **group**
    /// (Grok: click header → expand members). Otherwise toggles the individual item.
    pub fn toggle_fold_at(&mut self, index: usize) -> bool {
        if let Some(run) = run_containing(&self.items, index, &self.verb_group_expanded) {
            if index == run.start {
                return self.toggle_verb_group_at(run.start);
            }
        }

        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        match item {
            ChatItem::ToolCall {
                is_expanded,
                is_user_toggled,
                ..
            } => {
                let current = is_user_toggled.unwrap_or(*is_expanded);
                *is_user_toggled = Some(!current);
                self.bump_content();
                true
            }
            ChatItem::Reasoning {
                is_streaming,
                is_user_toggled,
                ..
            } => {
                let current = is_user_toggled.unwrap_or(*is_streaming);
                *is_user_toggled = Some(!current);
                self.bump_content();
                true
            }
            _ => false,
        }
    }

    /// Expand/collapse a verb-group run starting at `start`.
    pub fn toggle_verb_group_at(&mut self, start: usize) -> bool {
        let Some(anchor) = run_anchor_id(&self.items, start) else {
            return false;
        };
        if !self.verb_group_expanded.remove(&anchor) {
            self.verb_group_expanded.insert(anchor);
        }
        // Structure of visible rows changes (members appear/disappear).
        self.bump_structure();
        true
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
                // Role bookkeeping only — no item list change yet.
                self.bump_content();
            }
            UiEventMessage::TextDelta { message_id, text } => {
                let text = text.render_preview();
                if text.is_empty() {
                    return;
                }
                // Only stream into the tail text item for this message. If tools
                // or reasoning already sit after an earlier reply block (common
                // for Grok/Gemini ACP intermediate agent_message_chunk), open a
                // new assistant text item at the timeline end instead of
                // concatenating into the earlier bubble.
                if self.tail_text_item_matches(&message_id) {
                    if let Some(item) = self.items.last_mut() {
                        append_text_to_item(item, text);
                        item.set_streaming(true);
                    }
                    self.bump_content();
                } else {
                    self.finish_open_content_streaming();
                    self.push_text_item(message_id, text, true);
                    self.bump_structure();
                }
            }
            UiEventMessage::TextReplace { message_id, text } => {
                let text = text.render_preview();
                let base_id = message_id
                    .split('\u{1e}')
                    .next()
                    .unwrap_or(message_id.as_str());
                let is_streaming = self.open_message_ids.contains(base_id)
                    || self.open_message_ids.contains(&message_id);
                // Tail-only replace/update. Non-tail same-body snapshot (OpenCode
                // finished part after tools) is ignored to freeze mid-timeline.
                // Different body → new bubble at end (part segments).
                if self.tail_text_item_matches(&message_id) {
                    if let Some(item) = self.items.last_mut() {
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
                        item.set_streaming(is_streaming);
                        self.bump_content();
                    }
                } else if let Some(item) = self.find_text_item_mut(&message_id) {
                    let existing = match item {
                        ChatItem::UserMessage { text_parts, .. }
                        | ChatItem::AssistantText { text_parts, .. } => text_parts
                            .iter()
                            .map(|p| match p {
                                TextPart::Plain(s) => s.as_str(),
                                _ => "",
                            })
                            .collect::<String>(),
                        _ => String::new(),
                    };
                    if existing == text {
                        // Finished-part snapshot equal to frozen row — drop.
                        return;
                    }
                    if !text.is_empty() {
                        self.finish_open_content_streaming();
                        self.push_text_item(message_id, text, is_streaming);
                        self.bump_structure();
                    }
                } else if !text.is_empty() {
                    self.finish_open_content_streaming();
                    self.push_text_item(message_id, text, is_streaming);
                    self.bump_structure();
                }
            }
            UiEventMessage::ReasoningDelta { message_id, text } => {
                let text = text.render_preview();
                if text.is_empty() {
                    return;
                }
                // Only stream into the tail reasoning item. If tools/text have
                // already been appended after an earlier thought block, open a
                // new reasoning item at the end of the timeline instead of
                // rewriting content above those items.
                let append_to_tail = matches!(
                    self.items.last(),
                    Some(ChatItem::Reasoning {
                        message_id: existing_id,
                        ..
                    }) if existing_id == &message_id
                );
                if append_to_tail {
                    if let Some(ChatItem::Reasoning {
                        text: existing,
                        is_streaming,
                        ..
                    }) = self.items.last_mut()
                    {
                        existing.push_str(&text);
                        *is_streaming = true;
                    }
                    self.bump_content();
                } else {
                    self.finish_open_content_streaming();
                    self.items.push(ChatItem::Reasoning {
                        message_id,
                        text,
                        is_streaming: true,
                        is_user_toggled: None,
                    });
                    self.bump_structure();
                }
            }
            UiEventMessage::ReasoningReplace { message_id, text } => {
                let text = text.render_preview();
                if text.is_empty() {
                    let before = self.items.len();
                    self.items.retain(|item| {
                        !matches!(
                            item,
                            ChatItem::Reasoning {
                                message_id: item_message_id,
                                ..
                            } if item_message_id == &message_id
                        )
                    });
                    if self.items.len() != before {
                        self.bump_structure();
                    }
                } else {
                    let replace_tail = matches!(
                        self.items.last(),
                        Some(ChatItem::Reasoning {
                            message_id: existing_id,
                            ..
                        }) if existing_id == &message_id
                    );
                    if replace_tail {
                        if let Some(ChatItem::Reasoning {
                            text: existing,
                            is_streaming,
                            ..
                        }) = self.items.last_mut()
                        {
                            *existing = text;
                            *is_streaming = self.open_message_ids.contains(&message_id);
                        }
                        self.bump_content();
                    } else {
                        self.finish_open_content_streaming();
                        let is_streaming = self.open_message_ids.contains(&message_id);
                        self.items.push(ChatItem::Reasoning {
                            message_id,
                            text,
                            is_streaming,
                            is_user_toggled: None,
                        });
                        self.bump_structure();
                    }
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
                    self.bump_content();
                } else {
                    // Starting a tool ends the previous text/thought stream so
                    // intermediate ACP bubbles no longer show a live cursor.
                    self.finish_open_content_streaming();
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
                        is_user_toggled: None,
                        is_streaming: true,
                    });
                    self.bump_structure();
                }
            }
            UiEventMessage::ToolCallCompleted {
                tool_call_id,
                output,
                is_error,
            } => {
                let output = output.render_preview();
                let mut changed = false;
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
                    changed = true;
                }
                // Opencode (and replay of older projections) may only emit ToolCallCompleted
                // for the parent `task` tool without a separate SubagentStatusUpdated event.
                if let Some(ChatItem::SubagentCall {
                    status,
                    is_streaming,
                    ..
                }) = self.find_subagent_call_by_tool_id_mut(&tool_call_id)
                {
                    *status = if is_error {
                        SubagentStatus::Failed
                    } else {
                        SubagentStatus::Completed
                    };
                    *is_streaming = false;
                    changed = true;
                }
                if changed {
                    self.bump_content();
                }
            }
            UiEventMessage::SubagentSpawned {
                sub_session_id,
                tool_call_id,
                agent,
                model,
                prompt,
                ..
            } => {
                if let Some(ChatItem::SubagentCall {
                    model: existing_model,
                    prompt_summary,
                    status,
                    is_streaming,
                    ..
                }) = self.find_subagent_call_mut(&sub_session_id)
                {
                    *existing_model = model;
                    *prompt_summary = prompt.as_deref().map(subagent_prompt_summary);
                    *status = SubagentStatus::Running;
                    *is_streaming = true;
                    self.bump_content();
                } else {
                    self.items.push(ChatItem::SubagentCall {
                        message_id: tool_call_id.clone(),
                        tool_call_id,
                        sub_session_id,
                        agent,
                        model,
                        prompt_summary: prompt.as_deref().map(subagent_prompt_summary),
                        status: SubagentStatus::Running,
                        is_streaming: true,
                    });
                    self.bump_structure();
                }
            }
            UiEventMessage::SubagentStatusUpdated {
                sub_session_id,
                status,
            } => {
                if let Some(ChatItem::SubagentCall {
                    status: existing_status,
                    is_streaming,
                    ..
                }) = self.find_subagent_call_mut(&sub_session_id)
                {
                    *existing_status = status;
                    *is_streaming = status == SubagentStatus::Running;
                    self.bump_content();
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
                self.bump_content();
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
                self.bump_structure();
            }
            UiEventMessage::Raw { kind, payload_json } => {
                if self.apply_raw_request_event(&kind, &payload_json) {
                    // Pending request list is visual chrome for AgentDetail.
                    self.bump_content();
                    return;
                }
                debug!(
                    raw_kind = %kind,
                    payload_bytes = payload_json.len(),
                    "raw ui event suppressed from chat"
                );
            }
            UiEventMessage::SessionOpened { .. } | UiEventMessage::SessionTitleUpdated { .. } => {}
            UiEventMessage::SessionClosed { reason, .. } => {
                self.finish_all_streaming();
                self.items.push(ChatItem::SystemMessage {
                    text: format!("Thread closed: {reason:?}"),
                });
                self.bump_structure();
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

    /// True when the timeline tail is a text bubble for `message_id` that can
    /// still receive contiguous deltas.
    fn tail_text_item_matches(&self, message_id: &str) -> bool {
        matches!(
            self.items.last(),
            Some(ChatItem::UserMessage {
                message_id: existing_id,
                ..
            } | ChatItem::AssistantText {
                message_id: existing_id,
                ..
            }) if existing_id == message_id
        )
    }

    /// Last user/assistant text item for `message_id` (may sit above tools).
    fn find_text_item_mut(&mut self, message_id: &str) -> Option<&mut ChatItem> {
        self.items.iter_mut().rev().find(|item| match item {
            ChatItem::UserMessage {
                message_id: existing_id,
                ..
            }
            | ChatItem::AssistantText {
                message_id: existing_id,
                ..
            } => existing_id == message_id,
            _ => false,
        })
    }

    /// Clear streaming flags on open text/reasoning rows that are no longer
    /// the active stream (tools, a later thought block, or a later reply).
    fn finish_open_content_streaming(&mut self) {
        let mut changed = false;
        for item in &mut self.items {
            match item {
                ChatItem::UserMessage { is_streaming, .. }
                | ChatItem::AssistantText { is_streaming, .. }
                | ChatItem::Reasoning { is_streaming, .. } => {
                    if *is_streaming {
                        *is_streaming = false;
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        if changed {
            self.bump_content();
        }
    }

    fn find_tool_call_item_mut(&mut self, tool_call_id: &str) -> Option<&mut ChatItem> {
        self.items.iter_mut().rev().find(|item| {
            matches!(item, ChatItem::ToolCall { tool_call_id: id, .. } if id == tool_call_id)
        })
    }

    fn find_subagent_call_mut(&mut self, sub_session_id: &str) -> Option<&mut ChatItem> {
        self.items.iter_mut().rev().find(|item| {
            matches!(item, ChatItem::SubagentCall { sub_session_id: id, .. } if id == sub_session_id)
        })
    }

    fn find_subagent_call_by_tool_id_mut(&mut self, tool_call_id: &str) -> Option<&mut ChatItem> {
        self.items.iter_mut().rev().find(|item| {
            matches!(item, ChatItem::SubagentCall { tool_call_id: id, .. } if id == tool_call_id)
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

#[allow(dead_code)] // helper for last_completed_assistant_text
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

#[allow(dead_code)] // helper for last_completed_assistant_text
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

fn subagent_prompt_summary(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or("").trim();
    let mut summary = first_line.chars().take(120).collect::<String>();
    if first_line.chars().count() > 120 {
        summary.push_str("...");
    }
    summary
}
