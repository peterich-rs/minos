pub mod agent_picker;
pub mod chat;
pub mod conversation_detail;
pub mod conversation_list;
pub mod delete_confirm;
pub mod input_bar;
pub mod project_create_dialog;
pub mod project_list;
pub mod status_bar;
pub mod theme;

use crate::backend::{
    ConversationEntry, ConversationMessageEntry, ProjectEntry, ThreadSummaryEntry,
};
use crate::focus::{FocusManager, PaneId};
use crate::nav::NavLevel;
use crate::render::{Column, Renderable, Row};
use crate::translation::ChatState;
use crate::ui::chat::RenderCache;
use crate::ui::chat::{AgentChatRenderable, AgentChatTarget};
pub use crate::ui::delete_confirm::DeleteConfirmState;
use crate::ui::input_bar::{AgentMentionCandidate, InputLayoutMetrics, InputState};
use crate::ui::status_bar::StatusBarState;
use minos_agent_runtime::ThreadState;
use minos_domain::AgentName;
use minos_protocol::LocalGroupChatMessage;
use minos_ui_protocol::SubagentStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::ListState,
    Frame,
};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

pub struct ThreadEntry {
    pub thread_id: String,
    pub agent: AgentName,
    pub workspace: PathBuf,
    pub state: ThreadState,
    pub parent_thread_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FlatAgentSession {
    pub source_index: usize,
    pub thread_id: String,
    pub agent: AgentName,
    pub parent_thread_id: Option<String>,
    pub depth: u8,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct SubagentInfo {
    pub parent_thread_id: String,
    pub tool_call_id: String,
    pub agent: AgentName,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub title: Option<String>,
    pub status: SubagentStatus,
}

pub struct UiState {
    pub status: StatusBarState,
    pub threads: Vec<ThreadEntry>,
    pub selected_thread: Option<usize>,
    pub agent_list_state: ListState,
    pub group_chat: GroupChatState,
    pub agent_picker: Option<AgentPickerState>,
    pub chat_states: HashMap<String, ChatState>,
    pub room_input: InputState,
    pub agent_input: InputState,
    pub focus: FocusManager,
    pub error_flash: Option<(String, Instant)>,
    pub flash_copied: Option<Instant>,
    pub panel_areas: PanelAreas,
    pub input_metrics: [InputLayoutMetrics; 2],
    pub delete_confirm: Option<DeleteConfirmState>,
    pub render_cache: RenderCache,
    pub nav_stack: Vec<NavLevel>,
    pub projects: Vec<ProjectEntry>,
    pub selected_project: Option<usize>,
    pub project_list_state: ListState,
    pub conversations: Vec<ConversationEntry>,
    pub selected_conversation: Option<usize>,
    pub conversation_list_state: ListState,
    pub conversation_messages: Vec<ConversationMessageEntry>,
    pub conversation_scroll_offset: u16,
    pub conversation_auto_scroll: bool,
    pub conversation_max_scroll: u16,
    pub conversation_agent_sessions: Vec<ThreadSummaryEntry>,
    pub selected_agent_session: Option<usize>,
    pub subagent_info: HashMap<String, SubagentInfo>,
    pub project_create_dialog: Option<ProjectCreateDialogState>,
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
}

#[derive(Debug, Clone)]
pub struct ProjectCreateDialogState {
    pub name: String,
    pub path: String,
    pub editing_name: bool,
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
            threads: Vec::new(),
            selected_thread: None,
            agent_list_state: ListState::default(),
            group_chat: GroupChatState::new(),
            agent_picker: None,
            chat_states: HashMap::new(),
            room_input: InputState::new(readonly),
            agent_input: InputState::new(readonly),
            focus: FocusManager::new(false),
            error_flash: None,
            flash_copied: None,
            panel_areas: PanelAreas::default(),
            input_metrics: [InputLayoutMetrics::default(); 2],
            delete_confirm: None,
            render_cache: RenderCache::default(),
            nav_stack: vec![NavLevel::Projects],
            projects: Vec::new(),
            selected_project: None,
            project_list_state: ListState::default(),
            conversations: Vec::new(),
            selected_conversation: None,
            conversation_list_state: ListState::default(),
            conversation_messages: Vec::new(),
            conversation_scroll_offset: 0,
            conversation_auto_scroll: true,
            conversation_max_scroll: 0,
            conversation_agent_sessions: Vec::new(),
            selected_agent_session: None,
            subagent_info: HashMap::new(),
            project_create_dialog: None,
        }
    }

    pub fn nav_level(&self) -> &NavLevel {
        self.nav_stack.last().unwrap_or(&NavLevel::Projects)
    }

    pub fn push_nav(&mut self, level: NavLevel) {
        self.nav_stack.push(level);
    }

    pub fn pop_nav(&mut self) {
        if self.nav_stack.len() > 1 {
            self.nav_stack.pop();
        }
    }

    pub fn current_thread_id(&self) -> Option<&str> {
        if matches!(
            self.nav_level(),
            NavLevel::Conversation { .. } | NavLevel::AgentDetail { .. }
        ) {
            if let Some(thread_id) = self.selected_flat_agent_session_thread_id() {
                return Some(thread_id);
            }
        }
        self.selected_thread
            .and_then(|i| self.threads.get(i))
            .map(|t| t.thread_id.as_str())
    }

    pub fn flat_agent_sessions(&self) -> Vec<FlatAgentSession> {
        flat_agent_sessions(&self.conversation_agent_sessions)
    }

    pub fn flat_agent_session_count(&self) -> usize {
        self.flat_agent_sessions().len()
    }

    pub fn selected_flat_agent_session_thread_id(&self) -> Option<&str> {
        let selected = self.selected_agent_session?;
        let source_index = self.flat_agent_sessions().get(selected)?.source_index;
        self.conversation_agent_sessions
            .get(source_index)
            .map(|session| session.thread_id.as_str())
    }

    pub fn current_thread_is_subagent(&self) -> bool {
        let Some(thread_id) = self.current_thread_id() else {
            return false;
        };
        self.threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)
            .is_some_and(|thread| thread.parent_thread_id.is_some())
            || self
                .conversation_agent_sessions
                .iter()
                .find(|session| session.thread_id == thread_id)
                .is_some_and(|session| session.parent_thread_id.is_some())
    }

    pub fn current_chat_mut(&mut self) -> Option<&mut ChatState> {
        let thread_id = self.current_thread_id()?.to_owned();
        self.chat_states.get_mut(&thread_id)
    }

    pub fn current_chat(&self) -> Option<&ChatState> {
        let thread_id = self.current_thread_id()?;
        self.chat_states.get(thread_id)
    }

    /// Returns the active chat alongside the shared render cache, using a split
    /// borrow so callers can mutate both in the same scope.
    pub fn current_chat_and_cache_mut(&mut self) -> Option<(&mut ChatState, &mut RenderCache)> {
        let thread_id = self.current_thread_id()?.to_owned();
        let chat = self.chat_states.get_mut(&thread_id)?;
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
                .filter(|thread| thread.parent_thread_id.is_none())
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

pub(crate) fn flat_agent_sessions(sessions: &[ThreadSummaryEntry]) -> Vec<FlatAgentSession> {
    let mut out = Vec::with_capacity(sessions.len());
    let mut seen = HashSet::new();
    for (index, session) in sessions.iter().enumerate() {
        if session.parent_thread_id.is_some() {
            continue;
        }
        push_flat_session(&mut out, &mut seen, index, session, 0);
        for (child_index, child) in sessions.iter().enumerate() {
            if child.parent_thread_id.as_deref() == Some(session.thread_id.as_str()) {
                push_flat_session(&mut out, &mut seen, child_index, child, 1);
            }
        }
    }
    for (index, session) in sessions.iter().enumerate() {
        if !seen.contains(&session.thread_id) {
            push_flat_session(&mut out, &mut seen, index, session, 0);
        }
    }
    out
}

fn push_flat_session(
    out: &mut Vec<FlatAgentSession>,
    seen: &mut HashSet<String>,
    source_index: usize,
    session: &ThreadSummaryEntry,
    depth: u8,
) {
    seen.insert(session.thread_id.clone());
    out.push(FlatAgentSession {
        source_index,
        thread_id: session.thread_id.clone(),
        agent: session.agent,
        parent_thread_id: session.parent_thread_id.clone(),
        depth,
    });
}

#[cfg(test)]
mod subagent_tests {
    use super::*;

    fn session(thread_id: &str, parent_thread_id: Option<&str>) -> ThreadSummaryEntry {
        ThreadSummaryEntry {
            thread_id: thread_id.into(),
            agent: AgentName::Codex,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            parent_thread_id: parent_thread_id.map(str::to_string),
        }
    }

    #[test]
    fn flat_agent_sessions_groups_children_under_parent() {
        let flat = flat_agent_sessions(&[
            session("parent-a", None),
            session("parent-b", None),
            session("sub-a", Some("parent-a")),
            session("orphan", Some("missing")),
        ]);

        assert_eq!(
            flat.iter()
                .map(|entry| (entry.thread_id.as_str(), entry.depth))
                .collect::<Vec<_>>(),
            vec![
                ("parent-a", 0),
                ("sub-a", 1),
                ("parent-b", 0),
                ("orphan", 0),
            ]
        );
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
    let agent_input_active =
        matches!(state.nav_level(), NavLevel::AgentDetail { .. }) && state.focus.is(PaneId::Input);
    state.room_input.focused = state.focus.is(PaneId::Input) && !agent_input_active;
    state.agent_input.focused = agent_input_active;

    match state.nav_level() {
        NavLevel::Projects => {
            render_projects_level(f, state);
        }
        NavLevel::Conversations { .. } => {
            render_conversations_level(f, state);
        }
        NavLevel::Conversation { .. } => {
            render_conversation_level(f, state);
        }
        NavLevel::AgentDetail { .. } => {
            render_agent_detail_level(f, state);
        }
    }

    if let Some(dialog) = state.project_create_dialog.as_ref() {
        project_create_dialog::render(f, f.area(), dialog);
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

fn render_conversations_level(f: &mut Frame, state: &mut UiState) {
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
            Box::new(conversation_list::ConversationListRenderable::new(
                project_name,
                &state.conversations,
                state.selected_conversation,
                &mut state.conversation_list_state,
                true,
            )),
            Box::new(conversation_list::ConversationSidebarRenderable::new(
                &state.conversations,
                state.selected_conversation,
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

fn render_conversation_level(f: &mut Frame, state: &mut UiState) {
    let mention_candidates = state.room_agent_mention_candidates();
    let flash_active = state.is_flash_copied_active();
    let root = f.area();
    let input_height = input_bar::required_height(&state.room_input, root.width);
    let [_, middle, input_area] = root_sections(root, input_height);
    let areas = Row::areas_for(middle, &[80, 20]);

    state.panel_areas = PanelAreas {
        room_list: Rect::default(),
        room_chat: areas[0],
        agent_list: areas[1],
        agent_chat: Rect::default(),
        room_input: input_area,
        agent_input: Rect::default(),
    };
    state.input_metrics = [InputLayoutMetrics::default(); 2];

    let title = current_conversation_title(state);
    let main_row = Row::new(
        vec![
            Box::new(conversation_detail::ConversationMessagesRenderable::new(
                title,
                &state.conversation_messages,
                &mut state.conversation_scroll_offset,
                &mut state.conversation_auto_scroll,
                &mut state.conversation_max_scroll,
                false,
            )),
            Box::new(conversation_detail::AgentSessionListRenderable::new(
                &state.conversation_agent_sessions,
                state.selected_agent_session,
                &mut state.agent_list_state,
                state.focus.is(PaneId::Sidebar),
            )),
        ],
        vec![80, 20],
    );

    let input = input_bar::InputBarRenderable::new(
        room_input_title(&state.room_input),
        "Type @agent to run inside this conversation",
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

fn render_agent_detail_level(f: &mut Frame, state: &mut UiState) {
    let pending_agent_request = state
        .current_chat()
        .and_then(ChatState::active_pending_request)
        .is_some();
    let is_subagent = state.current_thread_is_subagent();
    let selected_thread_id = state.current_thread_id().map(str::to_owned);
    let agent_input_title = if is_subagent {
        "Subagent · read-only"
    } else {
        agent_input_title(pending_agent_request, state.agent_input.multiline)
    };
    let flash_active = state.is_flash_copied_active();
    let agent_input_hint = if is_subagent {
        "Subagent transcript is read-only"
    } else if pending_agent_request {
        "Reply to the pending agent request"
    } else {
        "Talk directly to the selected agent"
    };
    let root = f.area();
    let body_area = Rect {
        x: root.x,
        y: root.y.saturating_add(1),
        width: root.width,
        height: root.height.saturating_sub(1),
    };
    let columns = Row::areas_for(body_area, &[80, 20]);
    let agent_input_height =
        input_bar::required_height(&state.agent_input, columns[0].width).min(columns[0].height);
    let agent_chat_area = Rect {
        x: columns[0].x,
        y: columns[0].y,
        width: columns[0].width,
        height: columns[0].height.saturating_sub(agent_input_height.max(1)),
    };
    let agent_input_area = Rect {
        x: columns[0].x,
        y: columns[0].y.saturating_add(agent_chat_area.height),
        width: columns[0].width,
        height: agent_input_height.max(1),
    };

    state.panel_areas = PanelAreas {
        room_list: Rect::default(),
        room_chat: Rect::default(),
        agent_list: columns[1],
        agent_chat: agent_chat_area,
        room_input: Rect::default(),
        agent_input: agent_input_area,
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

    let [_, agent_metrics] = &mut state.input_metrics;
    let left_column = Column::with_fill(
        vec![
            Box::new(AgentChatRenderable::new(
                agent_chat_target,
                state.focus.is(PaneId::MainChat),
            )),
            Box::new(input_bar::InputBarRenderable::new(
                agent_input_title,
                agent_input_hint,
                &state.agent_input,
                &[],
                agent_metrics,
            )),
        ],
        0,
    );
    let main_row = Row::new(
        vec![
            Box::new(left_column),
            Box::new(conversation_detail::AgentSessionListRenderable::new(
                &state.conversation_agent_sessions,
                state.selected_agent_session,
                &mut state.agent_list_state,
                state.focus.is(PaneId::Sidebar),
            )),
        ],
        vec![80, 20],
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
    tree.render(f, root);
    if let Some(position) = tree.cursor_pos(root) {
        f.set_cursor_position(position);
    }
}

fn current_conversation_title(state: &UiState) -> String {
    let nav_conversation_id = state.nav_level().conversation_id();
    state
        .selected_conversation
        .and_then(|idx| state.conversations.get(idx))
        .filter(|conversation| {
            nav_conversation_id
                .map(|id| id == conversation.conversation_id)
                .unwrap_or(true)
        })
        .map(|conversation| format!("Conversation: {}", conversation.title))
        .unwrap_or_else(|| "Conversation".to_owned())
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
