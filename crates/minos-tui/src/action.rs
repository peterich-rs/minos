//! Semantic user intents produced by the event mapping layer.

use std::path::PathBuf;

use minos_domain::AgentName;

pub enum Action {
    Global(GlobalAction),
    Conversation(ConversationAction),
    Agent(AgentAction),
    Input(InputTarget, InputAction),
    EffectCompleted(EffectResult),
    Nav(crate::nav::NavAction),
}

pub enum GlobalAction {
    Quit,
    CycleFocus,
    CycleFocusPrev,
    Scroll(ScrollTarget, ScrollDirection, u16),
    InterruptOrQuit,
    MouseClick {
        target: ClickTarget,
        x: u16,
        y: u16,
    },
    MouseDrag {
        x: u16,
        y: u16,
        release: bool,
    },
    MouseScroll {
        target: ScrollTarget,
        direction: ScrollDirection,
        /// Wheel ticks × lines-per-tick (coalesced bursts accumulate here).
        lines: u16,
    },
    Tick,
    RequestRedraw,
    ConfirmDelete,
    CancelDelete,
    Escape,
    Enter,
}

pub enum ConversationAction {
    Scroll(ScrollDirection, u16),
}

pub enum AgentAction {
    Select(usize),
    Scroll(ScrollDirection, u16),
    Close,
    Delete,
    ToggleToolExpansion,
    ApprovalSelectNext,
    ApprovalSelectPrev,
    ApprovalConfirm,
    ApprovalQuickPick(usize),
    ApprovalCancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputTarget {
    Conversation,
    Agent,
}

pub enum InputAction {
    InsertChar(char),
    InsertText(String),
    DeleteBackward,
    DeleteForward,
    DeleteWord,
    DeleteNextWord,
    DeleteToStartOfLine,
    DeleteToEndOfLine,
    MoveCursor(CursorDirection),
    MoveCursorWord(CursorDirection),
    MoveCursorLine(CursorLineDirection),
    MoveToBufferStart,
    MoveToBufferEnd,
    Submit,
    NewLine,
    ToggleMultilineMode,
    ToggleCursorStyle,
    HistoryNavigate(HistoryDirection),
    TogglePathPicker,
    AcceptMentionCompletion,
    AcceptPathCompletion,
    SelectPreviousPickerItem,
    SelectNextPickerItem,
    DismissPicker,
    Consume,
}

pub enum CursorDirection {
    Left,
    Right,
    LineStart,
    LineEnd,
}

pub enum CursorLineDirection {
    Up,
    Down,
}

pub enum HistoryDirection {
    Previous,
    Next,
}

pub enum ScrollDirection {
    Up,
    Down,
    Top,
    Bottom,
}

pub enum ScrollTarget {
    MainList,
    ConversationChat,
    AgentList,
    AgentChat,
    ActivePane,
}

pub enum ClickTarget {
    MainList,
    ConversationChat,
    AgentList,
    AgentChat,
    ConversationInput,
    AgentInput,
}

pub enum EffectResult {
    AgentStarted {
        agent: AgentName,
        session_id: String,
        cwd: PathBuf,
        text: String,
    },
    SendFailed {
        session_id: String,
        error: String,
    },
    ProjectCreated(crate::backend::ProjectEntry),
    ConversationsLoaded {
        project_id: String,
        conversations: Vec<crate::backend::ConversationEntry>,
    },
    ConversationOpened {
        project_id: String,
        conversation_id: String,
        messages: Vec<crate::backend::ConversationMessageEntry>,
        sessions: Vec<crate::backend::SessionSummaryEntry>,
    },
    ConversationAgentStarted {
        conversation_id: String,
        agent: AgentName,
        session_id: String,
        cwd: PathBuf,
        text: String,
    },
    ProjectFailed(String),
}
