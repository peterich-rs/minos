pub mod chat;
pub mod input_bar;
pub mod status_bar;
pub mod thread_list;
pub mod theme;

use crate::translation::ChatState;
use crate::ui::input_bar::InputState;
use crate::ui::status_bar::StatusBarState;
use minos_agent_runtime::ThreadState;
use minos_domain::AgentName;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    widgets::ListState,
    Frame,
};

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

pub struct ThreadEntry {
    pub thread_id: String,
    pub agent: AgentName,
    pub workspace: PathBuf,
    pub state: ThreadState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    ThreadList,
    Chat,
    Input,
}

pub struct UiState {
    pub status: StatusBarState,
    pub threads: Vec<ThreadEntry>,
    pub selected_thread: Option<usize>,
    pub thread_list_state: ListState,
    pub chat_states: HashMap<String, ChatState>,
    pub input: InputState,
    pub focus: Focus,
    pub error_flash: Option<(String, Instant)>,
}

impl UiState {
    pub fn new(readonly: bool) -> Self {
        Self {
            status: StatusBarState::new(),
            threads: Vec::new(),
            selected_thread: None,
            thread_list_state: ListState::default(),
            chat_states: HashMap::new(),
            input: InputState::new(readonly),
            focus: Focus::Input,
            error_flash: None,
        }
    }

    pub fn current_thread_id(&self) -> Option<&str> {
        self.selected_thread
            .and_then(|i| self.threads.get(i))
            .map(|t| t.thread_id.as_str())
    }

    pub fn current_chat(&self) -> Option<&ChatState> {
        self.current_thread_id()
            .and_then(|id| self.chat_states.get(id))
    }

    pub fn current_chat_mut(&mut self) -> Option<&mut ChatState> {
        let id = self.selected_thread.and_then(|i| self.threads.get(i))?;
        self.chat_states.get_mut(&id.thread_id)
    }

    pub fn set_error(&mut self, msg: String) {
        self.error_flash = Some((msg, Instant::now()));
    }
}

pub fn render_ui(f: &mut Frame, state: &mut UiState) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    status_bar::render_status_bar(f, outer[0], &state.status);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(outer[1]);

    thread_list::render_thread_list(
        f,
        middle[0],
        &state.threads,
        state.selected_thread,
        &mut state.thread_list_state,
    );

    if let Some(chat) = state.current_chat() {
        chat::render_chat(f, middle[1], &chat.messages, chat.scroll_offset);
    } else {
        let placeholder = ratatui::widgets::Paragraph::new("No thread selected");
        f.render_widget(placeholder, middle[1]);
    }

    input_bar::render_input_bar(f, outer[2], &state.input);
}
