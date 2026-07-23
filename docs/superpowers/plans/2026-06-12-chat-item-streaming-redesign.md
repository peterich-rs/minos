# ChatItem Streaming Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the monolithic `RenderedMessage` with a flat `Vec<ChatItem>` where each variant (text, reasoning, tool call) is an independent list item that only appears when it has content.

**Architecture:** Replace `RenderedMessage` with a `ChatItem` enum. `ChatState.messages` becomes `ChatState.items: Vec<ChatItem>`. `MessageStarted` no longer pushes a placeholder — items are created on first content. The `minos-ui-protocol` crate is unchanged; all mapping happens in `ChatState::apply_ui_event`.

**Tech Stack:** Rust, ratatui, existing minos crates.

---

### Task 1: Define the ChatItem enum

**Files:**
- Modify: `minos-tui/src/translation.rs`

- [ ] **Step 1: Add `ChatItem` enum and keep `RenderedMessage` temporarily**

Add the `ChatItem` enum above the `ChatState` impl block. Keep `RenderedMessage` for now so existing code still compiles — it will be removed in Task 5.

```rust
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
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build -p minos-tui 2>&1 | tail -5`
Expected: Compiled successfully (enum is defined but unused, no errors).

- [ ] **Step 3: Commit**

```bash
git add crates/minos-tui/src/translation.rs
git commit -m "feat(tui): define ChatItem enum for streaming redesign"
```

---

### Task 2: Add `items` field to ChatState and new `apply_item_event` method

**Files:**
- Modify: `minos-tui/src/translation.rs`

This task adds the new `items: Vec<ChatItem>` field and a new `apply_item_event` method alongside the existing `apply_ui_event`. The old method remains so all existing tests keep passing during the transition.

- [ ] **Step 1: Add `items` field and `open_message_ids` tracking to `ChatState`**

Add two new fields to `ChatState`:

```rust
pub struct ChatState {
    pub session_id: String,
    pub agent: AgentName,
    pub translation_state: AgentTranslationState,
    pub messages: Vec<RenderedMessage>,  // kept temporarily
    pub items: Vec<ChatItem>,            // NEW
    pub pending_requests: Vec<PendingAgentRequest>,
    open_message_ids: std::collections::HashSet<String>, // NEW
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
    pub selection: Option<ChatSelection>,
}
```

Initialize them in `ChatState::new`:

```rust
impl ChatState {
    pub fn new(session_id: String, agent: AgentName) -> Self {
        Self {
            translation_state: AgentTranslationState::new(agent, session_id.clone()),
            session_id,
            agent,
            messages: Vec::new(),
            items: Vec::new(),
            pending_requests: Vec::new(),
            open_message_ids: std::collections::HashSet::new(),
            scroll_offset: 0,
            auto_scroll: true,
            max_scroll: 0,
            selection: None,
        }
    }
```

- [ ] **Step 2: Write failing tests for `apply_item_event`**

Add a new test module `mod item_tests` inside the existing `#[cfg(test)] mod tests` block in `translation.rs`:

```rust
mod item_tests {
    use super::*;
    use minos_domain::AgentName;
    use minos_ui_protocol::{MessageRole, UiEventMessage};

    fn apply(cs: &mut ChatState, event: UiEventMessage) {
        cs.apply_item_event(event);
    }

    fn apply_all(cs: &mut ChatState, events: Vec<UiEventMessage>) {
        for event in events {
            cs.apply_item_event(event);
        }
    }

    #[test]
    fn message_started_does_not_create_visible_item() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply(&mut cs, UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 0,
        });
        assert!(cs.items.is_empty());
        assert!(cs.open_message_ids.contains("m1"));
    }

    #[test]
    fn text_delta_creates_assistant_text_item() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "hello ".into(),
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "world".into(),
            },
        ]);
        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::AssistantText { text_parts, is_streaming, .. } => {
                assert_eq!(*text_parts, vec![TextPart::Plain("hello world".into())]);
                assert!(*is_streaming);
            }
            other => panic!("expected AssistantText, got {:?}", other_variant_name(other)),
        }
        assert!(!cs.open_message_ids.contains("m1"));
    }

    #[test]
    fn user_message_creates_user_item_on_text_delta() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "u1".into(),
                role: MessageRole::User,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "u1".into(),
                text: "my question".into(),
            },
        ]);
        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::UserMessage { text_parts, .. } => {
                assert_eq!(*text_parts, vec![TextPart::Plain("my question".into())]);
            }
            other => panic!("expected UserMessage, got {:?}", other_variant_name(other)),
        }
    }

    #[test]
    fn reasoning_delta_creates_reasoning_item() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ReasoningDelta {
                message_id: "m1".into(),
                text: "thinking...".into(),
            },
        ]);
        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::Reasoning { text, is_streaming, .. } => {
                assert_eq!(text, "thinking...");
                assert!(*is_streaming);
            }
            other => panic!("expected Reasoning, got {:?}", other_variant_name(other)),
        }
    }

    #[test]
    fn tool_call_placed_creates_tool_call_item() {
        let mut cs = ChatState::new("t1".into(), AgentName::Claude);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "read_file".into(),
                args_json: r#"{"path":"src/main.rs"}"#.into(),
            },
        ]);
        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::ToolCall { name, tool_call_id, is_streaming, .. } => {
                assert_eq!(name, "read_file");
                assert_eq!(tool_call_id, "tc1");
                assert!(*is_streaming);
            }
            other => panic!("expected ToolCall, got {:?}", other_variant_name(other)),
        }
    }

    #[test]
    fn items_appear_in_arrival_order() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
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
            UiEventMessage::ToolCallCompleted {
                tool_call_id: "tc1".into(),
                output: "ok".into(),
                is_error: false,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "the answer is 42".into(),
            },
            UiEventMessage::MessageCompleted {
                message_id: "m1".into(),
                finished_at_ms: 1,
            },
        ]);
        assert_eq!(cs.items.len(), 3);
        assert!(matches!(cs.items[0], ChatItem::Reasoning { .. }));
        assert!(matches!(cs.items[1], ChatItem::ToolCall { .. }));
        assert!(matches!(cs.items[2], ChatItem::AssistantText { .. }));
        for item in &cs.items {
            match item {
                ChatItem::Reasoning { is_streaming, .. }
                | ChatItem::ToolCall { is_streaming, .. }
                | ChatItem::AssistantText { is_streaming, .. } => {
                    assert!(!*is_streaming);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn message_completed_stops_streaming_on_all_matching_items() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ReasoningDelta {
                message_id: "m1".into(),
                text: "hmm".into(),
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
        assert_eq!(cs.items.len(), 2);
        for item in &cs.items {
            match item {
                ChatItem::Reasoning { is_streaming, .. }
                | ChatItem::AssistantText { is_streaming, .. } => {
                    assert!(!*is_streaming);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn tool_call_completed_updates_existing_item() {
        let mut cs = ChatState::new("t1".into(), AgentName::Claude);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "write_file".into(),
                args_json: r#"{"path":"foo.rs"}"#.into(),
            },
            UiEventMessage::ToolCallCompleted {
                tool_call_id: "tc1".into(),
                output: "done".into(),
                is_error: false,
            },
        ]);
        match &cs.items[0] {
            ChatItem::ToolCall { output_summary, is_error, is_streaming, .. } => {
                assert_eq!(output_summary.as_deref(), Some("done"));
                assert!(!*is_error);
                assert!(!*is_streaming);
            }
            other => panic!("expected ToolCall, got {:?}", other_variant_name(other)),
        }
    }

    #[test]
    fn error_pushes_error_item() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::Error {
                code: "fail".into(),
                message: "something broke".into(),
                message_id: Some("m1".into()),
            },
        ]);
        assert_eq!(cs.items.len(), 1);
        match &cs.items[0] {
            ChatItem::Error { message_id, text } => {
                assert_eq!(message_id.as_deref(), Some("m1"));
                assert_eq!(text, "something broke");
            }
            other => panic!("expected Error, got {:?}", other_variant_name(other)),
        }
    }

    #[test]
    fn thread_closed_pushes_system_message_and_finishes_streaming() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "partial".into(),
            },
            UiEventMessage::SessionClosed {
                session_id: "t1".into(),
                reason: minos_ui_protocol::ThreadEndReason::UserStopped,
                closed_at_ms: 1,
            },
        ]);
        assert_eq!(cs.items.len(), 2);
        match &cs.items[0] {
            ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
            other => panic!("expected AssistantText, got {:?}", other_variant_name(other)),
        }
        match &cs.items[1] {
            ChatItem::SystemMessage { text } => {
                assert!(text.contains("Thread closed"));
            }
            other => panic!("expected SystemMessage, got {:?}", other_variant_name(other)),
        }
    }

    #[test]
    fn finish_all_streaming_marks_everything_done() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 0,
            },
            UiEventMessage::ReasoningDelta {
                message_id: "m1".into(),
                text: "hmm".into(),
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "ans".into(),
            },
        ]);
        cs.finish_all_streaming();
        for item in &cs.items {
            match item {
                ChatItem::Reasoning { is_streaming, .. }
                | ChatItem::AssistantText { is_streaming, .. }
                | ChatItem::UserMessage { is_streaming, .. }
                | ChatItem::ToolCall { is_streaming, .. } => {
                    assert!(!*is_streaming);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn last_completed_assistant_text_finds_last_non_streaming() {
        let mut cs = ChatState::new("t1".into(), AgentName::Codex);
        apply_all(&mut cs, vec![
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
        let result = cs.last_completed_assistant_text_from_items();
        assert_eq!(result, Some(("m1".into(), "first".into())));
    }

    fn other_variant_name(item: &ChatItem) -> &'static str {
        match item {
            ChatItem::UserMessage { .. } => "UserMessage",
            ChatItem::AssistantText { .. } => "AssistantText",
            ChatItem::Reasoning { .. } => "Reasoning",
            ChatItem::ToolCall { .. } => "ToolCall",
            ChatItem::SystemMessage { .. } => "SystemMessage",
            ChatItem::Error { .. } => "Error",
        }
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p minos-tui item_tests -- 2>&1 | tail -10`
Expected: Compilation errors — `apply_item_event`, `finish_all_streaming`, `last_completed_assistant_text_from_items` do not exist yet.

- [ ] **Step 4: Implement `apply_item_event` and new helpers**

Add these methods to `impl ChatState`:

```rust
pub fn apply_item_event(&mut self, event: UiEventMessage) {
    match event {
        UiEventMessage::MessageStarted { message_id, role, .. } => {
            if matches!(role, MessageRole::Assistant) {
                self.finish_all_streaming();
            }
            self.open_message_ids.insert(message_id);
        }
        UiEventMessage::TextDelta { message_id, text } => {
            if let Some(item) = self.find_text_item_mut(&message_id) {
                append_text_to_item(item, text);
            } else if self.open_message_ids.contains(&message_id) {
                let role = self.infer_role(&message_id);
                let item = match role {
                    MessageRole::User => ChatItem::UserMessage {
                        message_id,
                        text_parts: vec![TextPart::Plain(text)],
                        is_streaming: true,
                    },
                    _ => ChatItem::AssistantText {
                        message_id,
                        text_parts: vec![TextPart::Plain(text)],
                        is_streaming: true,
                    },
                };
                self.open_message_ids.remove(item.message_id());
                self.items.push(item);
            }
        }
        UiEventMessage::TextReplace { message_id, text } => {
            if let Some(item) = self.find_text_item_mut(&message_id) {
                match item {
                    ChatItem::UserMessage { text_parts, .. }
                    | ChatItem::AssistantText { text_parts, .. } => {
                        *text_parts = if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![TextPart::Plain(text)]
                        };
                    }
                    _ => {}
                }
            }
        }
        UiEventMessage::ReasoningDelta { message_id, text } => {
            if let Some(ChatItem::Reasoning { text: existing, .. }) =
                self.find_reasoning_item_mut(&message_id)
            {
                existing.push_str(&text);
            } else if self.open_message_ids.contains(&message_id) {
                self.items.push(ChatItem::Reasoning {
                    message_id,
                    text,
                    is_streaming: true,
                });
            }
        }
        UiEventMessage::ReasoningReplace { message_id, text } => {
            if let Some(item) = self.find_reasoning_item_mut(&message_id) {
                if text.is_empty() {
                    *item = ChatItem::SystemMessage { text: String::new() };
                } else {
                    *item = ChatItem::Reasoning {
                        message_id,
                        text,
                        is_streaming: false,
                    };
                }
            }
        }
        UiEventMessage::ToolCallPlaced {
            message_id,
            tool_call_id,
            name,
            args_json,
        } => {
            if let Some(ChatItem::ToolCall {
                name: ref mut existing_name,
                args_summary: ref mut existing_summary,
                args_detail: ref mut existing_detail,
                is_expanded: ref mut existing_expanded,
                ..
            }) = self.find_tool_call_item_mut(&tool_call_id)
            {
                let args_summary = summarize_tool_args(&name, &args_json);
                let args_detail = compact_tool_args(&args_json)
                    .filter(|detail| !detail.is_empty() && detail != &args_summary);
                *existing_name = name;
                *existing_summary = args_summary;
                *existing_detail = args_detail;
                *existing_expanded |= is_diff_like(&args_json);
            } else {
                let args_summary = summarize_tool_args(&name, &args_json);
                let args_detail = compact_tool_args(&args_json)
                    .filter(|detail| !detail.is_empty() && detail != &args_summary);
                self.items.push(ChatItem::ToolCall {
                    message_id,
                    tool_call_id,
                    name,
                    args_summary,
                    args_detail,
                    output_summary: None,
                    output_detail: None,
                    is_error: false,
                    is_expanded: is_diff_like(&args_json),
                    is_streaming: true,
                });
            }
        }
        UiEventMessage::ToolCallCompleted {
            tool_call_id,
            output,
            is_error,
        } => {
            if let Some(item) = self.find_tool_call_item_mut(&tool_call_id) {
                match item {
                    ChatItem::ToolCall {
                        output_summary,
                        output_detail,
                        is_error: err,
                        is_expanded,
                        is_streaming,
                        ..
                    } => {
                        *output_summary = Some(summarize_tool_output(&output));
                        *output_detail = tool_output_detail(&output);
                        if is_diff_like(&output) {
                            *is_expanded = true;
                        }
                        *err = is_error;
                        *is_streaming = false;
                    }
                    _ => unreachable!(),
                }
            }
        }
        UiEventMessage::MessageCompleted { message_id, .. } => {
            self.open_message_ids.remove(&message_id);
            for item in &mut self.items {
                if item.message_id() == Some(&message_id) {
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
            if self.apply_raw_request_event_item(&kind, &payload_json) {
                return;
            }
        }
        UiEventMessage::SessionOpened { .. } | UiEventMessage::SessionTitleUpdated { .. } => {}
        UiEventMessage::SessionClosed { reason, .. } => {
            self.finish_all_streaming();
            self.items.push(ChatItem::SystemMessage {
                text: format!("Thread closed: {reason:?}"),
            });
        }
    }
}

pub fn finish_all_streaming(&mut self) {
    for item in &mut self.items {
        item.set_streaming(false);
    }
}

pub fn last_completed_assistant_text_from_items(&self) -> Option<(String, String)> {
    self.items
        .iter()
        .rev()
        .find_map(|item| match item {
            ChatItem::AssistantText {
                message_id,
                text_parts,
                is_streaming: false,
            } => {
                let text = text_parts_to_string(text_parts)?;
                Some((message_id.clone(), text))
            }
            _ => None,
        })

}
```

Add the helper methods and trait-like methods on `ChatItem`:

```rust
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
```

Add private helper methods on `ChatState`:

```rust
impl ChatState {
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
            matches!(item, ChatItem::ToolCall { tool_call_id: tc_id, .. } if tc_id == tool_call_id)
        })
    }

    fn infer_role(&self, message_id: &str) -> MessageRole {
        MessageRole::Assistant
    }
}
```

Note: `infer_role` returns `Assistant` as default. The `MessageStarted` event carries the role but we don't store it in `open_message_ids`. We need a small secondary tracking map. Add to `ChatState`:

```rust
open_message_roles: std::collections::HashMap<String, MessageRole>,
```

Update `MessageStarted` handling in `apply_item_event`:

```rust
UiEventMessage::MessageStarted { message_id, role, .. } => {
    if matches!(role, MessageRole::Assistant) {
        self.finish_all_streaming();
    }
    self.open_message_ids.insert(message_id.clone());
    self.open_message_roles.insert(message_id, role);
}
```

Update `infer_role`:

```rust
fn infer_role(&self, message_id: &str) -> MessageRole {
    self.open_message_roles
        .get(message_id)
        .copied()
        .unwrap_or(MessageRole::Assistant)
}
```

Update `MessageCompleted` to also clean up `open_message_roles`:

```rust
UiEventMessage::MessageCompleted { message_id, .. } => {
    self.open_message_ids.remove(&message_id);
    self.open_message_roles.remove(&message_id);
    for item in &mut self.items {
        if item.message_id() == Some(&message_id) {
            item.set_streaming(false);
        }
    }
}
```

Add `apply_raw_request_event_item` method (mirrors the existing `apply_raw_request_event` but pushes `ChatItem::SystemMessage` instead of `RenderedMessage`):

```rust
fn apply_raw_request_event_item(&mut self, kind: &str, payload_json: &str) -> bool {
    match kind {
        "approval/request" => {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) else {
                return false;
            };
            let Some(request) = PendingAgentRequest::from_approval_request(&value) else {
                return false;
            };
            self.push_pending_request_item(request);
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
            self.push_pending_request_item(request);
            true
        }
        _ => false,
    }
}

fn push_pending_request_item(&mut self, request: PendingAgentRequest) {
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
```

Also add the free function used by `last_completed_assistant_text_from_items`:

```rust
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
    if result.is_empty() {
        None
    } else {
        Some(result.join("\n"))
    }
}
```

Also add the `append_text_to_item` helper:

```rust
fn append_text_to_item(item: &mut ChatItem, text: String) {
    match item {
        ChatItem::UserMessage { text_parts, .. }
        | ChatItem::AssistantText { text_parts, .. } => {
            if let Some(TextPart::Plain(last)) = text_parts.last_mut() {
                last.push_str(&text);
            } else {
                text_parts.push(TextPart::Plain(text));
            }
        }
        _ => {}
    }
}
```

Also need the `MessageRole` import in the `ChatState` helper scope — it's already imported at the top of the file via `minos_ui_protocol::MessageRole`.

Don't forget to add `use std::collections::{HashMap, HashSet};` if not already imported at the top of `translation.rs`.

- [ ] **Step 5: Run the item_tests to verify they pass**

Run: `cargo test -p minos-tui item_tests -- 2>&1 | tail -20`
Expected: All `item_tests` pass.

- [ ] **Step 6: Run all existing tests to verify nothing is broken**

Run: `cargo test -p minos-tui -- 2>&1 | tail -20`
Expected: All tests pass (old `tests` module still uses `messages`/`apply_ui_event`).

- [ ] **Step 7: Commit**

```bash
git add crates/minos-tui/src/translation.rs
git commit -m "feat(tui): add ChatItem-based apply_item_event with tests"
```

---

### Task 3: Adapt chat.rs rendering to ChatItem

**Files:**
- Modify: `minos-tui/src/ui/chat.rs`

This task adds a new `build_item_lines` function alongside the existing `build_lines`, so both paths work during the transition.

- [ ] **Step 1: Write the `build_item_lines` function**

Add to `chat.rs`:

```rust
pub fn build_item_lines(items: &[crate::translation::ChatItem], separator_width: u16) -> Vec<Line<'static>> {
    use crate::translation::{ChatItem, TextPart};

    let mut lines: Vec<Line<'static>> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(Span::styled(
                "─".repeat(usize::from(separator_width.max(1))),
                ratatui::style::Style::new().fg(BORDER_FG),
            )));
        }

        match item {
            ChatItem::UserMessage { text_parts, is_streaming, .. } => {
                lines.push(Line::from(Span::styled("[You]", USER_LABEL)));
                for part in text_parts {
                    match part {
                        TextPart::Plain(text) => {
                            push_markdown_lines(&mut lines, text, Style::default());
                        }
                        TextPart::Code { lang, code } => {
                            push_code_block(&mut lines, lang, code);
                        }
                    }
                }
                if *is_streaming {
                    lines.push(Line::from(Span::styled("▓", STREAMING_CURSOR)));
                }
            }
            ChatItem::AssistantText { text_parts, is_streaming, .. } => {
                lines.push(Line::from(Span::styled("[Agent]", ASSISTANT_LABEL)));
                for part in text_parts {
                    match part {
                        TextPart::Plain(text) => {
                            push_markdown_lines(&mut lines, text, Style::default());
                        }
                        TextPart::Code { lang, code } => {
                            push_code_block(&mut lines, lang, code);
                        }
                    }
                }
                if *is_streaming {
                    lines.push(Line::from(Span::styled("▓", STREAMING_CURSOR)));
                }
            }
            ChatItem::Reasoning { text, is_streaming, .. } => {
                lines.push(Line::from(Span::styled("Thinking", REASONING_STYLE)));
                push_markdown_lines(&mut lines, text, REASONING_STYLE);
                if *is_streaming {
                    lines.push(Line::from(Span::styled("▓", STREAMING_CURSOR)));
                }
            }
            ChatItem::ToolCall {
                name,
                args_summary,
                args_detail,
                output_summary,
                output_detail,
                is_error,
                is_expanded,
                is_streaming,
                ..
            } => {
                let status_label = if is_streaming {
                    Span::styled("running", ratatui::style::Style::default())
                } else if output_summary.is_none() {
                    Span::styled("running", ratatui::style::Style::default())
                } else if *is_error {
                    Span::styled("failed", TOOL_ERROR)
                } else {
                    Span::styled("done", TOOL_SUCCESS)
                };
                let mut tc_spans = vec![
                    Span::raw("Tool "),
                    Span::styled(name.clone(), TOOL_NAME_STYLE),
                    Span::raw(" · "),
                    status_label,
                ];
                if !args_summary.is_empty() {
                    tc_spans.push(Span::raw(format!(" {}", args_summary)));
                }
                if *is_expanded {
                    let mut emitted_detail = false;
                    lines.push(Line::from(tc_spans.clone()));
                    if let Some(args) = args_detail {
                        emitted_detail = true;
                        push_tool_detail_lines(&mut lines, "args", args);
                    }
                    if let Some(output) = output_detail.as_ref().or(output_summary.as_ref()) {
                        emitted_detail = true;
                        push_tool_detail_lines(&mut lines, "out", output);
                    }
                    if emitted_detail {
                        continue;
                    }
                }
                lines.push(Line::from(tc_spans));
            }
            ChatItem::SystemMessage { text } => {
                lines.push(Line::from(Span::styled("[System]", REASONING_STYLE)));
                push_markdown_lines(&mut lines, text, Style::default());
            }
            ChatItem::Error { text, .. } => {
                lines.push(Line::from(Span::styled(text.clone(), ERROR_STYLE)));
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        )));
    }

    lines
}
```

- [ ] **Step 2: Add tests for `build_item_lines`**

Add to the test module in `chat.rs`:

```rust
#[test]
fn item_lines_render_user_message() {
    let items = vec![crate::translation::ChatItem::UserMessage {
        message_id: "m1".into(),
        text_parts: vec![super::super::translation::TextPart::Plain("hello".into())],
        is_streaming: false,
    }];
    let lines = super::build_item_lines(&items, 80);
    let rendered: Vec<String> = lines.iter().map(line_text).collect();
    assert!(rendered.iter().any(|l| l == "[You]"));
    assert!(rendered.iter().any(|l| l == "hello"));
}

#[test]
fn item_lines_render_reasoning_then_text() {
    let items = vec![
        crate::translation::ChatItem::Reasoning {
            message_id: "m1".into(),
            text: "let me think".into(),
            is_streaming: false,
        },
        crate::translation::ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![super::super::translation::TextPart::Plain("answer".into())],
            is_streaming: false,
        },
    ];
    let lines = super::build_item_lines(&items, 80);
    let rendered: Vec<String> = lines.iter().map(line_text).collect();
    assert!(rendered.iter().any(|l| l == "Thinking"));
    assert!(rendered.iter().any(|l| l == "[Agent]"));
    assert!(rendered.iter().any(|l| l == "answer"));
}

#[test]
fn item_lines_render_tool_call() {
    let items = vec![crate::translation::ChatItem::ToolCall {
        message_id: "m1".into(),
        tool_call_id: "tc1".into(),
        name: "read_file".into(),
        args_summary: "file=src/main.rs".into(),
        args_detail: None,
        output_summary: Some("ok".into()),
        output_detail: None,
        is_error: false,
        is_expanded: false,
        is_streaming: false,
    }];
    let lines = super::build_item_lines(&items, 80);
    let rendered: Vec<String> = lines.iter().map(line_text).collect();
    assert!(rendered.iter().any(|l| l.contains("Tool read_file") && l.contains("done")));
}
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo test -p minos-tui -- 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/ui/chat.rs
git commit -m "feat(tui): add build_item_lines for ChatItem rendering"
```

---

### Task 4: Wire ChatState.items into render_chat and app.rs

**Files:**
- Modify: `minos-tui/src/ui/chat.rs`
- Modify: `minos-tui/src/translation.rs`
- Modify: `minos-tui/src/app.rs`

This task switches the render path and app.rs from `messages` to `items`.

- [ ] **Step 1: Update `render_chat` to use `items`**

In `chat.rs`, change `render_chat` to use `items`:

```rust
pub fn render_chat(f: &mut Frame, area: Rect, chat: &mut ChatState, focused: bool) {
    let title = format!(
        "Chat: {} #{}{}",
        chat.agent.bin_name(),
        short_session_id(&chat.session_id),
        if chat.auto_scroll {
            ""
        } else {
            " [manual scroll]"
        }
    );
    let block = super::theme::border_block()
        .title(title)
        .border_style(if focused {
            FOCUSED_BORDER
        } else {
            ratatui::style::Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);

    if inner.width == 0 || inner.height == 0 {
        f.render_widget(block, area);
        return;
    }

    let mut lines = visual_lines(
        build_item_lines(chat.items.as_slice(), inner.width),
        inner.width,
    );
    let max_scroll = lines
        .len()
        .saturating_sub(usize::from(inner.height))
        .min(usize::from(u16::MAX)) as u16;
    chat.update_max_scroll(max_scroll);
    apply_selection(lines.as_mut_slice(), chat.selection.as_ref());
    let visible_lines: Vec<Line<'static>> = lines
        .into_iter()
        .skip(usize::from(chat.active_scroll()))
        .take(usize::from(inner.height))
        .map(|line| line.line)
        .collect();

    let paragraph = Paragraph::new(visible_lines).block(block);
    f.render_widget(paragraph, area);
}
```

- [ ] **Step 2: Update `selected_text` to use `items`**

```rust
pub fn selected_text(chat: &ChatState, width: u16) -> Option<String> {
    let selection = chat.selection.as_ref()?;
    if selection.is_empty() || width == 0 {
        return None;
    }

    let lines = visual_lines(build_item_lines(chat.items.as_slice(), width), width);
    selected_text_from_lines(lines.as_slice(), selection)
}
```

- [ ] **Step 3: Update `app.rs` — switch `apply_ui_events` to `apply_item_events`**

In `app.rs`, find all call sites of `chat.apply_ui_events(events)` and change to `chat.apply_item_events(events)`.

Add to `ChatState`:

```rust
pub fn apply_item_events(&mut self, events: Vec<UiEventMessage>) {
    for event in events {
        self.apply_item_event(event);
    }
}
```

The call sites in `app.rs` that reference `chat.messages` or `chat.apply_ui_events`:
- `handle_event` → `Ingest` branch: `chat.apply_ui_events(events)` → `chat.apply_item_events(events)`
- `replay_thread_history`: `chat.apply_ui_events(events)` → `chat.apply_item_events(events)`

- [ ] **Step 4: Update `app.rs` — change `chat.messages` to `chat.items`**

Find and replace all remaining `chat.messages` references in `app.rs`:
- `toggle_tool_expansion`: iterate `chat.items` and match on `ChatItem::ToolCall { is_expanded, .. }`

```rust
fn toggle_tool_expansion(&mut self) -> bool {
    if let Some(chat) = self.ui.current_chat_mut() {
        for item in &mut chat.items {
            if let crate::translation::ChatItem::ToolCall { is_expanded, .. } = item {
                *is_expanded = !*is_expanded;
            }
        }
        return true;
    }
    false
}
```

- `handle_manager_event` → `SessionStateChanged`: `chat.finish_streaming_assistant_messages()` → `chat.finish_all_streaming()`
- `handle_manager_event` → `SessionClosed`: same
- `handle_manager_event` → `InstanceCrashed`: same

- [ ] **Step 5: Update `last_completed_assistant_text` references**

In `app.rs`, `record_agent_group_result_if_done` calls `chat.last_completed_assistant_text()`. Change to `chat.last_completed_assistant_text_from_items()`.

- [ ] **Step 6: Build and run all tests**

Run: `cargo test -p minos-tui -- 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-tui/src/translation.rs crates/minos-tui/src/ui/chat.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): wire ChatItem into render and app layers"
```

---

### Task 5: Remove old RenderedMessage code and clean up

**Files:**
- Modify: `minos-tui/src/translation.rs`
- Modify: `minos-tui/src/ui/chat.rs`

- [ ] **Step 1: Remove `RenderedMessage` struct and old `apply_ui_event` method**

Delete from `translation.rs`:
- The `RenderedMessage` struct
- The `RenderedMessage::system` impl
- The old `apply_ui_event` method
- The old `apply_raw_request_event` method
- The old `push_pending_request_message` method
- The old `finish_streaming_assistant_messages` method
- The old `last_completed_assistant_text` method
- Remove the `messages: Vec<RenderedMessage>` field from `ChatState`

Rename `apply_item_event` → `apply_ui_event`, `apply_item_events` → `apply_ui_events`, `apply_raw_request_event_item` → `apply_raw_request_event`, `push_pending_request_item` → `push_pending_request_message`, `finish_all_streaming` stays, `last_completed_assistant_text_from_items` → `last_completed_assistant_text`.

- [ ] **Step 2: Remove old `build_lines` function from `chat.rs`**

Delete the old `build_lines` function. Rename `build_item_lines` → `build_lines`.

Update `render_chat` and `selected_text` calls from `build_item_lines` to `build_lines`.

- [ ] **Step 3: Migrate old tests to new ChatItem model**

Update the existing `tests` module in `translation.rs`:
- Replace `RenderedMessage` assertions with `ChatItem` assertions
- Replace `cs.messages` with `cs.items`
- Replace `message(...)` helper with a `chat_item(...)` helper

Replace the old test helper:

```rust
fn user_item(id: &str, text: &str, streaming: bool) -> ChatItem {
    ChatItem::UserMessage {
        message_id: id.into(),
        text_parts: vec![TextPart::Plain(text.into())],
        is_streaming: streaming,
    }
}

fn assistant_item(id: &str, text: &str, streaming: bool) -> ChatItem {
    ChatItem::AssistantText {
        message_id: id.into(),
        text_parts: vec![TextPart::Plain(text.into())],
        is_streaming: streaming,
    }
}
```

Update each test:

- `chat_state_message_started_then_text_delta`: use `apply_ui_event` (renamed), assert on `items[0]` matching `AssistantText`
- `tool_call_placed_then_completed`: assert items has `ToolCall`
- `duplicate_tool_call_placed_updates_existing_tool_block`: assert single `ToolCall` in items
- `codex_raw_events_render_final_text_and_keep_tool_on_original_message`: update for items — text and tool call are separate items now
- `text_replace_uses_completed_agent_message_as_authoritative`: update for items
- `reasoning_replace_uses_completed_reasoning_as_authoritative`: update for items
- `new_assistant_message_finishes_previous_streaming_assistant`: update for items
- `error_finishes_targeted_streaming_assistant`: update for items
- `raw_events_do_not_render_large_payloads_into_chat`: update — `items` should be empty
- `opencode_permission_update_creates_pending_request`: update for items
- `opencode_permission_completion_clears_pending_request`: update for items

Delete the `item_tests` module since those tests are now in the main `tests` module.

- [ ] **Step 4: Update chat.rs tests**

Remove the `RenderedMessage`-based tests and keep the `ChatItem`-based tests. The test helper `message(...)` is replaced by the new item helpers.

- [ ] **Step 5: Build and run all tests**

Run: `cargo test -p minos-tui -- 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-tui/src/translation.rs crates/minos-tui/src/ui/chat.rs
git commit -m "refactor(tui): remove RenderedMessage, clean up old code"
```

---

### Task 6: Full verification

- [ ] **Step 1: Run all TUI tests**

Run: `cargo test -p minos-tui -- 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 2: Run all UI protocol tests**

Run: `cargo test -p minos-ui-protocol -- 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 3: Run workspace check**

Run: `cargo build -p minos-tui 2>&1 | tail -5`
Expected: Compiled successfully.

- [ ] **Step 4: Final commit if any fixes needed**

```bash
git add -u
git commit -m "fix(tui): address test/compilation issues from ChatItem migration"
```
