use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use minos_agent_runtime::{ManagerEvent, ThreadState};
use minos_chat_store::mcp_socket::{SocketRequest, SocketResponse};
use minos_domain::{AgentName, AgentStatus};
use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};
use tracing::debug;

use crate::action::{
    Action, GlobalAction, InputAction, InputTarget, ScrollDirection, ScrollTarget,
};
use crate::agent_route::{parse_agent_name, short_thread_id, thread_can_receive_message};
use crate::backend::AgentBackend;
use crate::effect::Effect;
use crate::event::AppEvent;
use crate::focus::PaneId;
use crate::group_chat::GroupChatStore;
use crate::state::{self, frame_marks_agent_result_done, rect_contains, thread_is_done, AppState};
use crate::translation::{ChatState, PendingAgentRequestKind, PendingQuestionSpec};
use crate::ui::{ThreadEntry, UiState};

mod clipboard;
mod event_loop;
mod event_mapping;
mod group_chat;
mod helpers;
mod lifecycle;
mod mcp;
mod submission;
mod thread_ops;

use clipboard::{copy_to_clipboard, paste_from_clipboard};
use helpers::*;

#[cfg(test)]
use clipboard::TEST_CLIPBOARD;

pub struct App {
    backend: Arc<dyn AgentBackend>,
    state: AppState,
    ui: UiState,
    should_quit: bool,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<AppEvent>>,
    frame_requester: Option<crate::frame::FrameRequester>,
}

impl App {
    pub fn new(backend: Arc<dyn AgentBackend>, readonly: bool, workspace: PathBuf) -> Self {
        let group_chat_store = default_group_chat_store(&workspace);
        Self::with_group_chat_store(backend, readonly, workspace, group_chat_store)
    }

    fn with_group_chat_store(
        backend: Arc<dyn AgentBackend>,
        readonly: bool,
        workspace: PathBuf,
        group_chat_store: GroupChatStore,
    ) -> Self {
        let ui = UiState::new(readonly);
        Self {
            backend,
            state: AppState::new(workspace, group_chat_store),
            ui,
            should_quit: false,
            event_tx: None,
            frame_requester: None,
        }
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
