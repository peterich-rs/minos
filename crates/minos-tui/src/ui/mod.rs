pub mod approval_overlay;
pub mod chat;
pub mod conversation_detail;
pub mod conversation_list;
pub mod conversation_view;
pub mod delete_confirm;
pub mod input_bar;
pub mod list_panel;
pub mod panels;
pub mod project_create_dialog;
pub mod project_list;
pub mod status_bar;
pub mod stream_holdback;
pub mod theme;

use crate::agent_route::short_session_id;
use crate::backend::{ConversationEntry, ProjectEntry, SessionSummaryEntry};
use crate::focus::{FocusManager, PaneId};
use crate::nav::NavLevel;
use crate::render::{Column, Renderable, Row};
use crate::translation::ChatState;
use crate::ui::chat::RenderCache;
use crate::ui::chat::{AgentChatRenderable, AgentChatTarget};
pub use crate::ui::delete_confirm::DeleteConfirmState;
use crate::ui::input_bar::{AgentMentionCandidate, InputState};
pub use crate::ui::list_panel::ListPanel;
pub use crate::ui::panels::{
    ConversationPanel, InputsPanel, NavPanel, OverlaysPanel, SessionPanel,
};
use crate::ui::status_bar::StatusBarState;
use minos_agent_runtime::SessionState;
use minos_domain::AgentName;
use minos_ui_protocol::SubagentStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

pub struct SessionEntry {
    pub session_id: String,
    pub agent: AgentName,
    pub workspace: PathBuf,
    pub state: SessionState,
    pub parent_session_id: Option<String>,
}

/// Flat sidebar row: indexes into `conversation.agent_sessions.items` without cloning IDs.
#[derive(Clone, Copy, Debug)]
pub struct FlatAgentSession {
    pub source_index: usize,
    pub depth: u8,
}

/// Metadata for subagent cards. Written from ingest events; several fields are
/// reserved for AgentDetail presentation beyond the current status glyph.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct SubagentInfo {
    pub parent_session_id: String,
    pub tool_call_id: String,
    pub agent: AgentName,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub title: Option<String>,
    pub status: SubagentStatus,
}

pub struct UiState {
    pub nav: NavPanel,
    pub projects: ListPanel<ProjectEntry>,
    pub conversations: ListPanel<ConversationEntry>,
    pub conversation: ConversationPanel,
    pub session_panel: SessionPanel,
    pub inputs: InputsPanel,
    pub overlays: OverlaysPanel,
    pub focus: FocusManager,
    pub panel_areas: PanelAreas,
    pub status: StatusBarState,
    /// Host agent profiles cached from `list_agent_profiles` (mention + route).
    pub agent_profiles: Vec<minos_protocol::AgentProfileSummary>,
    pub error_flash: Option<(String, Instant)>,
    pub flash_copied: Option<Instant>,
    /// Agent chat render cache — top-level for split borrow with `session_panel.chat_states`.
    pub render_cache: RenderCache,
    /// Set during paint when viewport materialization still has work; main loop
    /// schedules another frame so overscan can continue without blocking input.
    pub needs_render_followup: bool,
    /// Cached sidebar "recent files" so scroll frames skip scanning every chat.
    recent_files_cache: Option<(u64, HashMap<String, Vec<String>>)>,
}

#[derive(Debug, Clone)]
pub struct ProjectCreateDialogState {
    pub name: String,
    pub path: String,
    pub editing_name: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PanelAreas {
    pub main_list: Rect,
    pub conversation_chat: Rect,
    pub agent_list: Rect,
    pub agent_chat: Rect,
    pub conversation_input: Rect,
    pub agent_input: Rect,
}

impl UiState {
    pub fn new(readonly: bool) -> Self {
        Self {
            nav: NavPanel::new(),
            projects: ListPanel::new(),
            conversations: ListPanel::new(),
            conversation: ConversationPanel::new(),
            session_panel: SessionPanel::new(),
            inputs: InputsPanel::new(readonly),
            overlays: OverlaysPanel::new(),
            focus: FocusManager::new(false),
            panel_areas: PanelAreas::default(),
            status: StatusBarState::new(),
            agent_profiles: Vec::new(),
            error_flash: None,
            flash_copied: None,
            render_cache: RenderCache::default(),
            needs_render_followup: false,
            recent_files_cache: None,
        }
    }

    /// Profiles as routing/mention structs (desktop `MentionProfile` parity).
    pub fn mention_profiles(&self) -> Vec<crate::agent_route::MentionProfile> {
        self.agent_profiles
            .iter()
            .map(|p| crate::agent_route::MentionProfile {
                id: p.id.clone(),
                name: p.name.clone(),
                runtime_agent: p.runtime_agent,
                updated_at_ms: p.updated_at_ms,
            })
            .collect()
    }

    pub fn take_render_followup(&mut self) -> bool {
        let pending = self.needs_render_followup || self.render_cache.needs_followup_frame();
        self.needs_render_followup = false;
        pending
    }

    /// Sidebar recent-files map, rebuilt only when chat versions change.
    pub fn recent_files_cached(&mut self) -> &HashMap<String, Vec<String>> {
        let fingerprint = recent_files_fingerprint(
            &self.session_panel.chat_states,
            &self.conversation.agent_sessions.items,
        );
        let needs = self
            .recent_files_cache
            .as_ref()
            .is_none_or(|(fp, _)| *fp != fingerprint);
        if needs {
            let map = compute_recent_files(
                &self.session_panel.chat_states,
                &self.conversation.agent_sessions.items,
            );
            self.recent_files_cache = Some((fingerprint, map));
        }
        &self.recent_files_cache.as_ref().unwrap().1
    }

    pub fn nav_level(&self) -> &NavLevel {
        self.nav.level()
    }

    pub fn push_nav(&mut self, level: NavLevel) {
        self.nav.push(level);
    }

    pub fn pop_nav(&mut self) {
        self.nav.pop();
    }

    pub fn current_session_id(&self) -> Option<&str> {
        if matches!(
            self.nav_level(),
            NavLevel::Conversation { .. } | NavLevel::AgentDetail { .. }
        ) {
            if let Some(session_id) = self.selected_flat_agent_session_session_id() {
                return Some(session_id);
            }
        }
        self.session_panel
            .list
            .selected
            .and_then(|i| self.session_panel.list.items.get(i))
            .map(|t| t.session_id.as_str())
    }

    pub fn flat_agent_sessions(&self) -> Vec<FlatAgentSession> {
        flat_agent_sessions(&self.conversation.agent_sessions.items)
    }

    pub fn flat_agent_session_count(&self) -> usize {
        flat_agent_session_count(&self.conversation.agent_sessions.items)
    }

    pub fn selected_flat_agent_session_session_id(&self) -> Option<&str> {
        let selected = self.conversation.agent_sessions.selected?;
        let source_index =
            flat_agent_session_source_index(&self.conversation.agent_sessions.items, selected)?;
        self.conversation
            .agent_sessions
            .items
            .get(source_index)
            .map(|session| session.session_id.as_str())
    }

    pub fn flat_session_index_for_thread(&self, session_id: &str) -> Option<usize> {
        self.flat_agent_sessions().into_iter().position(|flat| {
            self.conversation
                .agent_sessions
                .items
                .get(flat.source_index)
                .is_some_and(|session| session.session_id == session_id)
        })
    }

    pub fn flat_session_entry(&self, flat_index: usize) -> Option<&SessionSummaryEntry> {
        let source_index =
            flat_agent_session_source_index(&self.conversation.agent_sessions.items, flat_index)?;
        self.conversation.agent_sessions.items.get(source_index)
    }

    pub fn current_thread_is_subagent(&self) -> bool {
        let Some(session_id) = self.current_session_id() else {
            return false;
        };
        self.session_panel
            .list
            .items
            .iter()
            .find(|thread| thread.session_id == session_id)
            .is_some_and(|thread| thread.parent_session_id.is_some())
            || self
                .conversation
                .agent_sessions
                .items
                .iter()
                .find(|session| session.session_id == session_id)
                .is_some_and(|session| session.parent_session_id.is_some())
    }

    pub fn current_chat_mut(&mut self) -> Option<&mut ChatState> {
        let session_id = self.current_session_id()?.to_owned();
        self.session_panel.chat_states.get_mut(&session_id)
    }

    pub fn current_chat(&self) -> Option<&ChatState> {
        let session_id = self.current_session_id()?;
        self.session_panel.chat_states.get(session_id)
    }

    pub fn active_approval_request(&self) -> Option<&crate::translation::PendingAgentRequest> {
        let request = self.current_chat()?.active_pending_request()?;
        approval_overlay::is_selectable(request).then_some(request)
    }

    /// Returns the active chat alongside the shared render cache, using a split
    /// borrow so callers can mutate both in the same scope.
    pub fn current_chat_and_cache_mut(&mut self) -> Option<(&mut ChatState, &mut RenderCache)> {
        let session_id = self.current_session_id()?.to_owned();
        let chat = self.session_panel.chat_states.get_mut(&session_id)?;
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
            .is_some_and(|instant| instant.elapsed().as_millis() < 2_000)
    }

    /// How long error flash remains visible.
    pub const ERROR_FLASH_TTL: std::time::Duration = std::time::Duration::from_secs(3);
    /// How long "copied" flash remains visible.
    pub const COPIED_FLASH_TTL: std::time::Duration = std::time::Duration::from_secs(2);

    pub fn conversation_agent_mention_candidates(&self) -> Vec<AgentMentionCandidate> {
        // Order matches desktop: runtimes → profiles → continue sessions.
        let mut candidates: Vec<AgentMentionCandidate> = self
            .status
            .agents
            .iter()
            .map(|agent| AgentMentionCandidate::installed(agent.name, agent.status.clone()))
            .collect();

        let mention_profiles = self.mention_profiles();
        candidates.extend(mention_profiles.iter().map(|p| {
            // Insert form is `@Name ` or `@p/<id> `; picker stores token without `@`/space.
            let insert = crate::agent_route::profile_mention_insert(p, &mention_profiles);
            let token = insert.trim_start_matches('@').trim_end().to_owned();
            AgentMentionCandidate::profile(token, p.runtime_agent, p.id.clone())
        }));

        if self.nav_level().conversation_id().is_some() {
            candidates.extend(
                self.conversation
                    .agent_sessions
                    .items
                    .iter()
                    .filter(|session| session.parent_session_id.is_none())
                    .filter(|session| !matches!(session.state, SessionState::Closed { .. }))
                    .map(|session| {
                        AgentMentionCandidate::existing(
                            session.agent,
                            session.session_id.clone(),
                            short_session_id(&session.session_id).to_owned(),
                        )
                    }),
            );
        }
        candidates
    }
}

pub(crate) fn flat_agent_sessions(sessions: &[SessionSummaryEntry]) -> Vec<FlatAgentSession> {
    let mut out = Vec::with_capacity(sessions.len());
    let mut seen = HashSet::new();
    for (index, session) in sessions.iter().enumerate() {
        if session.parent_session_id.is_some() {
            continue;
        }
        push_flat_session(&mut out, &mut seen, index, 0);
        for (child_index, child) in sessions.iter().enumerate() {
            if child.parent_session_id.as_deref() == Some(session.session_id.as_str()) {
                push_flat_session(&mut out, &mut seen, child_index, 1);
            }
        }
    }
    for index in 0..sessions.len() {
        if !seen.contains(&index) {
            push_flat_session(&mut out, &mut seen, index, 0);
        }
    }
    out
}

pub(crate) fn flat_agent_session_count(sessions: &[SessionSummaryEntry]) -> usize {
    flat_agent_sessions(sessions).len()
}

pub(crate) fn flat_agent_session_source_index(
    sessions: &[SessionSummaryEntry],
    flat_index: usize,
) -> Option<usize> {
    flat_agent_sessions(sessions)
        .get(flat_index)
        .map(|entry| entry.source_index)
}

fn push_flat_session(
    out: &mut Vec<FlatAgentSession>,
    seen: &mut HashSet<usize>,
    source_index: usize,
    depth: u8,
) {
    if !seen.insert(source_index) {
        return;
    }
    out.push(FlatAgentSession {
        source_index,
        depth,
    });
}

pub fn render_ui(f: &mut Frame, state: &mut UiState) {
    state.needs_render_followup = false;
    let agent_input_active =
        matches!(state.nav_level(), NavLevel::AgentDetail { .. }) && state.focus.is(PaneId::Input);
    state.inputs.conversation.focused = state.focus.is(PaneId::Input) && !agent_input_active;
    state.inputs.agent.focused = agent_input_active;

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

    if let Some(dialog) = state.overlays.project_create.as_ref() {
        project_create_dialog::render(f, f.area(), dialog);
    }
    if let Some(confirm) = state.overlays.delete_confirm.as_ref() {
        let mut overlay = delete_confirm::DeleteConfirmRenderable::new(confirm);
        overlay.render(f, f.area());
    }

    if state.render_cache.needs_followup_frame() {
        state.needs_render_followup = true;
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
        main_list: main,
        conversation_chat: Rect::default(),
        agent_list: sidebar,
        agent_chat: Rect::default(),
        conversation_input: Rect::default(),
        agent_input: Rect::default(),
    };
    state.inputs.metrics = [crate::ui::input_bar::InputLayoutMetrics::default(); 2];

    let main_row = Row::new(
        vec![
            Box::new(project_list::ProjectListRenderable::new(
                &state.projects.items,
                state.projects.selected,
                &mut state.projects.list_state,
                true,
            )),
            Box::new(project_list::ProjectSidebarRenderable::new(
                &state.projects.items,
                state.projects.selected,
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
    let mention_candidates = state.conversation_agent_mention_candidates();
    let flash_active = state.is_flash_copied_active();
    let root = f.area();
    let input_height = input_bar::required_height(&state.inputs.conversation, root.width);

    let [_, middle, input_area] = root_sections(root, input_height);
    let areas = Row::areas_for(middle, &[78, 22]);
    let main = areas[0];
    let sidebar = areas[1];

    state.panel_areas = PanelAreas {
        main_list: main,
        conversation_chat: Rect::default(),
        agent_list: sidebar,
        agent_chat: Rect::default(),
        conversation_input: input_area,
        agent_input: Rect::default(),
    };
    state.inputs.metrics = [crate::ui::input_bar::InputLayoutMetrics::default(); 2];

    let project_name = state
        .projects
        .selected
        .and_then(|idx| state.projects.items.get(idx))
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let main_row = Row::new(
        vec![
            Box::new(conversation_list::ConversationListRenderable::new(
                project_name,
                &state.conversations.items,
                state.conversations.selected,
                &mut state.conversations.list_state,
                true,
            )),
            Box::new(conversation_list::ConversationSidebarRenderable::new(
                &state.conversations.items,
                state.conversations.selected,
            )),
        ],
        vec![78, 22],
    );

    let input = input_bar::InputBarRenderable::new(
        conversation_input_title(&state.inputs.conversation),
        "Type a message to start a new conversation...",
        &state.inputs.conversation,
        mention_candidates.as_slice(),
        &mut state.inputs.metrics[0],
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
    let mention_candidates = state.conversation_agent_mention_candidates();
    let flash_active = state.is_flash_copied_active();
    let root = f.area();
    let input_height = input_bar::required_height(&state.inputs.conversation, root.width);
    let [_, middle, input_area] = root_sections(root, input_height);
    let areas = Row::areas_for(middle, &[80, 20]);

    state.panel_areas = PanelAreas {
        main_list: Rect::default(),
        conversation_chat: areas[0],
        agent_list: areas[1],
        agent_chat: Rect::default(),
        conversation_input: input_area,
        agent_input: Rect::default(),
    };
    state.inputs.metrics = [crate::ui::input_bar::InputLayoutMetrics::default(); 2];

    let title = current_conversation_title(state);
    let recent_files = state.recent_files_cached().clone();
    let selected_agent_session = state.conversation.agent_sessions.selected;
    let messages_revision = state.conversation.messages_revision;
    // Clone selection so we can pass a reference without holding the whole panel borrow.
    let conversation_selection = state.conversation.selection.clone();
    let (conversation_messages, scroll_offset, auto_scroll, max_scroll, cache) = (
        &state.conversation.messages,
        &mut state.conversation.scroll_offset,
        &mut state.conversation.auto_scroll,
        &mut state.conversation.max_scroll,
        &mut state.conversation.chat_cache,
    );
    let (sessions, agent_sessions, agent_list_state) = (
        &state.session_panel.list.items,
        &state.conversation.agent_sessions.items,
        &mut state.conversation.agent_sessions.list_state,
    );
    let main_row = Row::new(
        vec![
            Box::new(conversation_view::ConversationChatRenderable::new(
                title,
                conversation_messages,
                messages_revision,
                scroll_offset,
                auto_scroll,
                max_scroll,
                cache,
                conversation_selection.as_ref(),
                state.focus.is(PaneId::MainChat),
            )),
            Box::new(conversation_detail::AgentSessionListRenderable::new(
                agent_sessions,
                sessions,
                &recent_files,
                selected_agent_session,
                agent_list_state,
                state.focus.is(PaneId::Sidebar),
            )),
        ],
        vec![80, 20],
    );

    let input = input_bar::InputBarRenderable::new(
        conversation_input_title(&state.inputs.conversation),
        "Type @agent to run inside this conversation",
        &state.inputs.conversation,
        mention_candidates.as_slice(),
        &mut state.inputs.metrics[0],
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
    let selected_session_id = state.current_session_id().map(str::to_owned);
    let approval_request = state.active_approval_request().cloned();
    let approval_request_count = state
        .current_chat()
        .map(|chat| chat.pending_requests.len())
        .unwrap_or(0);
    let agent_input_title = if is_subagent {
        "Subagent · read-only"
    } else {
        agent_input_title(pending_agent_request, state.inputs.agent.multiline)
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
        input_bar::required_height(&state.inputs.agent, columns[0].width).min(columns[0].height);
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
        main_list: Rect::default(),
        conversation_chat: Rect::default(),
        agent_list: columns[1],
        agent_chat: agent_chat_area,
        conversation_input: Rect::default(),
        agent_input: agent_input_area,
    };

    let recent_files = state.recent_files_cached().clone();
    let selected_agent_session = state.conversation.agent_sessions.selected;

    let agent_chat_target = if let Some(session_id) = selected_session_id.as_deref() {
        if let Some(chat) = state.session_panel.chat_states.get_mut(session_id) {
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

    let [_, agent_metrics] = &mut state.inputs.metrics;
    let left_column = Column::with_fill(
        vec![
            Box::new(AgentChatRenderable::new(
                agent_chat_target,
                state.focus.is(PaneId::MainChat),
            )),
            Box::new(input_bar::InputBarRenderable::new(
                agent_input_title,
                agent_input_hint,
                &state.inputs.agent,
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
                &state.conversation.agent_sessions.items,
                &state.session_panel.list.items,
                &recent_files,
                selected_agent_session,
                &mut state.conversation.agent_sessions.list_state,
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
    if let Some(request) = approval_request.as_ref() {
        approval_overlay::render(
            f,
            agent_chat_area,
            request,
            state.overlays.approval_selected,
            approval_request_count,
        );
    }
    if let Some(position) = tree.cursor_pos(root) {
        f.set_cursor_position(position);
    }
}

fn current_conversation_title(state: &UiState) -> String {
    let nav_conversation_id = state.nav_level().conversation_id();
    state
        .conversations
        .selected
        .and_then(|idx| state.conversations.items.get(idx))
        .filter(|conversation| {
            nav_conversation_id
                .map(|id| id == conversation.conversation_id)
                .unwrap_or(true)
        })
        .map(|conversation| format!("Conversation: {}", conversation.title))
        .unwrap_or_else(|| "Conversation".to_owned())
}

fn recent_files_fingerprint(
    chat_states: &HashMap<String, ChatState>,
    sessions: &[SessionSummaryEntry],
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    sessions.len().hash(&mut h);
    for session in sessions {
        session.session_id.hash(&mut h);
        if let Some(chat) = chat_states.get(&session.session_id) {
            chat.version.hash(&mut h);
            chat.structure_version.hash(&mut h);
            chat.items.len().hash(&mut h);
        }
    }
    h.finish()
}

fn compute_recent_files(
    chat_states: &HashMap<String, ChatState>,
    sessions: &[SessionSummaryEntry],
) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for session in sessions {
        let Some(chat) = chat_states.get(&session.session_id) else {
            continue;
        };
        let mut files = Vec::new();
        for item in chat.items.iter().rev() {
            if let crate::translation::ChatItem::ToolCall {
                name, args_summary, ..
            } = item
            {
                if let Some(path) = conversation_detail::extract_file_path(name, args_summary) {
                    if !files.contains(&path) {
                        files.push(path);
                    }
                    if files.len() >= 2 {
                        break;
                    }
                }
            }
        }
        if !files.is_empty() {
            out.insert(session.session_id.clone(), files);
        }
    }
    out
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

fn conversation_input_title(state: &InputState) -> &'static str {
    if state.multiline {
        "Conversation Input [multi]"
    } else {
        "Conversation Input"
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

#[cfg(test)]
mod subagent_tests {
    use super::*;
    use crate::ui::input_bar::AgentMentionCandidateKind;

    fn session(session_id: &str, parent_session_id: Option<&str>) -> SessionSummaryEntry {
        SessionSummaryEntry {
            session_id: session_id.into(),
            agent: AgentName::Codex,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            parent_session_id: parent_session_id.map(str::to_string),
            state: SessionState::Idle,
            needs_continue: false,
        }
    }

    fn existing_session_ids(state: &UiState) -> Vec<String> {
        state
            .conversation_agent_mention_candidates()
            .into_iter()
            .filter_map(|candidate| match candidate.kind {
                AgentMentionCandidateKind::Existing { session_id } => Some(session_id),
                AgentMentionCandidateKind::Installed { .. }
                | AgentMentionCandidateKind::Profile { .. } => None,
            })
            .collect()
    }

    #[test]
    fn flat_agent_sessions_groups_children_under_parent() {
        let sessions = [
            session("parent-a", None),
            session("parent-b", None),
            session("sub-a", Some("parent-a")),
            session("orphan", Some("missing")),
        ];
        let flat = flat_agent_sessions(&sessions);

        assert_eq!(
            flat.iter()
                .map(|entry| {
                    (
                        sessions[entry.source_index].session_id.as_str(),
                        entry.depth,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("parent-a", 0),
                ("sub-a", 1),
                ("parent-b", 0),
                ("orphan", 0),
            ]
        );
    }

    #[test]
    fn conversation_mentions_in_conversation_use_conversation_sessions_only() {
        let mut state = UiState::new(false);
        state.session_panel.list.items.push(SessionEntry {
            session_id: "global-opencode-1234".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp"),
            state: SessionState::Idle,
            parent_session_id: None,
        });
        state.conversation.agent_sessions.items = vec![
            SessionSummaryEntry {
                session_id: "conv-opencode-5678".into(),
                agent: AgentName::Opencode,
                title: None,
                first_ts_ms: 0,
                last_ts_ms: 0,
                message_count: 0,
                ended_at_ms: None,
                parent_session_id: None,
                state: SessionState::Idle,
                needs_continue: false,
            },
            SessionSummaryEntry {
                session_id: "conv-closed-9999".into(),
                agent: AgentName::Codex,
                title: None,
                first_ts_ms: 0,
                last_ts_ms: 0,
                message_count: 0,
                ended_at_ms: None,
                parent_session_id: None,
                state: SessionState::Closed {
                    reason: minos_agent_runtime::CloseReason::UserClose,
                },
                needs_continue: false,
            },
            SessionSummaryEntry {
                session_id: "conv-child-0000".into(),
                agent: AgentName::Gemini,
                title: None,
                first_ts_ms: 0,
                last_ts_ms: 0,
                message_count: 0,
                ended_at_ms: None,
                parent_session_id: Some("conv-opencode-5678".into()),
                state: SessionState::Idle,
                needs_continue: false,
            },
        ];
        state.nav.stack = vec![
            NavLevel::Projects,
            NavLevel::Conversations {
                project_id: "p".into(),
            },
            NavLevel::Conversation {
                project_id: "p".into(),
                conversation_id: "c".into(),
            },
        ];

        assert_eq!(
            existing_session_ids(&state),
            vec!["conv-opencode-5678".to_owned()]
        );
    }

    #[test]
    fn conversation_mentions_outside_conversation_hide_existing_threads() {
        let mut state = UiState::new(false);
        state.session_panel.list.items.push(SessionEntry {
            session_id: "global-codex-1234".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: SessionState::Idle,
            parent_session_id: None,
        });
        state.conversation.agent_sessions.items = vec![SessionSummaryEntry {
            session_id: "conv-codex-5678".into(),
            agent: AgentName::Codex,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            parent_session_id: None,
            state: SessionState::Idle,
            needs_continue: false,
        }];
        state.nav.stack = vec![
            NavLevel::Projects,
            NavLevel::Conversations {
                project_id: "p".into(),
            },
        ];

        assert_eq!(existing_session_ids(&state), Vec::<String>::new());
    }
}
