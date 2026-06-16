pub mod agent_picker;
pub mod chat;
pub mod group_chat;
pub mod input_bar;
pub mod room_list;
pub mod status_bar;
pub mod theme;
pub mod thread_list;

use crate::translation::ChatState;
use crate::ui::chat::RenderCache;
use crate::ui::input_bar::{AgentMentionCandidate, InputState};
use crate::ui::room_list::RoomEntry;
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
    RoomList,
    RoomChat,
    AgentList,
    AgentChat,
    RoomInput,
    AgentInput,
}

pub struct UiState {
    pub status: StatusBarState,
    pub rooms: Vec<RoomEntry>,
    pub selected_room: Option<usize>,
    pub room_list_state: ListState,
    pub threads: Vec<ThreadEntry>,
    pub selected_thread: Option<usize>,
    pub agent_list_state: ListState,
    pub group_chat: GroupChatState,
    pub agent_picker: Option<AgentPickerState>,
    pub chat_states: HashMap<String, ChatState>,
    pub room_input: InputState,
    pub agent_input: InputState,
    pub agent_detail_visible: bool,
    pub focus: Focus,
    pub error_flash: Option<(String, Instant)>,
    pub panel_areas: PanelAreas,
    pub delete_confirm: Option<DeleteConfirmState>,
    pub render_cache: RenderCache,
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
    pub room_list: Rect,
    pub room_chat: Rect,
    pub agent_list: Rect,
    pub agent_chat: Rect,
    pub room_input: Rect,
    pub agent_input: Rect,
}

impl UiState {
    pub fn new(readonly: bool) -> Self {
        Self {
            status: StatusBarState::new(),
            rooms: Vec::new(),
            selected_room: None,
            room_list_state: ListState::default(),
            threads: Vec::new(),
            selected_thread: None,
            agent_list_state: ListState::default(),
            group_chat: GroupChatState::new(),
            agent_picker: None,
            chat_states: HashMap::new(),
            room_input: InputState::new(readonly),
            agent_input: InputState::new(readonly),
            agent_detail_visible: false,
            focus: Focus::RoomList,
            error_flash: None,
            panel_areas: PanelAreas::default(),
            delete_confirm: None,
            render_cache: RenderCache::default(),
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

    pub fn current_chat(&self) -> Option<&ChatState> {
        let id = self.selected_thread.and_then(|i| self.threads.get(i))?;
        self.chat_states.get(&id.thread_id)
    }

    /// Returns the active chat alongside the shared render cache, using a split
    /// borrow so callers can mutate both in the same scope.
    pub fn current_chat_and_cache_mut(&mut self) -> Option<(&mut ChatState, &mut RenderCache)> {
        let id = self.selected_thread.and_then(|i| self.threads.get(i))?;
        let chat = self.chat_states.get_mut(&id.thread_id)?;
        Some((chat, &mut self.render_cache))
    }

    pub fn set_error(&mut self, msg: String) {
        self.error_flash = Some((msg, Instant::now()));
    }

    pub fn current_room(&self) -> Option<&RoomEntry> {
        self.selected_room.and_then(|index| self.rooms.get(index))
    }

    pub fn room_agent_mention_candidates(&self) -> Vec<AgentMentionCandidate> {
        let mut candidates: Vec<AgentMentionCandidate> = self
            .status
            .agents
            .iter()
            .map(|agent| AgentMentionCandidate::installed(agent.name, agent.status.clone()))
            .collect();
        candidates.extend(
            self.threads
                .iter()
                .filter(|thread| !matches!(thread.state, ThreadState::Closed { .. }))
                .map(|thread| {
                    AgentMentionCandidate::existing(
                        thread.agent,
                        thread.thread_id.clone(),
                        short_thread_id(&thread.thread_id),
                    )
                }),
        );
        candidates
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
        self.messages = sorted_dedup_messages(messages);
        self.scroll_to_bottom();
    }

    pub fn push_message(&mut self, message: LocalGroupChatMessage) {
        self.merge_messages(std::iter::once(message));
        self.scroll_to_bottom();
    }

    pub fn merge_messages<I>(&mut self, messages: I)
    where
        I: IntoIterator<Item = LocalGroupChatMessage>,
    {
        self.messages.extend(messages);
        self.messages = sorted_dedup_messages(std::mem::take(&mut self.messages));
    }

    pub fn last_seq(&self) -> u64 {
        self.messages
            .iter()
            .map(|message| message.seq)
            .max()
            .unwrap_or(0)
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

fn sorted_dedup_messages(messages: Vec<LocalGroupChatMessage>) -> Vec<LocalGroupChatMessage> {
    let mut keyed = HashMap::<String, LocalGroupChatMessage>::new();
    for message in messages {
        let key = if message.message_id.is_empty() {
            format!(
                "seq:{}:{}:{}",
                message.seq, message.created_at_ms, message.text
            )
        } else {
            message.message_id.clone()
        };
        keyed.insert(key, message);
    }

    let mut messages: Vec<_> = keyed.into_values().collect();
    messages.sort_by_key(|message| (message.seq, message.created_at_ms));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_protocol::LocalGroupChatMessageKind;

    fn group_message(seq: u64, message_id: &str, text: &str) -> LocalGroupChatMessage {
        LocalGroupChatMessage {
            seq,
            message_id: message_id.to_owned(),
            created_at_ms: i64::try_from(seq).unwrap_or(0),
            kind: LocalGroupChatMessageKind::User,
            text: text.to_owned(),
            agent: Some(AgentName::Codex),
            thread_id: Some("thread-1".into()),
            thread_short_id: Some("thread-1".into()),
            workspace: Some("/tmp/ws".into()),
        }
    }

    #[test]
    fn group_chat_merge_sorts_by_sequence_and_dedups_messages() {
        let mut state = GroupChatState::new();

        state.push_message(group_message(5, "newer", "newer local message"));
        state.merge_messages(vec![
            group_message(2, "older", "older daemon message"),
            group_message(5, "newer", "duplicate local message"),
        ]);

        assert_eq!(
            state
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![2, 5]
        );
        assert_eq!(state.messages[0].message_id, "older");
        assert_eq!(state.messages[1].message_id, "newer");
    }
}

pub fn render_ui(f: &mut Frame, state: &mut UiState) {
    state.room_input.focused = matches!(state.focus, Focus::RoomInput);
    state.agent_input.focused = matches!(state.focus, Focus::AgentInput);

    let available_height = f.area().height.saturating_sub(1);
    let room_input_height = input_bar::required_height(&state.room_input, f.area().width);
    let detail_agent_width = f.area().width.saturating_mul(35) / 100;
    let agent_input_height = input_bar::required_height(&state.agent_input, detail_agent_width);
    let input_height = if state.agent_detail_visible {
        room_input_height.max(agent_input_height)
    } else {
        room_input_height
    }
    .min(available_height);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(input_height.max(1)),
        ])
        .split(f.area());

    status_bar::render_status_bar(f, outer[0], &state.status);

    if state.agent_detail_visible {
        render_detail_mode(f, outer[1], outer[2], state);
    } else {
        render_overview_mode(f, outer[1], outer[2], state);
    }

    if let Some(picker) = state.agent_picker.as_ref() {
        agent_picker::render_agent_picker(f, state.status.agents.as_slice(), picker);
    }

    if let Some(confirm) = state.delete_confirm.as_ref() {
        render_delete_confirm(f, confirm);
    }
}

fn render_overview_mode(f: &mut Frame, middle: Rect, input_area: Rect, state: &mut UiState) {
    let room_title = state
        .current_room()
        .map(|room| format!("Chat Room: {}", room.title))
        .unwrap_or_else(|| "Chat Room".to_owned());
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(55),
            Constraint::Percentage(25),
        ])
        .split(middle);

    state.panel_areas = PanelAreas {
        room_list: columns[0],
        room_chat: columns[1],
        agent_list: columns[2],
        agent_chat: Rect::default(),
        room_input: input_area,
        agent_input: Rect::default(),
    };

    room_list::render_room_list(
        f,
        columns[0],
        &state.rooms,
        state.selected_room,
        &mut state.room_list_state,
        matches!(state.focus, Focus::RoomList),
    );
    group_chat::render_group_chat(
        f,
        columns[1],
        room_title.as_str(),
        &mut state.group_chat,
        matches!(state.focus, Focus::RoomChat),
    );
    thread_list::render_thread_list(
        f,
        columns[2],
        "Agents",
        &state.threads,
        state.selected_thread,
        &mut state.agent_list_state,
        matches!(state.focus, Focus::AgentList),
    );
    let mention_candidates = state.room_agent_mention_candidates();
    input_bar::render_input_bar(
        f,
        input_area,
        "Chat Room Input",
        "Type @ to choose an agent or send to the room",
        &state.room_input,
        mention_candidates.as_slice(),
    );
}

fn render_detail_mode(f: &mut Frame, middle: Rect, input_area: Rect, state: &mut UiState) {
    let room_title = state
        .current_room()
        .map(|room| format!("Chat Room: {}", room.title))
        .unwrap_or_else(|| "Chat Room".to_owned());
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(45),
            Constraint::Percentage(20),
            Constraint::Percentage(35),
        ])
        .split(middle);
    let inputs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(input_area);

    state.panel_areas = PanelAreas {
        room_list: Rect::default(),
        room_chat: columns[0],
        agent_list: columns[1],
        agent_chat: columns[2],
        room_input: inputs[0],
        agent_input: inputs[1],
    };

    group_chat::render_group_chat(
        f,
        columns[0],
        room_title.as_str(),
        &mut state.group_chat,
        matches!(state.focus, Focus::RoomChat),
    );
    thread_list::render_thread_list(
        f,
        columns[1],
        "Agents",
        &state.threads,
        state.selected_thread,
        &mut state.agent_list_state,
        matches!(state.focus, Focus::AgentList),
    );
    let agent_chat_focused = matches!(state.focus, Focus::AgentChat);
    if let Some((chat, cache)) = state.current_chat_and_cache_mut() {
        chat::render_chat(f, columns[2], chat, agent_chat_focused, cache);
    } else {
        render_agent_chat_placeholder(f, columns[2], agent_chat_focused);
    }

    let mention_candidates = state.room_agent_mention_candidates();
    input_bar::render_input_bar(
        f,
        inputs[0],
        "Chat Room Input",
        "Type @ to choose an agent or send to the room",
        &state.room_input,
        mention_candidates.as_slice(),
    );
    let pending_agent_request = state
        .current_chat()
        .and_then(ChatState::active_pending_request)
        .is_some();
    input_bar::render_input_bar(
        f,
        inputs[1],
        if pending_agent_request {
            "Agent Input: Reply Required"
        } else {
            "Agent Input"
        },
        if pending_agent_request {
            "Reply to the pending agent request"
        } else {
            "Talk directly to the selected agent"
        },
        &state.agent_input,
        &[],
    );
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
}

fn render_agent_chat_placeholder(f: &mut Frame, area: Rect, focused: bool) {
    let paragraph = ratatui::widgets::Paragraph::new(
        "No agent selected\n\nChoose an agent from the list to inspect its detailed transcript.",
    )
    .block(
        theme::border_block()
            .title("Agent Detail")
            .border_style(if focused {
                theme::FOCUSED_BORDER
            } else {
                ratatui::style::Style::new().fg(theme::BORDER_FG)
            }),
    );
    f.render_widget(paragraph, area);
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
