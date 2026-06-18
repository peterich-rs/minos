//! Semantic user intents produced by the event mapping layer.

use std::path::PathBuf;

use minos_agent_runtime::ManagerEvent;
use minos_domain::AgentName;
use minos_protocol::LocalIngestFrame;

use crate::event::McpToolEvent;

pub enum Action {
    Global(GlobalAction),
    Room(RoomAction),
    Agent(AgentAction),
    Input(InputTarget, InputAction),
    EffectCompleted(EffectResult),
}

pub enum GlobalAction {
    Quit,
    CycleFocus,
    CycleFocusPrev,
    OpenAgentPicker,
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
    },
    McpToolCall(McpToolEvent),
    Tick,
    RequestRedraw,
    ConfirmDelete,
    CancelDelete,
    SelectPrevious,
    SelectNext,
    Escape,
    Enter,
    SelectIndex(usize),
}

pub enum RoomAction {
    Select(usize),
    Scroll(ScrollDirection, u16),
}

pub enum AgentAction {
    Select(usize),
    Scroll(ScrollDirection, u16),
    Close,
    Delete,
    ToggleToolExpansion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputTarget {
    Room,
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
    RoomList,
    GroupChat,
    AgentList,
    AgentChat,
    ActivePane,
}

pub enum ClickTarget {
    RoomList,
    GroupChat,
    AgentList,
    AgentChat,
    RoomInput,
    AgentInput,
}

pub enum EffectResult {
    AgentStarted {
        agent: AgentName,
        thread_id: String,
        cwd: PathBuf,
        text: String,
    },
    SendFailed {
        thread_id: String,
        error: String,
    },
    IngestArrived(LocalIngestFrame),
    ManagerEvent(ManagerEvent),
}
