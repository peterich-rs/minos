#[derive(Debug, PartialEq, Eq, Hash)]
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

impl ChatItem {
    pub(super) fn message_id(&self) -> Option<&str> {
        match self {
            ChatItem::UserMessage { message_id, .. }
            | ChatItem::AssistantText { message_id, .. }
            | ChatItem::Reasoning { message_id, .. }
            | ChatItem::ToolCall { message_id, .. } => Some(message_id),
            ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => None,
        }
    }

    pub(super) fn set_streaming(&mut self, value: bool) {
        match self {
            ChatItem::UserMessage { is_streaming, .. }
            | ChatItem::AssistantText { is_streaming, .. }
            | ChatItem::Reasoning { is_streaming, .. }
            | ChatItem::ToolCall { is_streaming, .. } => *is_streaming = value,
            ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextPart {
    Plain(String),
    #[allow(dead_code)]
    Code {
        lang: String,
        code: String,
    },
}
