use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use minos_agent_runtime::{ManagerEvent, ThreadState};
use minos_domain::{AgentName, AgentStatus};
use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};
use tracing::debug;

use crate::backend::AgentBackend;
use crate::event::AppEvent;
use crate::group_chat::GroupChatStore;
use crate::translation::{ChatSelectionPoint, ChatState};
use crate::ui::{AgentPickerState, DeleteConfirmState, Focus, ThreadEntry, UiState};

pub struct App {
    backend: Arc<dyn AgentBackend>,
    ui: UiState,
    should_quit: bool,
    workspace: PathBuf,
    hydrated_threads: HashSet<String>,
    group_chat_store: GroupChatStore,
    recorded_agent_results: HashMap<String, String>,
}

impl App {
    pub fn new(backend: Arc<dyn AgentBackend>, readonly: bool, workspace: PathBuf) -> Self {
        let group_chat_store = default_group_chat_store();
        Self::with_group_chat_store(backend, readonly, workspace, group_chat_store)
    }

    fn with_group_chat_store(
        backend: Arc<dyn AgentBackend>,
        readonly: bool,
        workspace: PathBuf,
        group_chat_store: GroupChatStore,
    ) -> Self {
        Self {
            backend,
            ui: UiState::new(readonly),
            should_quit: false,
            workspace,
            hydrated_threads: HashSet::new(),
            group_chat_store,
            recorded_agent_results: HashMap::new(),
        }
    }

    pub async fn init(&mut self) -> anyhow::Result<()> {
        self.load_group_chat_history();
        let agents = self.backend.detect_clis().await?;
        self.ui.status.update_agents(agents);
        self.sync_input_agent_picker();
        if matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            self.hydrate_daemon_threads().await;
        }
        Ok(())
    }

    pub async fn handle_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Ingest(ingest) => {
                if let Some(chat) = self.ui.chat_states.get_mut(&ingest.thread_id) {
                    let events = chat.translation_state.translate(&ingest.payload);
                    debug!(
                        agent = %ingest.agent.bin_name(),
                        thread_id = %ingest.thread_id,
                        event_count = events.len(),
                        "translated ingest payload"
                    );
                    chat.apply_ui_events(events);
                    let thread_id = ingest.thread_id.clone();
                    self.record_agent_group_result_if_done(&thread_id);
                    return true;
                }
                debug!(
                    agent = %ingest.agent.bin_name(),
                    thread_id = %ingest.thread_id,
                    "dropping ingest event because no chat state exists"
                );
                false
            }
            AppEvent::ManagerEvent(event) => self.handle_manager_event(event).await,
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Mouse(mouse) => self.handle_mouse(mouse).await,
            AppEvent::Tick => {
                if let Some((_, instant)) = self.ui.error_flash {
                    if instant.elapsed() > Duration::from_secs(3) {
                        self.ui.error_flash = None;
                        return true;
                    }
                }
                self.ui
                    .status
                    .update_backend_state(self.backend.connection_state());
                false
            }
            AppEvent::Resize(_, _) => true,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub async fn shutdown(&self) {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Embedded
        ) {
            return;
        }
        let thread_ids: Vec<String> = self
            .ui
            .threads
            .iter()
            .map(|t| t.thread_id.clone())
            .collect();
        for thread_id in thread_ids {
            let _ = self.backend.close_thread(&thread_id).await;
        }
    }

    pub fn ui(&mut self) -> &mut UiState {
        &mut self.ui
    }

    async fn hydrate_daemon_threads(&mut self) {
        match self.backend.list_threads().await {
            Ok(threads) => {
                for snap in threads {
                    let agent = snap.agent.unwrap_or(AgentName::Codex);
                    if let Some(entry) = self
                        .ui
                        .threads
                        .iter_mut()
                        .find(|thread| thread.thread_id == snap.thread_id)
                    {
                        entry.agent = agent;
                        entry.workspace = snap.workspace.clone();
                        entry.state = snap.state.clone();
                    } else {
                        self.ui.threads.push(ThreadEntry {
                            thread_id: snap.thread_id.clone(),
                            agent,
                            workspace: snap.workspace.clone(),
                            state: snap.state.clone(),
                        });
                    }
                    self.ensure_chat_state_agent(&snap.thread_id, agent);
                    self.hydrate_thread_if_needed(&snap.thread_id).await;
                }
                if !self.ui.threads.is_empty() && self.ui.selected_thread.is_none() {
                    self.select_thread(0);
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %e,
                    "hydrate_daemon_threads failed"
                );
            }
        }
    }

    async fn replay_thread_history(&mut self, thread_id: &str) {
        let mut from_seq = None;
        loop {
            let response = match self
                .backend
                .read_thread_raw_history(thread_id, from_seq, 1000)
                .await
            {
                Ok(response) => response,
                Err(e) => {
                    tracing::warn!(
                        target: "minos_tui::app",
                        error = %e,
                        thread_id = %thread_id,
                        "replay_thread_history failed"
                    );
                    return;
                }
            };

            if let Some(chat) = self.ui.chat_states.get_mut(thread_id) {
                for frame in response.events {
                    let events = chat.translation_state.translate(&frame.payload);
                    chat.apply_ui_events(events);
                }
            }

            let Some(next_seq) = response.next_seq else {
                self.hydrated_threads.insert(thread_id.to_owned());
                return;
            };
            from_seq = Some(next_seq.saturating_sub(1));
        }
    }

    async fn hydrate_thread_if_needed(&mut self, thread_id: &str) {
        if self.hydrated_threads.contains(thread_id) {
            return;
        }
        self.replay_thread_history(thread_id).await;
    }

    fn ensure_chat_state_agent(&mut self, thread_id: &str, agent: AgentName) {
        match self.ui.chat_states.entry(thread_id.to_owned()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().agent != agent {
                    entry.insert(ChatState::new(thread_id.to_owned(), agent));
                    self.hydrated_threads.remove(thread_id);
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(ChatState::new(thread_id.to_owned(), agent));
            }
        }
    }

    async fn handle_manager_event(&mut self, event: ManagerEvent) -> bool {
        match event {
            ManagerEvent::ThreadAdded {
                thread_id,
                workspace,
                agent,
            } => {
                if let Some(index) = self
                    .ui
                    .threads
                    .iter()
                    .position(|t| t.thread_id == thread_id)
                {
                    if let Some(entry) = self.ui.threads.get_mut(index) {
                        entry.agent = agent;
                        entry.workspace = workspace;
                    }
                    self.ensure_chat_state_agent(&thread_id, agent);
                    return true;
                }

                let entry = ThreadEntry {
                    thread_id: thread_id.clone(),
                    agent,
                    workspace,
                    state: ThreadState::Starting,
                };
                self.ui.threads.push(entry);
                self.ui
                    .chat_states
                    .insert(thread_id.clone(), ChatState::new(thread_id, agent));
                self.select_thread(self.ui.threads.len().saturating_sub(1));
                self.ui.focus = Focus::Input;
                self.sync_input_agent_picker();
                true
            }
            ManagerEvent::ThreadStateChanged { thread_id, new, .. } => {
                if let Some(entry) = self
                    .ui
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == thread_id)
                {
                    entry.state = new;
                }
                self.record_agent_group_result_if_done(&thread_id);
                true
            }
            ManagerEvent::ThreadClosed { thread_id, reason } => {
                if let Some(entry) = self
                    .ui
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == thread_id)
                {
                    entry.state = ThreadState::Closed { reason };
                }
                self.record_agent_group_result_if_done(&thread_id);
                true
            }
            ManagerEvent::InstanceCrashed {
                affected_threads, ..
            } => {
                for tid in affected_threads {
                    if let Some(entry) = self.ui.threads.iter_mut().find(|t| t.thread_id == tid) {
                        entry.state = ThreadState::Suspended {
                            reason: minos_agent_runtime::PauseReason::InstanceReaped,
                        };
                    }
                }
                true
            }
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) -> bool {
        if matches!(key.kind, KeyEventKind::Release) {
            return false;
        }

        if self.ui.delete_confirm.is_some() {
            return self.handle_delete_confirm_key(key).await;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return false;
                }
                KeyCode::Char('c') => {
                    return self.handle_ctrl_c().await;
                }
                KeyCode::Char('d') => {
                    return self.close_current_thread().await;
                }
                _ => {}
            }
        }

        if self.ui.agent_picker.is_some() {
            return self.handle_agent_picker_key(key).await;
        }

        match key.code {
            KeyCode::PageUp => return self.scroll_current_chat_up(5),
            KeyCode::PageDown => return self.scroll_current_chat_down(5),
            KeyCode::Home => return self.scroll_current_chat_to_top(),
            KeyCode::End => return self.scroll_current_chat_to_bottom(),
            KeyCode::Char('n') if !matches!(self.ui.focus, Focus::Input) => {
                return self.open_agent_picker();
            }
            _ => {}
        }

        match self.ui.focus {
            Focus::Input => self.handle_input_key(key).await,
            Focus::ThreadList => self.handle_thread_list_key(key).await,
            Focus::Chat => self.handle_chat_key(key).await,
        }
    }

    async fn handle_input_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.ui.input.insert_char('\n');
                    self.sync_input_agent_picker();
                    true
                } else if self.ui.input.has_agent_picker() {
                    let agents = self.ui.status.agents.clone();
                    self.ui.input.accept_agent_completion(agents.as_slice());
                    self.sync_input_agent_picker();
                    true
                } else {
                    self.submit_input().await
                }
            }
            KeyCode::Char(c) => {
                self.ui.input.insert_char(c);
                self.sync_input_agent_picker();
                true
            }
            KeyCode::Backspace => {
                self.ui.input.backspace();
                self.sync_input_agent_picker();
                true
            }
            KeyCode::Up if self.ui.input.has_agent_picker() => {
                self.ui.input.select_previous_agent();
                true
            }
            KeyCode::Down if self.ui.input.has_agent_picker() => {
                self.ui.input.select_next_agent();
                true
            }
            KeyCode::Tab => {
                self.cycle_focus();
                true
            }
            KeyCode::Esc => {
                if self.ui.input.has_agent_picker() {
                    self.ui.input.clear_agent_picker();
                    true
                } else {
                    self.handle_escape()
                }
            }
            _ => false,
        }
    }

    async fn handle_thread_list_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                if let Some(selected) = self.ui.selected_thread {
                    self.select_thread(selected.saturating_sub(1));
                }
                true
            }
            KeyCode::Down => {
                if let Some(selected) = self.ui.selected_thread {
                    let last = self.ui.threads.len().saturating_sub(1);
                    self.select_thread((selected + 1).min(last));
                }
                true
            }
            KeyCode::Enter => {
                self.ui.focus = Focus::Chat;
                true
            }
            KeyCode::Delete => self.request_delete_current_thread(),
            KeyCode::Tab => {
                self.cycle_focus();
                true
            }
            KeyCode::Esc => self.handle_escape(),
            _ => false,
        }
    }

    async fn handle_chat_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => self.scroll_current_chat_up(1),
            KeyCode::Down => self.scroll_current_chat_down(1),
            KeyCode::PageUp => self.scroll_current_chat_up(5),
            KeyCode::PageDown => self.scroll_current_chat_down(5),
            KeyCode::Home => self.scroll_current_chat_to_top(),
            KeyCode::End => self.scroll_current_chat_to_bottom(),
            KeyCode::Enter => {
                self.ui.focus = Focus::Input;
                true
            }
            KeyCode::Char('e') => self.toggle_tool_expansion(),
            KeyCode::Tab => {
                self.cycle_focus();
                true
            }
            KeyCode::Esc => self.handle_escape(),
            _ => false,
        }
    }

    async fn handle_agent_picker_key(&mut self, key: KeyEvent) -> bool {
        let len = self.ui.status.agents.len();
        if len == 0 {
            self.ui.agent_picker = None;
            self.ui
                .set_error("No agent detection results available for picker".into());
            return true;
        }

        match key.code {
            KeyCode::Up => {
                if let Some(picker) = self.ui.agent_picker.as_mut() {
                    picker.selected = if picker.selected == 0 {
                        len - 1
                    } else {
                        picker.selected - 1
                    };
                }
                true
            }
            KeyCode::Down => {
                if let Some(picker) = self.ui.agent_picker.as_mut() {
                    picker.selected = (picker.selected + 1) % len;
                }
                true
            }
            KeyCode::Enter => {
                let Some(index) = self.ui.agent_picker.as_ref().map(|picker| picker.selected)
                else {
                    return false;
                };
                self.start_agent_at(index).await
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = usize::from(c as u8 - b'1');
                if index < len {
                    if let Some(picker) = self.ui.agent_picker.as_mut() {
                        picker.selected = index;
                    }
                    return self.start_agent_at(index).await;
                }
                false
            }
            KeyCode::Esc => {
                self.ui.agent_picker = None;
                true
            }
            _ => false,
        }
    }

    async fn handle_delete_confirm_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_delete_thread().await
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.ui.delete_confirm = None;
                true
            }
            _ => true,
        }
    }

    async fn handle_ctrl_c(&mut self) -> bool {
        if self.current_thread_is_interruptible() {
            if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
                if let Err(error) = self.backend.interrupt_thread(&thread_id).await {
                    self.ui
                        .set_error(format!("Failed to interrupt thread: {error}"));
                }
                return true;
            }
        }

        self.should_quit = true;
        false
    }

    async fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        if self.current_chat_selection_active() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    return self.handle_chat_selection_mouse(mouse);
                }
                _ => {}
            }
        }

        if rect_contains(self.ui.panel_areas.thread_list, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.ui.focus = Focus::ThreadList;
                    self.select_previous_thread();
                    true
                }
                MouseEventKind::ScrollDown => {
                    self.ui.focus = Focus::ThreadList;
                    self.select_next_thread();
                    true
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::ThreadList;
                    if let Some(index) = clicked_thread_index(
                        self.ui.panel_areas.thread_list,
                        &self.ui.thread_list_state,
                        mouse.row,
                        self.ui.threads.len(),
                    ) {
                        self.select_thread(index);
                    }
                    true
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.group_chat, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.ui.group_chat.scroll_up(3);
                    true
                }
                MouseEventKind::ScrollDown => {
                    self.ui.group_chat.scroll_down(3);
                    true
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::ThreadList;
                    true
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.chat, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.ui.focus = Focus::Chat;
                    self.sync_input_agent_picker();
                    self.scroll_current_chat_up(3)
                }
                MouseEventKind::ScrollDown => {
                    self.ui.focus = Focus::Chat;
                    self.sync_input_agent_picker();
                    self.scroll_current_chat_down(3)
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::Chat;
                    self.sync_input_agent_picker();
                    if self.begin_chat_selection(mouse.column, mouse.row) {
                        return true;
                    }
                    true
                }
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    self.handle_chat_selection_mouse(mouse)
                }
                MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => false,
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.input, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::Input;
                    self.sync_input_agent_picker();
                    true
                }
                _ => false,
            };
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.ui.focus = Focus::Chat;
                self.sync_input_agent_picker();
                self.scroll_current_chat_up(3)
            }
            MouseEventKind::ScrollDown => {
                self.ui.focus = Focus::Chat;
                self.sync_input_agent_picker();
                self.scroll_current_chat_down(3)
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => false,
            _ => false,
        }
    }

    fn current_chat_selection_active(&self) -> bool {
        self.ui
            .selected_thread
            .and_then(|index| self.ui.threads.get(index))
            .and_then(|thread| self.ui.chat_states.get(&thread.thread_id))
            .is_some_and(|chat| chat.selection.is_some())
    }

    fn begin_chat_selection(&mut self, column: u16, row: u16) -> bool {
        let content_area = chat_content_area(self.ui.panel_areas.chat);
        if !rect_contains(content_area, column, row) {
            if let Some(chat) = self.ui.current_chat_mut() {
                chat.clear_selection();
            }
            return false;
        }

        let Some(chat) = self.ui.current_chat_mut() else {
            return false;
        };
        let point = chat_selection_point(content_area, chat.active_scroll(), column, row);
        chat.begin_selection(point);
        true
    }

    fn handle_chat_selection_mouse(&mut self, mouse: MouseEvent) -> bool {
        let content_area = chat_content_area(self.ui.panel_areas.chat);
        let selected_text = {
            let Some(chat) = self.ui.current_chat_mut() else {
                return false;
            };
            let point =
                chat_selection_point(content_area, chat.active_scroll(), mouse.column, mouse.row);
            chat.update_selection(point);
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
                let text = crate::ui::chat::selected_text(chat, content_area.width);
                if text.is_none() {
                    chat.clear_selection();
                }
                text
            } else {
                None
            }
        };

        if let Some(text) = selected_text {
            if let Err(error) = copy_to_clipboard(&text) {
                self.ui
                    .set_error(format!("Failed to copy selection: {error}"));
            }
        }
        true
    }

    async fn close_current_thread(&mut self) -> bool {
        if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
            if let Err(error) = self.backend.close_thread(&thread_id).await {
                self.ui
                    .set_error(format!("Failed to close thread: {error}"));
            }
            return true;
        }
        false
    }

    fn request_delete_current_thread(&mut self) -> bool {
        let Some(selected) = self.ui.selected_thread else {
            return false;
        };
        let Some(thread) = self.ui.threads.get(selected) else {
            return false;
        };

        self.ui.delete_confirm = Some(DeleteConfirmState {
            thread_id: thread.thread_id.clone(),
            agent: thread.agent,
            workspace: thread.workspace.clone(),
            selected_index: selected,
        });
        true
    }

    async fn confirm_delete_thread(&mut self) -> bool {
        let Some(pending) = self.ui.delete_confirm.take() else {
            return false;
        };

        if let Err(error) = self.backend.delete_thread(&pending.thread_id).await {
            self.ui
                .set_error(format!("Failed to delete thread: {error}"));
            return true;
        }

        self.remove_thread_from_ui(pending.selected_index, &pending.thread_id);
        true
    }

    fn remove_thread_from_ui(&mut self, selected: usize, thread_id: &str) {
        let index = self
            .ui
            .threads
            .get(selected)
            .filter(|entry| entry.thread_id == thread_id)
            .map(|_| selected)
            .or_else(|| {
                self.ui
                    .threads
                    .iter()
                    .position(|entry| entry.thread_id == thread_id)
            });
        let Some(index) = index else {
            return;
        };

        self.ui.threads.remove(index);
        self.ui.chat_states.remove(thread_id);
        self.hydrated_threads.remove(thread_id);

        if self.ui.threads.is_empty() {
            self.ui.selected_thread = None;
            self.ui.thread_list_state.select(None);
        } else {
            self.select_thread(index.min(self.ui.threads.len().saturating_sub(1)));
        }
        self.sync_input_agent_picker();
    }

    fn open_agent_picker(&mut self) -> bool {
        let selected = self
            .ui
            .selected_thread
            .and_then(|index| self.ui.threads.get(index))
            .and_then(|thread| {
                self.ui
                    .status
                    .agents
                    .iter()
                    .position(|agent| agent.name == thread.agent)
            })
            .or_else(|| {
                self.ui
                    .status
                    .agents
                    .iter()
                    .position(|agent| matches!(agent.status, AgentStatus::Ok))
            })
            .unwrap_or(0);
        self.ui.agent_picker = Some(AgentPickerState { selected });
        true
    }

    async fn start_agent_at(&mut self, index: usize) -> bool {
        let Some(agent_name) = self.ui.status.agents.get(index).map(|desc| desc.name) else {
            return false;
        };

        match self.start_new_thread(agent_name).await {
            Ok(_) => {
                self.ui.agent_picker = None;
                true
            }
            Err(error) => {
                self.ui.set_error(error);
                true
            }
        }
    }

    fn select_thread(&mut self, index: usize) {
        self.ui.selected_thread = Some(index);
        self.ui.thread_list_state.select(Some(index));
    }

    fn select_previous_thread(&mut self) {
        if let Some(selected) = self.ui.selected_thread {
            self.select_thread(selected.saturating_sub(1));
        }
    }

    fn select_next_thread(&mut self) {
        if let Some(selected) = self.ui.selected_thread {
            let last = self.ui.threads.len().saturating_sub(1);
            self.select_thread((selected + 1).min(last));
        }
    }

    fn cycle_focus(&mut self) {
        self.ui.focus = match self.ui.focus {
            Focus::ThreadList => Focus::Chat,
            Focus::Chat => Focus::Input,
            Focus::Input => Focus::ThreadList,
        };
        self.sync_input_agent_picker();
    }

    fn handle_escape(&mut self) -> bool {
        if self.ui.agent_picker.is_some() {
            self.ui.agent_picker = None;
            return true;
        }

        if self.ui.input.has_agent_picker() {
            self.ui.input.clear_agent_picker();
            return true;
        }

        if !matches!(self.ui.focus, Focus::ThreadList) {
            self.ui.focus = Focus::ThreadList;
            self.sync_input_agent_picker();
            return true;
        }

        false
    }

    fn scroll_current_chat_up(&mut self, lines: u16) -> bool {
        if let Some(chat) = self.ui.current_chat_mut() {
            chat.scroll_up(lines);
            return true;
        }
        false
    }

    fn scroll_current_chat_down(&mut self, lines: u16) -> bool {
        if let Some(chat) = self.ui.current_chat_mut() {
            chat.scroll_down(lines);
            return true;
        }
        false
    }

    fn scroll_current_chat_to_top(&mut self) -> bool {
        if let Some(chat) = self.ui.current_chat_mut() {
            chat.scroll_to_top();
            return true;
        }
        false
    }

    fn scroll_current_chat_to_bottom(&mut self) -> bool {
        if let Some(chat) = self.ui.current_chat_mut() {
            chat.scroll_to_bottom();
            return true;
        }
        false
    }

    fn toggle_tool_expansion(&mut self) -> bool {
        if let Some(chat) = self.ui.current_chat_mut() {
            for msg in &mut chat.messages {
                for tc in &mut msg.tool_calls {
                    tc.is_expanded = !tc.is_expanded;
                }
            }
            return true;
        }
        false
    }

    fn current_thread_is_interruptible(&self) -> bool {
        self.ui
            .selected_thread
            .and_then(|index| self.ui.threads.get(index))
            .is_some_and(|thread| {
                matches!(
                    thread.state,
                    ThreadState::Starting | ThreadState::Running { .. } | ThreadState::Resuming
                )
            })
    }

    async fn submit_input(&mut self) -> bool {
        let text = self.ui.input.content.clone();
        if text.trim().is_empty() {
            self.ui.input.take_input();
            return true;
        }

        if let Some((agent, body)) = parse_agent_routing(text.as_str()) {
            if body.trim().is_empty() {
                self.ui
                    .set_error(format!("Type a prompt after @{}", agent.bin_name()));
                return true;
            }
            self.ui.input.take_input();
            return self
                .dispatch_prompt_to_agent(agent, body, text.trim().to_owned())
                .await;
        }

        let Some(thread_id) = self.ui.current_thread_id().map(str::to_owned) else {
            self.ui
                .set_error("No thread selected. Press `n` or start with @agent.".into());
            return true;
        };
        let group_text = self.group_user_text_for_thread(&thread_id, text.as_str());
        self.ui.input.take_input();
        self.send_text_to_thread(thread_id, text, group_text).await
    }

    async fn dispatch_prompt_to_agent(
        &mut self,
        agent: AgentName,
        text: String,
        group_text: String,
    ) -> bool {
        if let Some(thread_id) = self.selected_thread_for_agent(agent) {
            return self
                .send_text_to_thread(thread_id, text, Some(group_text))
                .await;
        }

        match self.start_new_thread(agent).await {
            Ok(thread_id) => {
                self.send_text_to_thread(thread_id, text, Some(group_text))
                    .await
            }
            Err(error) => {
                self.ui.set_error(error);
                true
            }
        }
    }

    async fn send_text_to_thread(
        &mut self,
        thread_id: String,
        text: String,
        group_text: Option<String>,
    ) -> bool {
        if let Some(index) = self
            .ui
            .threads
            .iter()
            .position(|thread| thread.thread_id == thread_id)
        {
            self.select_thread(index);
        }
        if let Some(chat) = self.ui.current_chat_mut() {
            chat.scroll_to_bottom();
        }
        self.hydrate_thread_if_needed(&thread_id).await;
        if let Err(e) = self.backend.resume_thread(&thread_id).await {
            tracing::debug!(
                target: "minos_tui::app",
                error = %e,
                thread_id = %thread_id,
                "resume_thread failed or not needed"
            );
        }
        if let Err(error) = self.backend.send_message(&thread_id, &text).await {
            self.ui
                .set_error(format!("Failed to send message: {error}"));
        } else if let Some(group_text) = group_text {
            self.record_user_group_message(&thread_id, group_text);
        }
        true
    }

    async fn start_new_thread(&mut self, agent: AgentName) -> Result<String, String> {
        let Some(descriptor) = self
            .ui
            .status
            .agents
            .iter()
            .find(|desc| desc.name == agent)
            .cloned()
        else {
            return Err(format!("Unknown agent: {}", agent.bin_name()));
        };

        match descriptor.status {
            AgentStatus::Ok => match self
                .backend
                .start_agent(agent, self.workspace.clone())
                .await
            {
                Ok(outcome) => {
                    let thread_id = outcome.thread_id.clone();
                    self.ensure_thread_visible(thread_id.clone(), agent, outcome.cwd);
                    Ok(thread_id)
                }
                Err(error) => Err(format!("Failed to start {}: {error}", agent.bin_name())),
            },
            AgentStatus::Missing => Err(format!("{} is not installed on PATH", agent.bin_name())),
            AgentStatus::Error { reason } => {
                Err(format!("{} is unavailable: {reason}", agent.bin_name()))
            }
        }
    }

    fn ensure_thread_visible(&mut self, thread_id: String, agent: AgentName, workspace: PathBuf) {
        if let Some(index) = self
            .ui
            .threads
            .iter()
            .position(|thread| thread.thread_id == thread_id)
        {
            if let Some(entry) = self.ui.threads.get_mut(index) {
                entry.agent = agent;
                entry.workspace = workspace;
            }
            self.ensure_chat_state_agent(&thread_id, agent);
            self.select_thread(index);
            self.ui.focus = Focus::Input;
            self.sync_input_agent_picker();
            return;
        }

        self.ui.threads.push(ThreadEntry {
            thread_id: thread_id.clone(),
            agent,
            workspace,
            state: ThreadState::Starting,
        });
        self.ensure_chat_state_agent(&thread_id, agent);
        self.select_thread(self.ui.threads.len().saturating_sub(1));
        self.ui.focus = Focus::Input;
        self.sync_input_agent_picker();
    }

    fn selected_thread_for_agent(&self, agent: AgentName) -> Option<String> {
        self.ui
            .selected_thread
            .and_then(|index| self.ui.threads.get(index))
            .filter(|thread| thread.agent == agent)
            .map(|thread| thread.thread_id.clone())
    }

    fn sync_input_agent_picker(&mut self) {
        let agents = self.ui.status.agents.clone();
        self.ui
            .input
            .sync_agent_picker(agents.as_slice(), matches!(self.ui.focus, Focus::Input));
    }

    fn load_group_chat_history(&mut self) {
        match self.group_chat_store.load_recent(500) {
            Ok(messages) => self.ui.group_chat.replace_messages(messages),
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    "failed to load group chat history"
                );
                self.ui
                    .set_error(format!("Failed to load group chat history: {error}"));
            }
        }
    }

    fn group_user_text_for_thread(&self, thread_id: &str, text: &str) -> Option<String> {
        let thread = self
            .ui
            .threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(format!("@{} {trimmed}", thread.agent.bin_name()))
    }

    fn record_user_group_message(&mut self, thread_id: &str, text: String) {
        let Some(message) = self.group_message(thread_id, LocalGroupChatMessageKind::User, text)
        else {
            return;
        };
        self.append_group_chat_message(message);
    }

    fn record_agent_group_result_if_done(&mut self, thread_id: &str) {
        let Some(thread) = self
            .ui
            .threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)
        else {
            return;
        };
        if !thread_is_done(&thread.state) {
            return;
        }
        let Some(chat) = self.ui.chat_states.get(thread_id) else {
            return;
        };
        let Some((message_key, text)) = chat.last_completed_assistant_text() else {
            return;
        };
        if self
            .recorded_agent_results
            .get(thread_id)
            .is_some_and(|recorded| recorded == &message_key)
        {
            return;
        }
        let Some(message) =
            self.group_message(thread_id, LocalGroupChatMessageKind::AgentResult, text)
        else {
            return;
        };
        self.append_group_chat_message(message);
        self.recorded_agent_results
            .insert(thread_id.to_owned(), message_key);
    }

    fn group_message(
        &self,
        thread_id: &str,
        kind: LocalGroupChatMessageKind,
        text: String,
    ) -> Option<LocalGroupChatMessage> {
        let thread = self
            .ui
            .threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)?;
        Some(LocalGroupChatMessage {
            seq: 0,
            message_id: String::new(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            kind,
            text,
            agent: Some(thread.agent),
            thread_id: Some(thread.thread_id.clone()),
            thread_short_id: Some(short_thread_id(&thread.thread_id)),
            workspace: Some(thread.workspace.display().to_string()),
        })
    }

    fn append_group_chat_message(&mut self, message: LocalGroupChatMessage) {
        match self.group_chat_store.append(message) {
            Ok(message) => self.ui.group_chat.push_message(message),
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    "failed to append group chat message"
                );
                self.ui
                    .set_error(format!("Failed to record group chat message: {error}"));
            }
        }
    }
}

#[cfg(not(test))]
fn default_group_chat_store() -> GroupChatStore {
    match GroupChatStore::default_for_runtime() {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(
                target: "minos_tui::app",
                error = %error,
                "group chat persistence disabled"
            );
            GroupChatStore::disabled()
        }
    }
}

#[cfg(test)]
fn default_group_chat_store() -> GroupChatStore {
    GroupChatStore::disabled()
}

fn thread_is_done(state: &ThreadState) -> bool {
    matches!(state, ThreadState::Idle | ThreadState::Closed { .. })
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
}

fn parse_agent_routing(text: &str) -> Option<(AgentName, String)> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('@')?;
    let split_at = rest
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let agent = parse_agent_name(&rest[..split_at])?;
    let body = rest[split_at..].trim_start().to_owned();
    Some((agent, body))
}

fn parse_agent_name(value: &str) -> Option<AgentName> {
    let normalized = value.to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
}

fn rect_contains(area: ratatui::layout::Rect, column: u16, row: u16) -> bool {
    area.width > 0
        && area.height > 0
        && column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn chat_content_area(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    if area.width <= 2 || area.height <= 2 {
        return ratatui::layout::Rect::new(area.x, area.y, 0, 0);
    }
    ratatui::layout::Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

fn chat_selection_point(
    area: ratatui::layout::Rect,
    scroll: u16,
    column: u16,
    row: u16,
) -> ChatSelectionPoint {
    let col = if area.width == 0 || column <= area.x {
        0
    } else if column >= area.x.saturating_add(area.width) {
        area.width.saturating_sub(1)
    } else {
        column.saturating_sub(area.x)
    };
    let row_offset = if area.height == 0 || row <= area.y {
        0
    } else if row >= area.y.saturating_add(area.height) {
        area.height.saturating_sub(1)
    } else {
        row.saturating_sub(area.y)
    };
    ChatSelectionPoint {
        row: usize::from(scroll).saturating_add(usize::from(row_offset)),
        col: usize::from(col),
    }
}

#[cfg(test)]
static TEST_CLIPBOARD: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .push(text.to_owned());
    Ok(())
}

#[cfg(not(test))]
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    const COMMANDS: &[(&str, &[&str])] = &[("pbcopy", &[])];
    #[cfg(target_os = "linux")]
    const COMMANDS: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    #[cfg(target_os = "windows")]
    const COMMANDS: &[(&str, &[&str])] =
        &[("powershell", &["-NoProfile", "-Command", "Set-Clipboard"])];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    const COMMANDS: &[(&str, &[&str])] = &[];

    let mut last_error = None;
    for (program, args) in COMMANDS {
        match run_clipboard_command(program, args, text) {
            Ok(true) => return Ok(()),
            Ok(false) => {
                last_error = Some(anyhow::anyhow!("{program} exited with a non-zero status"));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => last_error = Some(error.into()),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no clipboard command available")))
}

#[cfg(not(test))]
fn run_clipboard_command(program: &str, args: &[&str], text: &str) -> std::io::Result<bool> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    Ok(child.wait()?.success())
}

fn clicked_thread_index(
    area: ratatui::layout::Rect,
    list_state: &ratatui::widgets::ListState,
    row: u16,
    thread_count: usize,
) -> Option<usize> {
    if area.height <= 2
        || row <= area.y
        || row >= area.y.saturating_add(area.height).saturating_sub(1)
    {
        return None;
    }

    let item_row = usize::from(row.saturating_sub(area.y + 1));
    let index = list_state.offset().saturating_add(item_row);
    (index < thread_count).then_some(index)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use crate::backend::{BackendConnectionState, BackendThreadSnapshot};
    use anyhow::Result;
    use async_trait::async_trait;
    use crossterm::event::{KeyEventState, MouseEvent, MouseEventKind};
    use minos_agent_runtime::{RawIngest, StartAgentOutcome};
    use minos_domain::{AgentDescriptor, AgentName, AgentStatus};
    use minos_protocol::local_rpc::ReadThreadRawHistoryResponse;
    use minos_protocol::LocalGroupChatMessageKind;
    use minos_ui_protocol::{MessageRole, UiEventMessage};
    use ratatui::layout::Rect;
    use tokio::sync::broadcast;

    use super::*;

    struct TestBackend {
        detected_agents: Vec<AgentDescriptor>,
        started: Mutex<Vec<AgentName>>,
        sent_messages: Mutex<Vec<(String, String)>>,
        next_thread: Mutex<usize>,
        interrupted: Mutex<Vec<String>>,
        closed: Mutex<Vec<String>>,
        deleted: Mutex<Vec<String>>,
        listed_threads: Mutex<Vec<BackendThreadSnapshot>>,
        history_pages: Mutex<HashMap<String, VecDeque<ReadThreadRawHistoryResponse>>>,
        history_calls: Mutex<Vec<(String, Option<u64>, u32)>>,
        connection_state: BackendConnectionState,
        ingest_tx: broadcast::Sender<RawIngest>,
        manager_tx: broadcast::Sender<ManagerEvent>,
    }

    impl TestBackend {
        fn new() -> Self {
            Self::with_agents(Vec::new())
        }

        fn with_agents(detected_agents: Vec<AgentDescriptor>) -> Self {
            let (ingest_tx, _) = broadcast::channel(8);
            let (manager_tx, _) = broadcast::channel(8);
            Self {
                detected_agents,
                started: Mutex::new(Vec::new()),
                sent_messages: Mutex::new(Vec::new()),
                next_thread: Mutex::new(0),
                interrupted: Mutex::new(Vec::new()),
                closed: Mutex::new(Vec::new()),
                deleted: Mutex::new(Vec::new()),
                listed_threads: Mutex::new(Vec::new()),
                history_pages: Mutex::new(HashMap::new()),
                history_calls: Mutex::new(Vec::new()),
                connection_state: BackendConnectionState::Embedded,
                ingest_tx,
                manager_tx,
            }
        }

        fn with_connection_state(mut self, connection_state: BackendConnectionState) -> Self {
            self.connection_state = connection_state;
            self
        }

        fn with_listed_threads(self, listed_threads: Vec<BackendThreadSnapshot>) -> Self {
            *self.listed_threads.lock().expect("listed threads lock") = listed_threads;
            self
        }

        fn with_history_pages(
            self,
            thread_id: &str,
            pages: Vec<ReadThreadRawHistoryResponse>,
        ) -> Self {
            self.history_pages
                .lock()
                .expect("history pages lock")
                .insert(thread_id.to_owned(), VecDeque::from(pages));
            self
        }
    }

    #[async_trait]
    impl AgentBackend for TestBackend {
        async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>> {
            Ok(self.detected_agents.clone())
        }

        async fn start_agent(
            &self,
            agent: AgentName,
            workspace: PathBuf,
        ) -> Result<StartAgentOutcome> {
            self.started.lock().expect("started list lock").push(agent);
            let mut next_thread = self.next_thread.lock().expect("next_thread lock");
            *next_thread += 1;
            Ok(StartAgentOutcome {
                thread_id: format!("thread-{}", *next_thread),
                cwd: workspace,
                provider_session_id: None,
            })
        }

        async fn send_message(&self, thread_id: &str, text: &str) -> Result<()> {
            self.sent_messages
                .lock()
                .expect("sent messages lock")
                .push((thread_id.to_owned(), text.to_owned()));
            Ok(())
        }

        async fn interrupt_thread(&self, thread_id: &str) -> Result<()> {
            self.interrupted
                .lock()
                .expect("interrupt list lock")
                .push(thread_id.to_owned());
            Ok(())
        }

        async fn close_thread(&self, thread_id: &str) -> Result<()> {
            self.closed
                .lock()
                .expect("close list lock")
                .push(thread_id.to_owned());
            Ok(())
        }

        async fn delete_thread(&self, thread_id: &str) -> Result<()> {
            self.deleted
                .lock()
                .expect("delete list lock")
                .push(thread_id.to_owned());
            Ok(())
        }

        async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>> {
            Ok(self
                .listed_threads
                .lock()
                .expect("listed threads lock")
                .clone())
        }

        async fn resume_thread(&self, _thread_id: &str) -> Result<StartAgentOutcome> {
            Ok(StartAgentOutcome {
                thread_id: String::new(),
                cwd: PathBuf::new(),
                provider_session_id: None,
            })
        }

        async fn read_thread_raw_history(
            &self,
            thread_id: &str,
            from_seq: Option<u64>,
            limit: u32,
        ) -> Result<ReadThreadRawHistoryResponse> {
            self.history_calls
                .lock()
                .expect("history calls lock")
                .push((thread_id.to_owned(), from_seq, limit));
            let mut pages = self.history_pages.lock().expect("history pages lock");
            Ok(pages
                .get_mut(thread_id)
                .and_then(VecDeque::pop_front)
                .unwrap_or(ReadThreadRawHistoryResponse {
                    events: Vec::new(),
                    next_seq: None,
                }))
        }

        async fn subscribe_ingest(&self) -> broadcast::Receiver<RawIngest> {
            self.ingest_tx.subscribe()
        }

        async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
            self.manager_tx.subscribe()
        }

        fn connection_state(&self) -> BackendConnectionState {
            self.connection_state.clone()
        }
    }

    fn ok_agent(agent: AgentName) -> AgentDescriptor {
        AgentDescriptor {
            name: agent,
            path: Some(format!("/usr/local/bin/{}", agent.bin_name())),
            version: Some("1.0.0".into()),
            status: AgentStatus::Ok,
        }
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn scroll(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[tokio::test]
    async fn ctrl_c_interrupts_running_thread() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        app.select_thread(0);

        let key = KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let redraw = app.handle_key(key).await;

        assert!(redraw);
        assert!(!app.should_quit());
        assert_eq!(
            backend
                .interrupted
                .lock()
                .expect("interrupt list lock")
                .as_slice(),
            &["thread-1".to_owned()]
        );
    }

    #[tokio::test]
    async fn ctrl_c_quits_idle_thread_view() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Gemini,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.select_thread(0);

        let key = KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let redraw = app.handle_key(key).await;

        assert!(!redraw);
        assert!(app.should_quit());
        assert!(backend
            .interrupted
            .lock()
            .expect("interrupt list lock")
            .is_empty());
    }

    #[tokio::test]
    async fn open_agent_picker_defaults_to_current_thread_agent() {
        let backend = Arc::new(TestBackend::with_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
            ok_agent(AgentName::Gemini),
        ]));
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.status.update_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
            ok_agent(AgentName::Gemini),
        ]);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-claude".into(),
            agent: AgentName::Claude,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.select_thread(0);

        assert!(app.open_agent_picker());
        assert_eq!(
            app.ui.agent_picker.as_ref().map(|picker| picker.selected),
            Some(1)
        );
    }

    #[tokio::test]
    async fn at_completion_inserts_selected_agent() {
        let backend = Arc::new(TestBackend::with_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
            ok_agent(AgentName::Gemini),
        ]));
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.status.update_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
            ok_agent(AgentName::Gemini),
        ]);
        app.ui.focus = Focus::Input;
        app.sync_input_agent_picker();

        assert!(app.handle_key(press(KeyCode::Char('@'))).await);
        assert!(app.handle_key(press(KeyCode::Char('c'))).await);
        assert!(app.handle_key(press(KeyCode::Down)).await);
        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(app.ui.input.content, "@claude ");
        assert_eq!(app.ui.input.cursor_pos, "@claude ".len());
        assert!(!app.ui.input.has_agent_picker());
    }

    #[tokio::test]
    async fn routed_prompt_starts_target_agent_and_sends_body_only() {
        let backend = Arc::new(TestBackend::with_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
            ok_agent(AgentName::Gemini),
        ]));
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.status.update_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
            ok_agent(AgentName::Gemini),
        ]);
        app.ui.focus = Focus::Input;
        app.ui.input.content = "@gemini write tests".into();
        app.ui.input.cursor_pos = app.ui.input.content.len();
        app.sync_input_agent_picker();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(
            backend
                .started
                .lock()
                .expect("started list lock")
                .as_slice(),
            &[AgentName::Gemini]
        );
        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[("thread-1".to_owned(), "write tests".to_owned())]
        );
        assert_eq!(app.ui.input.content, "");
        assert_eq!(app.ui.selected_thread, Some(0));
        assert_eq!(app.ui.threads[0].agent, AgentName::Gemini);
    }

    #[tokio::test]
    async fn routed_prompt_records_user_message_in_group_chat() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Gemini)]));
        let mut app =
            App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Gemini)]);
        app.ui.focus = Focus::Input;
        app.ui.input.content = "@gemini write tests".into();
        app.ui.input.cursor_pos = app.ui.input.content.len();
        app.sync_input_agent_picker();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[("thread-1".to_owned(), "write tests".to_owned())]
        );
        assert_eq!(app.ui.group_chat.messages.len(), 1);
        let message = &app.ui.group_chat.messages[0];
        assert_eq!(message.seq, 1);
        assert_eq!(message.kind, LocalGroupChatMessageKind::User);
        assert_eq!(message.text, "@gemini write tests");
        assert_eq!(message.agent, Some(AgentName::Gemini));

        let persisted = std::fs::read_to_string(temp.path().join("group.jsonl"))
            .expect("group chat log should be written");
        assert!(persisted.contains(r#""kind":"user""#));
        assert!(persisted.contains(r#""text":"@gemini write tests""#));
    }

    #[tokio::test]
    async fn routed_prompt_reuses_selected_thread_for_same_agent() {
        let backend = Arc::new(TestBackend::with_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
        ]));
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.status.update_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
        ]);
        app.ui.focus = Focus::Input;
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-codex".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.select_thread(0);
        app.ui.chat_states.insert(
            "thread-codex".into(),
            ChatState::new("thread-codex".into(), AgentName::Codex),
        );
        app.ui.input.content = "@codex explain the diff".into();
        app.ui.input.cursor_pos = app.ui.input.content.len();
        app.sync_input_agent_picker();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert!(backend
            .started
            .lock()
            .expect("started list lock")
            .is_empty());
        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[("thread-codex".to_owned(), "explain the diff".to_owned())]
        );
    }

    #[tokio::test]
    async fn idle_thread_records_last_assistant_message_in_group_chat_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(TestBackend::new());
        let mut app =
            App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-gemini-1234".into(),
            agent: AgentName::Gemini,
            workspace: PathBuf::from("/tmp/ws"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        let mut chat = ChatState::new("thread-gemini-1234".into(), AgentName::Gemini);
        chat.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "assistant-1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 1,
            },
            UiEventMessage::TextDelta {
                message_id: "assistant-1".into(),
                text: "The module handles auth.".into(),
            },
            UiEventMessage::MessageCompleted {
                message_id: "assistant-1".into(),
                finished_at_ms: 2,
            },
        ]);
        app.ui.chat_states.insert("thread-gemini-1234".into(), chat);
        app.select_thread(0);

        let event = ManagerEvent::ThreadStateChanged {
            thread_id: "thread-gemini-1234".into(),
            old: ThreadState::Running {
                turn_started_at_ms: 0,
            },
            new: ThreadState::Idle,
            at_ms: 3,
        };
        assert!(
            app.handle_event(AppEvent::ManagerEvent(event.clone()))
                .await
        );
        assert!(app.handle_event(AppEvent::ManagerEvent(event)).await);

        assert_eq!(app.ui.group_chat.messages.len(), 1);
        let message = &app.ui.group_chat.messages[0];
        assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
        assert_eq!(message.text, "The module handles auth.");
        assert_eq!(message.agent, Some(AgentName::Gemini));
        assert_eq!(message.thread_short_id.as_deref(), Some("thread-g"));
    }

    #[tokio::test]
    async fn esc_moves_focus_without_quitting() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.focus = Focus::Chat;

        let redraw = app.handle_key(press(KeyCode::Esc)).await;

        assert!(redraw);
        assert_eq!(app.ui.focus, Focus::ThreadList);
        assert!(!app.should_quit());
    }

    #[tokio::test]
    async fn mouse_wheel_scrolls_chat_and_focuses_it() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.chat_states.insert(
            "thread-1".into(),
            ChatState::new("thread-1".into(), AgentName::Codex),
        );
        app.select_thread(0);
        app.ui.focus = Focus::Input;
        app.ui.panel_areas.chat = Rect::new(20, 0, 60, 20);
        if let Some(chat) = app.ui.current_chat_mut() {
            chat.update_max_scroll(40);
        }

        let redraw = app
            .handle_mouse(MouseEvent {
                row: 1,
                column: 25,
                ..scroll(MouseEventKind::ScrollUp)
            })
            .await;

        assert!(redraw);
        assert_eq!(app.ui.focus, Focus::Chat);
        assert!(app
            .ui
            .current_chat_mut()
            .is_some_and(|chat| !chat.auto_scroll));
    }

    #[tokio::test]
    async fn mouse_wheel_over_thread_list_moves_selection() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-2".into(),
            agent: AgentName::Claude,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.select_thread(0);
        app.ui.panel_areas.thread_list = Rect::new(0, 0, 20, 10);

        let redraw = app
            .handle_mouse(MouseEvent {
                row: 2,
                column: 1,
                ..scroll(MouseEventKind::ScrollDown)
            })
            .await;

        assert!(redraw);
        assert_eq!(app.ui.focus, Focus::ThreadList);
        assert_eq!(app.ui.selected_thread, Some(1));
    }

    #[tokio::test]
    async fn clicking_thread_list_blank_area_focuses_thread_list() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.panel_areas.thread_list = Rect::new(0, 0, 20, 10);
        app.ui.focus = Focus::Chat;

        let redraw = app
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 8,
                modifiers: KeyModifiers::NONE,
            })
            .await;

        assert!(redraw);
        assert_eq!(app.ui.focus, Focus::ThreadList);
    }

    #[tokio::test]
    async fn mouse_selection_copies_chat_text_on_release() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        let mut chat = ChatState::new("thread-1".into(), AgentName::Codex);
        chat.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "m1".into(),
                role: MessageRole::User,
                started_at_ms: 0,
            },
            UiEventMessage::TextDelta {
                message_id: "m1".into(),
                text: "hello\nworld".into(),
            },
            UiEventMessage::MessageCompleted {
                message_id: "m1".into(),
                finished_at_ms: 1,
            },
        ]);
        app.ui.chat_states.insert("thread-1".into(), chat);
        app.select_thread(0);
        app.ui.panel_areas.chat = Rect::new(0, 0, 40, 10);
        super::TEST_CLIPBOARD
            .lock()
            .expect("test clipboard lock")
            .clear();

        let down = app
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 2,
                modifiers: KeyModifiers::NONE,
            })
            .await;
        assert!(down);
        assert!(super::TEST_CLIPBOARD
            .lock()
            .expect("test clipboard lock")
            .is_empty());

        let up = app
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 3,
                row: 3,
                modifiers: KeyModifiers::NONE,
            })
            .await;

        assert!(up);
        assert_eq!(
            super::TEST_CLIPBOARD
                .lock()
                .expect("test clipboard lock")
                .as_slice(),
            &["ello\nwor".to_owned()]
        );
    }

    #[tokio::test]
    async fn delete_key_in_thread_list_opens_confirmation() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-2".into(),
            agent: AgentName::Claude,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.chat_states.insert(
            "thread-1".into(),
            ChatState::new("thread-1".into(), AgentName::Codex),
        );
        app.hydrated_threads.insert("thread-1".into());
        app.select_thread(0);
        app.ui.focus = Focus::ThreadList;

        let redraw = app.handle_key(press(KeyCode::Delete)).await;

        assert!(redraw);
        assert!(app.ui.delete_confirm.is_some());
        assert!(backend.deleted.lock().expect("delete list lock").is_empty());
        assert_eq!(app.ui.threads.len(), 2);
        assert!(app.ui.chat_states.contains_key("thread-1"));

        let redraw = app.handle_key(press(KeyCode::Esc)).await;

        assert!(redraw);
        assert!(app.ui.delete_confirm.is_none());
        assert_eq!(app.ui.threads.len(), 2);
    }

    #[tokio::test]
    async fn enter_confirms_thread_delete_and_removes_local_state() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-2".into(),
            agent: AgentName::Claude,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.chat_states.insert(
            "thread-1".into(),
            ChatState::new("thread-1".into(), AgentName::Codex),
        );
        app.hydrated_threads.insert("thread-1".into());
        app.select_thread(0);
        app.ui.focus = Focus::ThreadList;

        assert!(app.handle_key(press(KeyCode::Delete)).await);
        let redraw = app.handle_key(press(KeyCode::Enter)).await;

        assert!(redraw);
        assert!(app.ui.delete_confirm.is_none());
        assert_eq!(
            backend.deleted.lock().expect("delete list lock").as_slice(),
            &["thread-1".to_owned()]
        );
        assert_eq!(app.ui.threads.len(), 1);
        assert_eq!(app.ui.threads[0].thread_id, "thread-2");
        assert_eq!(app.ui.selected_thread, Some(0));
        assert!(!app.ui.chat_states.contains_key("thread-1"));
        assert!(!app.hydrated_threads.contains("thread-1"));
    }

    #[tokio::test]
    async fn init_hydrates_connected_daemon_threads_with_agent_and_paginated_history() {
        let backend = Arc::new(
            TestBackend::with_agents(vec![ok_agent(AgentName::Claude)])
                .with_connection_state(BackendConnectionState::Connected {
                    endpoint: "ws://127.0.0.1:43123".into(),
                })
                .with_listed_threads(vec![BackendThreadSnapshot {
                    thread_id: "thread-1".into(),
                    agent: Some(AgentName::Claude),
                    workspace: PathBuf::from("/tmp/ws"),
                    state: ThreadState::Suspended {
                        reason: minos_agent_runtime::PauseReason::DaemonRestart,
                    },
                }])
                .with_history_pages(
                    "thread-1",
                    vec![
                        ReadThreadRawHistoryResponse {
                            events: Vec::new(),
                            next_seq: Some(2),
                        },
                        ReadThreadRawHistoryResponse {
                            events: Vec::new(),
                            next_seq: None,
                        },
                    ],
                ),
        );
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));

        app.init().await.unwrap();

        assert_eq!(app.ui.threads.len(), 1);
        assert_eq!(app.ui.threads[0].agent, AgentName::Claude);
        assert_eq!(app.ui.selected_thread, Some(0));
        assert_eq!(
            app.ui
                .chat_states
                .get("thread-1")
                .expect("chat state")
                .agent,
            AgentName::Claude
        );
        assert_eq!(
            backend
                .history_calls
                .lock()
                .expect("history calls lock")
                .as_slice(),
            &[
                ("thread-1".to_owned(), None, 1000),
                ("thread-1".to_owned(), Some(1), 1000),
            ]
        );
    }

    #[tokio::test]
    async fn shutdown_does_not_close_threads_for_daemon_backend() {
        let backend = Arc::new(TestBackend::new().with_connection_state(
            BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:43123".into(),
            },
        ));
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-1".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });

        app.shutdown().await;

        assert!(backend.closed.lock().expect("close list lock").is_empty());
    }
}
