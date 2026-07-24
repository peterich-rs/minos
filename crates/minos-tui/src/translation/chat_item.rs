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
        /// When `None`, expanded only while streaming (idle thinking is collapsed).
        is_user_toggled: Option<bool>,
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
        is_user_toggled: Option<bool>,
        is_streaming: bool,
    },
    SubagentCall {
        message_id: String,
        tool_call_id: String,
        sub_session_id: String,
        agent: minos_domain::AgentName,
        model: Option<String>,
        prompt_summary: Option<String>,
        status: minos_ui_protocol::SubagentStatus,
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
            | ChatItem::ToolCall { message_id, .. }
            | ChatItem::SubagentCall { message_id, .. } => Some(message_id),
            ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => None,
        }
    }

    pub(super) fn set_streaming(&mut self, value: bool) {
        match self {
            ChatItem::UserMessage { is_streaming, .. }
            | ChatItem::AssistantText { is_streaming, .. }
            | ChatItem::Reasoning { is_streaming, .. }
            | ChatItem::ToolCall { is_streaming, .. }
            | ChatItem::SubagentCall { is_streaming, .. } => *is_streaming = value,
            ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => {}
        }
    }

    pub(super) fn is_streaming(&self) -> bool {
        match self {
            ChatItem::UserMessage { is_streaming, .. }
            | ChatItem::AssistantText { is_streaming, .. }
            | ChatItem::Reasoning { is_streaming, .. }
            | ChatItem::ToolCall { is_streaming, .. }
            | ChatItem::SubagentCall { is_streaming, .. } => *is_streaming,
            ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => false,
        }
    }

    /// Whether a foldable item (tool / thinking) currently shows its body.
    pub(crate) fn is_fold_expanded(&self) -> bool {
        match self {
            ChatItem::ToolCall {
                is_expanded,
                is_user_toggled,
                ..
            } => is_user_toggled.unwrap_or(*is_expanded),
            ChatItem::Reasoning {
                is_streaming,
                is_user_toggled,
                ..
            } => is_user_toggled.unwrap_or(*is_streaming),
            _ => false,
        }
    }

    pub(crate) fn is_foldable(&self) -> bool {
        matches!(self, ChatItem::ToolCall { .. } | ChatItem::Reasoning { .. })
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
