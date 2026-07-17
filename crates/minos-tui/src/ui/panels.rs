use crate::backend::{ConversationMessageEntry, ThreadSummaryEntry};
use crate::nav::NavLevel;
use crate::translation::ChatState;
use crate::ui::conversation_view::ConversationChatRenderCache;
use crate::ui::delete_confirm::DeleteConfirmState;
use crate::ui::input_bar::{InputLayoutMetrics, InputState};
use crate::ui::list_panel::ListPanel;
use crate::ui::{ProjectCreateDialogState, SubagentInfo, ThreadEntry};
use std::collections::HashMap;

pub struct NavPanel {
    pub stack: Vec<NavLevel>,
}

impl NavPanel {
    pub fn new() -> Self {
        Self {
            stack: vec![NavLevel::Projects],
        }
    }

    pub fn level(&self) -> &NavLevel {
        self.stack.last().unwrap_or(&NavLevel::Projects)
    }

    pub fn push(&mut self, level: NavLevel) {
        self.stack.push(level);
    }

    pub fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
}

impl Default for NavPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Current conversation timeline, agent-session sidebar, and conversation chat cache.
///
/// `agent_sessions.selected` is the **flat** sidebar index (parent + subagent rows),
/// not a raw index into `agent_sessions.items`. Use flat helpers on `UiState`.
pub struct ConversationPanel {
    pub messages: Vec<ConversationMessageEntry>,
    /// Bumped on any messages mutation so render cache validity is O(1).
    pub messages_revision: u64,
    pub scroll_offset: u32,
    pub auto_scroll: bool,
    pub max_scroll: u32,
    pub agent_sessions: ListPanel<ThreadSummaryEntry>,
    pub subagent_info: HashMap<String, SubagentInfo>,
    pub chat_cache: ConversationChatRenderCache,
    /// Mouse drag text selection over the conversation timeline (agent-chat style).
    pub selection: Option<crate::translation::ChatSelection>,
}

impl ConversationPanel {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            messages_revision: 0,
            scroll_offset: 0,
            auto_scroll: true,
            max_scroll: 0,
            agent_sessions: ListPanel::new(),
            subagent_info: HashMap::new(),
            chat_cache: ConversationChatRenderCache::default(),
            selection: None,
        }
    }

    pub fn set_messages(&mut self, messages: Vec<ConversationMessageEntry>) {
        self.messages = messages;
        self.bump_messages_revision();
        self.clear_selection();
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.bump_messages_revision();
        self.clear_selection();
    }

    fn bump_messages_revision(&mut self) {
        self.messages_revision = self.messages_revision.wrapping_add(1);
    }

    pub fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
        self.auto_scroll = true;
        self.max_scroll = 0;
    }

    pub fn active_scroll(&self) -> u32 {
        self.scroll_offset
    }

    pub fn begin_selection(&mut self, point: crate::translation::ChatSelectionPoint) {
        self.selection = Some(crate::translation::ChatSelection {
            anchor: point,
            focus: point,
        });
    }

    pub fn update_selection(&mut self, point: crate::translation::ChatSelectionPoint) {
        if let Some(selection) = self.selection.as_mut() {
            selection.focus = point;
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }
}

impl Default for ConversationPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy / hydration thread list + per-thread chat projection.
///
/// Field name `list` avoids awkward paths like `ui.threads.threads`.
pub struct ThreadPanel {
    pub list: ListPanel<ThreadEntry>,
    pub chat_states: HashMap<String, ChatState>,
}

impl ThreadPanel {
    pub fn new() -> Self {
        Self {
            list: ListPanel::new(),
            chat_states: HashMap::new(),
        }
    }
}

impl Default for ThreadPanel {
    fn default() -> Self {
        Self::new()
    }
}

pub struct InputsPanel {
    pub conversation: InputState,
    pub agent: InputState,
    pub metrics: [InputLayoutMetrics; 2],
}

impl InputsPanel {
    pub fn new(readonly: bool) -> Self {
        Self {
            conversation: InputState::new(readonly),
            agent: InputState::new(readonly),
            metrics: [InputLayoutMetrics::default(); 2],
        }
    }
}

pub struct OverlaysPanel {
    pub delete_confirm: Option<DeleteConfirmState>,
    pub project_create: Option<ProjectCreateDialogState>,
    pub approval_selected: usize,
}

impl OverlaysPanel {
    pub fn new() -> Self {
        Self {
            delete_confirm: None,
            project_create: None,
            approval_selected: 0,
        }
    }
}

impl Default for OverlaysPanel {
    fn default() -> Self {
        Self::new()
    }
}
