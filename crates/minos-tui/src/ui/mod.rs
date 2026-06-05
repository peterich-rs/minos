pub mod agent_picker;
pub mod chat;
pub mod group_chat;
pub mod input_bar;
pub mod status_bar;
pub mod theme;
pub mod thread_list;

use crate::translation::ChatState;
use crate::ui::input_bar::InputState;
use crate::ui::status_bar::StatusBarState;
use minos_agent_runtime::ThreadState;
use minos_domain::AgentName;
use minos_protocol::LocalGroupChatMessage;
use ratatui::{
    layout::{Constraint, Direction, Flex, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Clear, ListState, Paragraph, Wrap},
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
    pub group_chat: GroupChatState,
    pub agent_picker: Option<AgentPickerState>,
    pub chat_states: HashMap<String, ChatState>,
    pub input: InputState,
    pub focus: Focus,
    pub error_flash: Option<(String, Instant)>,
    pub panel_areas: PanelAreas,
    pub delete_confirm: Option<DeleteConfirmState>,
}

pub struct AgentPickerState {
    pub selected: usize,
}

pub struct DeleteConfirmState {
    pub thread_id: String,
    pub agent: AgentName,
    pub workspace: PathBuf,
    pub selected_index: usize,
}

pub struct GroupChatState {
    pub messages: Vec<LocalGroupChatMessage>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PanelAreas {
    pub thread_list: Rect,
    pub group_chat: Rect,
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
            group_chat: GroupChatState::new(),
            agent_picker: None,
            chat_states: HashMap::new(),
            input: InputState::new(readonly),
            focus: Focus::ThreadList,
            error_flash: None,
            panel_areas: PanelAreas::default(),
            delete_confirm: None,
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

impl GroupChatState {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
            max_scroll: 0,
        }
    }

    pub fn replace_messages(&mut self, messages: Vec<LocalGroupChatMessage>) {
        self.messages = messages;
        self.scroll_to_bottom();
    }

    pub fn push_message(&mut self, message: LocalGroupChatMessage) {
        self.messages.push(message);
        self.scroll_to_bottom();
    }

    pub fn update_max_scroll(&mut self, max_scroll: u16) {
        self.max_scroll = max_scroll;
        if !self.auto_scroll {
            self.scroll_offset = self.scroll_offset.min(self.max_scroll);
        }
    }

    pub fn active_scroll(&self) -> u16 {
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
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        if self.auto_scroll {
            return;
        }

        self.scroll_offset = self
            .scroll_offset
            .saturating_add(lines)
            .min(self.max_scroll);
        if self.scroll_offset >= self.max_scroll {
            self.scroll_to_bottom();
        }
    }

    fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        self.scroll_offset = 0;
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
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(middle[0]);
    state.panel_areas = PanelAreas {
        thread_list: left[0],
        group_chat: left[1],
        chat: middle[1],
        input: outer[2],
    };

    thread_list::render_thread_list(
        f,
        left[0],
        &state.threads,
        state.selected_thread,
        &mut state.thread_list_state,
        thread_list_focused,
    );
    group_chat::render_group_chat(f, left[1], &mut state.group_chat);

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

    if let Some(confirm) = state.delete_confirm.as_ref() {
        render_delete_confirm(f, confirm);
    }
}

fn render_delete_confirm(f: &mut Frame, state: &DeleteConfirmState) {
    let area = centered_rect(f.area(), 64, 8);
    f.render_widget(Clear, area);

    let tid_short = &state.thread_id[..8.min(state.thread_id.len())];
    let workspace = state
        .workspace
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let lines = vec![
        Line::from("Delete this local thread?"),
        Line::from(""),
        Line::from(vec![
            Span::raw("Thread "),
            Span::styled(tid_short.to_owned(), theme::HIGHLIGHTED),
            Span::raw(format!(
                "  Agent {}  Workspace {}",
                state.agent.bin_name(),
                workspace
            )),
        ]),
        Line::from(""),
        Line::from("Enter/Y confirm    Esc/N cancel"),
    ];
    let paragraph = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title("Confirm Delete")
                .border_style(theme::FOCUSED_BORDER),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let [vertical] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);

    let [horizontal] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(vertical);

    horizontal
}
