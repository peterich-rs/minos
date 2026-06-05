pub mod agent_picker;
pub mod chat;
pub mod input_bar;
pub mod status_bar;
pub mod theme;
pub mod thread_list;

use crate::translation::ChatState;
use crate::ui::input_bar::InputState;
use crate::ui::status_bar::StatusBarState;
use minos_agent_runtime::ThreadState;
use minos_domain::AgentName;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
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
    pub agent_picker: Option<AgentPickerState>,
    pub chat_states: HashMap<String, ChatState>,
    pub input: InputState,
    pub focus: Focus,
    pub error_flash: Option<(String, Instant)>,
    pub panel_areas: PanelAreas,
}

pub struct AgentPickerState {
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PanelAreas {
    pub thread_list: Rect,
    pub chat: Rect,
    pub input: Rect,
}

impl UiState {
    pub fn new(readonly: bool) -> Self {
        Self {
            status: StatusBarState::new(),
            threads: Vec::new(),
            selected_thread: None,
            thread_list_state: ListState::default(),
            agent_picker: None,
            chat_states: HashMap::new(),
            input: InputState::new(readonly),
            focus: Focus::ThreadList,
            error_flash: None,
            panel_areas: PanelAreas::default(),
        }
    }

    pub fn current_thread_id(&self) -> Option<&str> {
        self.selected_thread
            .and_then(|i| self.threads.get(i))
            .map(|t| t.thread_id.as_str())
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
    state.input.focused = matches!(state.focus, Focus::Input);
    let chat_focused = matches!(state.focus, Focus::Chat);
    let thread_list_focused = matches!(state.focus, Focus::ThreadList);
    let input_height =
        input_bar::required_height(&state.input).min(f.area().height.saturating_sub(1));

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(input_height.max(1)),
        ])
        .split(f.area());

    status_bar::render_status_bar(f, outer[0], &state.status);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(outer[1]);
    state.panel_areas = PanelAreas {
        thread_list: middle[0],
        chat: middle[1],
        input: outer[2],
    };

    thread_list::render_thread_list(
        f,
        middle[0],
        &state.threads,
        state.selected_thread,
        &mut state.thread_list_state,
        thread_list_focused,
    );

    if let Some(chat) = state.current_chat_mut() {
        chat::render_chat(f, middle[1], chat, chat_focused);
    } else {
        let placeholder = ratatui::widgets::Paragraph::new(
            "No thread selected\n\nPress `n` to pick an agent, or type `@codex` / `@claude` / `@gemini` / `@opencode` below.\nUse `Tab` to move focus and `PgUp`/`PgDn` to scroll chat.",
        )
        .block(theme::border_block().title("Chat").border_style(if chat_focused {
            theme::FOCUSED_BORDER
        } else {
            ratatui::style::Style::new().fg(theme::BORDER_FG)
        }));
        f.render_widget(placeholder, middle[1]);
    }

    input_bar::render_input_bar(f, outer[2], &state.input, state.status.agents.as_slice());

    if let Some(picker) = state.agent_picker.as_ref() {
        agent_picker::render_agent_picker(f, state.status.agents.as_slice(), picker);
    }
}
