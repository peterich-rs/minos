use minos_domain::AgentName;
use minos_ui_protocol::{
    ClaudeTranslatorState, CodexTranslatorState, GeminiTranslatorState, OpencodeTranslatorState,
    UiEventMessage, MessageRole,
};

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

    pub fn translate(
        &mut self,
        payload: &serde_json::Value,
    ) -> Vec<UiEventMessage> {
        match self {
            Self::Codex(s) => minos_ui_protocol::translate_codex(s, payload)
                .unwrap_or_default(),
            Self::Claude(s) => minos_ui_protocol::translate_claude(s, payload)
                .unwrap_or_default(),
            Self::Gemini(s) => minos_ui_protocol::translate_gemini(s, payload)
                .unwrap_or_default(),
            Self::Opencode(s) => minos_ui_protocol::translate_opencode(s, payload)
                .unwrap_or_default(),
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
        }
    }

    pub fn apply_ui_events(&mut self, events: Vec<UiEventMessage>) {
        for event in events {
            self.apply_ui_event(event);
        }
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
                    msg.tool_calls.push(ToolCallBlock {
                        tool_call_id,
                        name,
                        args_summary: truncate_str(&args_json, 120),
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
                if let Some(msg) = message_id.and_then(|mid| {
                    self.messages.iter_mut().rev().find(|m| m.message_id == mid)
                }) {
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
            UiEventMessage::Raw {
                kind,
                payload_json,
            } => {
                self.messages.push(RenderedMessage {
                    message_id: String::new(),
                    role: MessageRole::System,
                    text_parts: vec![TextPart::Plain(format!("[raw:{kind}] {payload_json}"))],
                    tool_calls: Vec::new(),
                    reasoning: None,
                    is_streaming: false,
                    error: None,
                });
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

pub struct RenderedMessage {
    pub message_id: String,
    pub role: MessageRole,
    pub text_parts: Vec<TextPart>,
    pub tool_calls: Vec<ToolCallBlock>,
    pub reasoning: Option<String>,
    pub is_streaming: bool,
    pub error: Option<String>,
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
}
