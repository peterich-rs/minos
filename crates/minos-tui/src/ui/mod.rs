pub mod agent_picker;
pub mod chat;
pub mod delete_confirm;
pub mod group_chat;
pub mod input_bar;
pub mod project_create_dialog;
pub mod project_list;
pub mod project_sessions;
pub mod room_list;
pub mod status_bar;
pub mod theme;
pub mod thread_list;

use crate::backend::{ProjectEntry, ThreadSummaryEntry};
use crate::focus::{FocusManager, PaneId};
use crate::nav::NavLevel;
use crate::render::{Column, Renderable, Row};
use crate::translation::ChatState;
use crate::ui::chat::RenderCache;
use crate::ui::chat::{AgentChatRenderable, AgentChatTarget};
pub use crate::ui::delete_confirm::DeleteConfirmState;
use crate::ui::group_chat::GroupChatRenderCache;
use crate::ui::input_bar::{AgentMentionCandidate, InputLayoutMetrics, InputState};
use crate::ui::room_list::RoomEntry;
use crate::ui::status_bar::StatusBarState;
use minos_agent_runtime::ThreadState;
use minos_domain::AgentName;
use minos_protocol::LocalGroupChatMessage;
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
    pub focus: FocusManager,
    pub error_flash: Option<(String, Instant)>,
    pub flash_copied: Option<Instant>,
    pub panel_areas: PanelAreas,
    pub input_metrics: [InputLayoutMetrics; 2],
    pub delete_confirm: Option<DeleteConfirmState>,
    pub render_cache: RenderCache,
    pub nav_level: NavLevel,
    pub projects: Vec<ProjectEntry>,
    pub selected_project: Option<usize>,
    pub project_list_state: ListState,
    pub project_sessions: Vec<ThreadSummaryEntry>,
    pub project_create_dialog: Option<ProjectCreateDialogState>,
    pub startup_create_prompt: Option<StartupCreatePromptState>,
}

pub struct AgentPickerState {
    pub selected: usize,
}

pub struct GroupChatState {
    pub messages: Vec<LocalGroupChatMessage>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub max_scroll: u16,
    pub version: u64,
    pub render_cache: GroupChatRenderCache,
}

#[derive(Debug, Clone)]
pub struct ProjectCreateDialogState {
    pub name: String,
    pub path: String,
    pub editing_name: bool,
}

#[derive(Debug, Clone)]
pub struct StartupCreatePromptState {
    pub dir_name: String,
    pub path: String,
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
            focus: FocusManager::new(false),
            error_flash: None,
            flash_copied: None,
            panel_areas: PanelAreas::default(),
            input_metrics: [InputLayoutMetrics::default(); 2],
            delete_confirm: None,
            render_cache: RenderCache::default(),
            nav_level: NavLevel::Projects,
            projects: Vec::new(),
            selected_project: None,
            project_list_state: ListState::default(),
            project_sessions: Vec::new(),
            project_create_dialog: None,
            startup_create_prompt: None,
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

    pub fn flash_copied(&mut self) {
        self.flash_copied = Some(Instant::now());
    }

    pub fn is_flash_copied_active(&self) -> bool {
        self.flash_copied
            .is_some_and(|instant| instant.elapsed().as_secs() < 2)
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
            version: 0,
            render_cache: GroupChatRenderCache::default(),
        }
    }

    pub fn replace_messages(&mut self, messages: Vec<LocalGroupChatMessage>) {
        let messages = sorted_dedup_messages(messages);
        if self.messages != messages {
            self.messages = messages;
            self.bump_version();
            self.scroll_to_bottom();
        }
    }

    pub fn push_message(&mut self, message: LocalGroupChatMessage) {
        if self.merge_messages(std::iter::once(message)) {
            self.scroll_to_bottom();
        }
    }

    pub fn merge_messages<I>(&mut self, messages: I) -> bool
    where
        I: IntoIterator<Item = LocalGroupChatMessage>,
    {
        let mut changed = false;
        for message in messages {
            let key = message_key(&message);
            match self
                .messages
                .iter_mut()
                .find(|existing| message_key(existing) == key)
            {
                Some(existing) if *existing == message => {}
                Some(existing) => {
                    *existing = message;
                    changed = true;
                }
                None => {
                    self.messages.push(message);
                    changed = true;
                }
            }
        }

        if changed {
            sort_messages(&mut self.messages);
            self.bump_version();
        }
        changed
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

    fn bump_version(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
}

fn sorted_dedup_messages(messages: Vec<LocalGroupChatMessage>) -> Vec<LocalGroupChatMessage> {
    let mut keyed = HashMap::<String, LocalGroupChatMessage>::new();
    for message in messages {
        keyed.insert(message_key(&message), message);
    }

    let mut messages: Vec<_> = keyed.into_values().collect();
    sort_messages(&mut messages);
    messages
}

fn message_key(message: &LocalGroupChatMessage) -> String {
    if message.message_id.is_empty() {
        format!(
            "seq:{}:{}:{}",
            message.seq, message.created_at_ms, message.text
        )
    } else {
        message.message_id.clone()
    }
}

fn sort_messages(messages: &mut [LocalGroupChatMessage]) {
    messages.sort_by_key(|message| (message.seq, message.created_at_ms));
}

pub fn render_ui(f: &mut Frame, state: &mut UiState) {
    state.room_input.focused = state.focus.is(PaneId::RoomInput);
    state.agent_input.focused = state.focus.is(PaneId::AgentInput);

    match &state.nav_level {
        NavLevel::Projects => {
            render_projects_level(f, state);
        }
        NavLevel::Sessions { .. } => {
            render_sessions_level(f, state);
        }
        NavLevel::Session { .. } | NavLevel::AgentDetail { .. } => {
            render_legacy(f, state);
        }
    }

    if let Some(dialog) = state.project_create_dialog.as_ref() {
        project_create_dialog::render(f, f.area(), dialog);
    }
    if let Some(prompt) = state.startup_create_prompt.as_ref() {
        render_startup_prompt(f, prompt);
    }
    if let Some(picker) = state.agent_picker.as_ref() {
        let mut overlay =
            agent_picker::AgentPickerRenderable::new(state.status.agents.as_slice(), picker);
        overlay.render(f, f.area());
    }

    if let Some(confirm) = state.delete_confirm.as_ref() {
        let mut overlay = delete_confirm::DeleteConfirmRenderable::new(confirm);
        overlay.render(f, f.area());
    }
}

fn render_legacy(f: &mut Frame, state: &mut UiState) {
    if state.agent_detail_visible {
        render_detail_tree(f, state);
    } else {
        render_overview_tree(f, state);
    }
}

fn render_projects_level(f: &mut Frame, state: &mut UiState) {
    let flash_active = state.is_flash_copied_active();
    let root = f.area();
    let [_, middle, hint_area] = root_sections(root, 1);
    let shell = Rect {
        height: root.height.saturating_sub(hint_area.height),
        ..root
    };
    let areas = Row::areas_for(middle, &[78, 22]);
    let main = areas[0];
    let sidebar = areas[1];

    state.panel_areas = PanelAreas {
        room_list: main,
        room_chat: Rect::default(),
        agent_list: sidebar,
        agent_chat: Rect::default(),
        room_input: Rect::default(),
        agent_input: Rect::default(),
    };
    state.input_metrics = [InputLayoutMetrics::default(); 2];

    let main_row = Row::new(
        vec![
            Box::new(project_list::ProjectListRenderable::new(
                &state.projects,
                state.selected_project,
                &mut state.project_list_state,
                true,
            )),
            Box::new(project_list::ProjectSidebarRenderable::new(
                &state.projects,
                state.selected_project,
            )),
        ],
        vec![78, 22],
    );

    let mut tree = Column::with_fill(
        vec![
            Box::new(status_bar::StatusBarRenderable::new(
                &state.status,
                flash_active,
            )),
            Box::new(main_row),
        ],
        1,
    );
    tree.render(f, shell);
    if let Some(position) = tree.cursor_pos(shell) {
        f.set_cursor_position(position);
    }

    let hint = ratatui::text::Text::from(ratatui::text::Line::from(vec![
        ratatui::text::Span::styled("[n] ", theme::FOCUSED_BORDER),
        ratatui::text::Span::raw("new  "),
        ratatui::text::Span::styled("[Enter] ", theme::FOCUSED_BORDER),
        ratatui::text::Span::raw("open  "),
        ratatui::text::Span::styled("[Esc] ", theme::FOCUSED_BORDER),
        ratatui::text::Span::raw("quit"),
    ]));
    f.render_widget(hint, hint_area);
}

fn render_sessions_level(f: &mut Frame, state: &mut UiState) {
    let mention_candidates = state.room_agent_mention_candidates();
    let flash_active = state.is_flash_copied_active();
    let root = f.area();
    let input_height = input_bar::required_height(&state.room_input, root.width);

    let [_, middle, input_area] = root_sections(root, input_height);
    let areas = Row::areas_for(middle, &[78, 22]);
    let main = areas[0];
    let sidebar = areas[1];

    state.panel_areas = PanelAreas {
        room_list: main,
        room_chat: Rect::default(),
        agent_list: sidebar,
        agent_chat: Rect::default(),
        room_input: input_area,
        agent_input: Rect::default(),
    };
    state.input_metrics = [InputLayoutMetrics::default(); 2];

    let project_name = state
        .selected_project
        .and_then(|idx| state.projects.get(idx))
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let main_row = Row::new(
        vec![
            Box::new(project_sessions::SessionListRenderable::new(
                project_name,
                &state.project_sessions,
                state.selected_thread,
                &mut state.room_list_state,
                true,
            )),
            Box::new(project_sessions::SessionSidebarRenderable::new(
                &state.project_sessions,
                state.selected_thread,
            )),
        ],
        vec![78, 22],
    );

    let input = input_bar::InputBarRenderable::new(
        room_input_title(&state.room_input),
        "Type a message to start a new conversation...",
        &state.room_input,
        mention_candidates.as_slice(),
        &mut state.input_metrics[0],
    );

    let mut tree = Column::with_fill(
        vec![
            Box::new(status_bar::StatusBarRenderable::new(
                &state.status,
                flash_active,
            )),
            Box::new(main_row),
            Box::new(input),
        ],
        1,
    );
    tree.render(f, root);
    if let Some(position) = tree.cursor_pos(root) {
        f.set_cursor_position(position);
    }
}

fn render_startup_prompt(f: &mut Frame, prompt: &StartupCreatePromptState) {
    use ratatui::layout::Flex;
    let area = f.area();
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(7), Constraint::Fill(1)])
        .flex(Flex::Center)
        .split(area);
    let popup = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(52), Constraint::Fill(1)])
        .flex(Flex::Center)
        .split(popup[1])[1];

    f.render_widget(ratatui::widgets::Clear, popup);
    let lines = vec![
        ratatui::text::Line::raw(""),
        ratatui::text::Line::from(format!(
            "  Create project \"{}\" ({})?",
            prompt.dir_name, prompt.path
        )),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("[Y] ", theme::FOCUSED_BORDER),
            ratatui::text::Span::raw("Create & enter  "),
            ratatui::text::Span::styled("[n] ", theme::FOCUSED_BORDER),
            ratatui::text::Span::raw("Skip"),
        ]),
    ];
    let block = ratatui::widgets::Block::bordered()
        .title("New Directory Detected")
        .border_style(theme::FOCUSED_BORDER);
    f.render_widget(ratatui::widgets::Paragraph::new(lines).block(block), popup);
}

fn render_overview_tree(f: &mut Frame, state: &mut UiState) {
    let room_title = state
        .current_room()
        .map(|room| format!("Chat Room: {}", room.title))
        .unwrap_or_else(|| "Chat Room".to_owned());
    let mention_candidates = state.room_agent_mention_candidates();
    let room_input_title = room_input_title(&state.room_input);
    let flash_active = state.is_flash_copied_active();
    let root = f.area();
    let input_height = input_bar::required_height(&state.room_input, root.width);
    let [_, middle, input_area] = root_sections(root, input_height);
    let columns = Row::areas_for(middle, &[20, 55, 25]);

    state.panel_areas = PanelAreas {
        room_list: columns[0],
        room_chat: columns[1],
        agent_list: columns[2],
        agent_chat: Rect::default(),
        room_input: input_area,
        agent_input: Rect::default(),
    };
    state.input_metrics[1] = InputLayoutMetrics::default();

    let main_row = Row::new(
        vec![
            Box::new(room_list::RoomListRenderable::new(
                &state.rooms,
                state.selected_room,
                &mut state.room_list_state,
                state.focus.is(PaneId::RoomList),
            )),
            Box::new(group_chat::GroupChatRenderable::new(
                room_title,
                &mut state.group_chat,
                state.focus.is(PaneId::GroupChat),
            )),
            Box::new(thread_list::ThreadListRenderable::new(
                "Agents",
                &state.threads,
                state.selected_thread,
                &mut state.agent_list_state,
                state.focus.is(PaneId::AgentList),
            )),
        ],
        vec![20, 55, 25],
    );
    let input = input_bar::InputBarRenderable::new(
        room_input_title,
        "Type @ to choose an agent or send to the room",
        &state.room_input,
        mention_candidates.as_slice(),
        &mut state.input_metrics[0],
    );
    let mut tree = Column::with_fill(
        vec![
            Box::new(status_bar::StatusBarRenderable::new(
                &state.status,
                flash_active,
            )),
            Box::new(main_row),
            Box::new(input),
        ],
        1,
    );
    tree.render(f, root);
    if let Some(position) = tree.cursor_pos(root) {
        f.set_cursor_position(position);
    }
}

fn render_detail_tree(f: &mut Frame, state: &mut UiState) {
    let room_title = state
        .current_room()
        .map(|room| format!("Chat Room: {}", room.title))
        .unwrap_or_else(|| "Chat Room".to_owned());
    let mention_candidates = state.room_agent_mention_candidates();
    let pending_agent_request = state
        .current_chat()
        .and_then(ChatState::active_pending_request)
        .is_some();
    let selected_thread_id = state.current_thread_id().map(str::to_owned);
    let room_input_title = room_input_title(&state.room_input);
    let agent_input_title = agent_input_title(pending_agent_request, state.agent_input.multiline);
    let flash_active = state.is_flash_copied_active();
    let agent_input_hint = if pending_agent_request {
        "Reply to the pending agent request"
    } else {
        "Talk directly to the selected agent"
    };
    let root = f.area();
    let input_widths = Row::areas_for(
        Rect {
            x: root.x,
            y: root.y,
            width: root.width,
            height: 0,
        },
        &[65, 35],
    );
    let input_height = input_bar::required_height(&state.room_input, input_widths[0].width).max(
        input_bar::required_height(&state.agent_input, input_widths[1].width),
    );
    let [_, middle, input_area] = root_sections(root, input_height);
    let columns = Row::areas_for(middle, &[45, 20, 35]);
    let inputs = Row::areas_for(input_area, &[65, 35]);

    state.panel_areas = PanelAreas {
        room_list: Rect::default(),
        room_chat: columns[0],
        agent_list: columns[1],
        agent_chat: columns[2],
        room_input: inputs[0],
        agent_input: inputs[1],
    };

    let agent_chat_target = if let Some(thread_id) = selected_thread_id.as_deref() {
        if let Some(chat) = state.chat_states.get_mut(thread_id) {
            AgentChatTarget::Chat {
                chat,
                cache: &mut state.render_cache,
            }
        } else {
            AgentChatTarget::Empty
        }
    } else {
        AgentChatTarget::Empty
    };

    let [room_metrics, agent_metrics] = &mut state.input_metrics;
    let main_row = Row::new(
        vec![
            Box::new(group_chat::GroupChatRenderable::new(
                room_title,
                &mut state.group_chat,
                state.focus.is(PaneId::GroupChat),
            )),
            Box::new(thread_list::ThreadListRenderable::new(
                "Agents",
                &state.threads,
                state.selected_thread,
                &mut state.agent_list_state,
                state.focus.is(PaneId::AgentList),
            )),
            Box::new(AgentChatRenderable::new(
                agent_chat_target,
                state.focus.is(PaneId::AgentChat),
            )),
        ],
        vec![45, 20, 35],
    );
    let input_row = Row::new(
        vec![
            Box::new(input_bar::InputBarRenderable::new(
                room_input_title,
                "Type @ to choose an agent or send to the room",
                &state.room_input,
                mention_candidates.as_slice(),
                room_metrics,
            )),
            Box::new(input_bar::InputBarRenderable::new(
                agent_input_title,
                agent_input_hint,
                &state.agent_input,
                &[],
                agent_metrics,
            )),
        ],
        vec![65, 35],
    );
    let mut tree = Column::with_fill(
        vec![
            Box::new(status_bar::StatusBarRenderable::new(
                &state.status,
                flash_active,
            )),
            Box::new(main_row),
            Box::new(input_row),
        ],
        1,
    );
    tree.render(f, root);
    if let Some(position) = tree.cursor_pos(root) {
        f.set_cursor_position(position);
    }
}

fn root_sections(area: Rect, input_height: u16) -> [Rect; 3] {
    let available_height = area.height.saturating_sub(1);
    let input_height = input_height.min(available_height);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(input_height.max(1)),
        ])
        .split(area);
    [outer[0], outer[1], outer[2]]
}

fn room_input_title(state: &InputState) -> &'static str {
    if state.multiline {
        "Chat Room Input [multi]"
    } else {
        "Chat Room Input"
    }
}

fn agent_input_title(pending_agent_request: bool, multiline: bool) -> &'static str {
    match (pending_agent_request, multiline) {
        (true, true) => "Agent Input: Reply Required [multi]",
        (true, false) => "Agent Input: Reply Required",
        (false, true) => "Agent Input [multi]",
        (false, false) => "Agent Input",
    }
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
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

    #[test]
    fn group_chat_merge_duplicate_keeps_version() {
        let mut state = GroupChatState::new();
        let message = group_message(1, "m1", "same message");

        assert!(state.merge_messages(vec![message.clone()]));
        let version = state.version;

        assert!(!state.merge_messages(vec![message]));
        assert_eq!(state.version, version);
    }
}
