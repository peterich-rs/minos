use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use minos_agent_runtime::{ManagerEvent, ThreadState};
use minos_chat_store::mcp_socket::{SocketRequest, SocketResponse};
use minos_domain::{AgentName, AgentStatus};
use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind};
use tracing::debug;

use crate::backend::AgentBackend;
use crate::event::AppEvent;
use crate::group_chat::GroupChatStore;
use crate::translation::{
    ChatItem, ChatSelectionPoint, ChatState, PendingAgentRequestKind, PendingQuestionSpec,
};
use crate::ui::{
    room_list::RoomEntry, AgentPickerState, DeleteConfirmState, Focus, ThreadEntry, UiState,
};

enum MessageTarget {
    ExistingThread(String),
    NewAgent(AgentName),
}

pub struct App {
    backend: Arc<dyn AgentBackend>,
    ui: UiState,
    should_quit: bool,
    workspace: PathBuf,
    hydrated_threads: HashSet<String>,
    thread_watermarks: HashMap<String, u64>,
    applied_ingest_fingerprints: HashSet<String>,
    group_chat_store: GroupChatStore,
    recorded_agent_results: HashMap<String, String>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<AppEvent>>,
    last_daemon_history_sync: Option<Instant>,
    last_group_result_retry: Option<Instant>,
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
        let mut ui = UiState::new(readonly);
        ui.rooms.push(RoomEntry {
            room_id: workspace_room_id(workspace.as_path()),
            title: default_room_title(workspace.as_path()),
        });
        ui.selected_room = Some(0);
        ui.room_list_state.select(Some(0));
        Self {
            backend,
            ui,
            should_quit: false,
            workspace,
            hydrated_threads: HashSet::new(),
            thread_watermarks: HashMap::new(),
            applied_ingest_fingerprints: HashSet::new(),
            group_chat_store,
            recorded_agent_results: HashMap::new(),
            event_tx: None,
            last_daemon_history_sync: None,
            last_group_result_retry: None,
        }
    }

    pub async fn init(&mut self) -> anyhow::Result<()> {
        self.load_group_chat_history().await;
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
                if !self.ui.chat_states.contains_key(&ingest.thread_id) {
                    debug!(
                        agent = %ingest.agent.bin_name(),
                        thread_id = %ingest.thread_id,
                        "dropping ingest event because no chat state exists"
                    );
                    return false;
                }
                if !self.mark_ingest_applied(&ingest) {
                    return false;
                }
                let marks_done = frame_marks_agent_result_done(&ingest);
                if let Some(chat) = self.ui.chat_states.get_mut(&ingest.thread_id) {
                    debug!(
                        agent = %ingest.agent.bin_name(),
                        thread_id = %ingest.thread_id,
                        event_count = ingest.ui_events.len(),
                        "applying projected ingest frame"
                    );
                    chat.apply_ui_events(ingest.ui_events.clone());
                    let thread_id = ingest.thread_id.clone();
                    self.record_agent_group_result_if_ingest_done(&thread_id, marks_done)
                        .await;
                    return true;
                }
                false
            }
            AppEvent::ManagerEvent(event) => self.handle_manager_event(event).await,
            AppEvent::AgentStartedForPrompt {
                agent,
                thread_id,
                cwd,
                text,
            } => {
                self.ensure_thread_visible(thread_id.clone(), agent, cwd);
                self.send_text_to_thread(thread_id, text, None).await
            }
            AppEvent::SendMessageFailed { thread_id, error } => {
                self.ui.set_error(format!(
                    "Failed to send message to {}: {error}",
                    short_thread_id(&thread_id)
                ));
                true
            }
            AppEvent::McpToolCall(event) => {
                let response = self.handle_mcp_tool_call(event.request).await;
                let _ = event.response_tx.send(response);
                true
            }
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Paste(text) => self.handle_paste(text),
            AppEvent::Mouse(mouse) => self.handle_mouse(mouse).await,
            AppEvent::Tick => self.handle_tick().await,
            AppEvent::Resize(_, _) => true,
        }
    }

    async fn handle_tick(&mut self) -> bool {
        let mut redraw = false;
        if self.sync_daemon_threads_if_due().await {
            redraw = true;
        }
        if let Some((_, instant)) = self.ui.error_flash {
            if instant.elapsed() > Duration::from_secs(3) {
                self.ui.error_flash = None;
                redraw = true;
            }
        }
        self.ui
            .status
            .update_backend_state(self.backend.connection_state());
        if self.refresh_group_chat_from_backend().await {
            redraw = true;
        }
        if self.retry_pending_agent_group_results_if_due().await {
            redraw = true;
        }
        redraw
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

    pub fn set_event_sender(&mut self, tx: tokio::sync::mpsc::UnboundedSender<AppEvent>) {
        self.event_tx = Some(tx);
    }

    async fn hydrate_daemon_threads(&mut self) {
        let _ = self.sync_daemon_threads_from_backend(false).await;
    }

    async fn sync_daemon_threads_if_due(&mut self) -> bool {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            return false;
        }
        let now = Instant::now();
        if self
            .last_daemon_history_sync
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return false;
        }
        self.last_daemon_history_sync = Some(now);
        self.sync_daemon_threads_from_backend(true).await
    }

    async fn sync_daemon_threads_from_backend(&mut self, incremental: bool) -> bool {
        let mut changed = false;
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
                        if entry.agent != agent {
                            entry.agent = agent;
                            changed = true;
                        }
                        if entry.workspace != snap.workspace {
                            entry.workspace = snap.workspace.clone();
                            changed = true;
                        }
                        if entry.state != snap.state {
                            entry.state = snap.state.clone();
                            changed = true;
                        }
                    } else {
                        self.ui.threads.push(ThreadEntry {
                            thread_id: snap.thread_id.clone(),
                            agent,
                            workspace: snap.workspace.clone(),
                            state: snap.state.clone(),
                        });
                        changed = true;
                    }
                    self.ensure_chat_state_agent(&snap.thread_id, agent);
                    if incremental && self.hydrated_threads.contains(&snap.thread_id) {
                        if self
                            .replay_thread_history_after_watermark(&snap.thread_id)
                            .await
                        {
                            changed = true;
                        }
                    } else if self.hydrate_thread_if_needed(&snap.thread_id).await {
                        changed = true;
                    }
                    let before = self.ui.group_chat.messages.len();
                    self.record_agent_group_result_if_done(&snap.thread_id)
                        .await;
                    if self.ui.group_chat.messages.len() != before {
                        changed = true;
                    }
                }
                if !self.ui.threads.is_empty() && self.ui.selected_thread.is_none() {
                    self.select_thread(0);
                    changed = true;
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
        changed
    }

    async fn replay_thread_history_from(
        &mut self,
        thread_id: &str,
        mut from_seq: Option<u64>,
        mark_hydrated: bool,
    ) -> bool {
        let mut changed = false;
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
                    return changed;
                }
            };

            for frame in response.events {
                if !self.mark_ingest_applied(&frame) {
                    continue;
                }
                if let Some(chat) = self.ui.chat_states.get_mut(thread_id) {
                    if !frame.ui_events.is_empty() {
                        changed = true;
                    }
                    chat.apply_ui_events(frame.ui_events);
                }
            }

            let Some(next_seq) = response.next_seq else {
                if mark_hydrated {
                    self.hydrated_threads.insert(thread_id.to_owned());
                }
                return changed;
            };
            from_seq = Some(next_seq.saturating_sub(1));
        }
    }

    async fn replay_thread_history_after_watermark(&mut self, thread_id: &str) -> bool {
        let from_seq = self.thread_watermarks.get(thread_id).copied();
        self.replay_thread_history_from(thread_id, from_seq, false)
            .await
    }

    async fn retry_pending_agent_group_results_if_due(&mut self) -> bool {
        let now = Instant::now();
        if self
            .last_group_result_retry
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return false;
        }
        self.last_group_result_retry = Some(now);

        let thread_ids: Vec<String> = self
            .ui
            .threads
            .iter()
            .filter(|thread| thread_is_done(&thread.state))
            .map(|thread| thread.thread_id.clone())
            .collect();
        if thread_ids.is_empty() {
            return false;
        }

        let before = self.ui.group_chat.messages.len();
        for thread_id in thread_ids {
            self.record_agent_group_result_if_done(&thread_id).await;
        }
        self.ui.group_chat.messages.len() != before
    }

    async fn hydrate_thread_if_needed(&mut self, thread_id: &str) -> bool {
        if self.hydrated_threads.contains(thread_id) {
            return false;
        }
        self.replay_thread_history_from(thread_id, None, true).await
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

    fn mark_ingest_applied(&mut self, frame: &minos_protocol::LocalIngestFrame) -> bool {
        let thread_id = frame.thread_id.as_str();
        let seq = frame.seq;
        let fingerprint = ingest_fingerprint(frame);
        if self.applied_ingest_fingerprints.contains(&fingerprint) {
            if seq > 0 {
                let watermark = self
                    .thread_watermarks
                    .entry(thread_id.to_owned())
                    .or_insert(0);
                *watermark = (*watermark).max(seq);
            }
            return false;
        }

        if seq > 0 {
            let watermark = self
                .thread_watermarks
                .entry(thread_id.to_owned())
                .or_insert(0);
            if seq <= *watermark {
                return false;
            }
            *watermark = seq;
        }

        self.applied_ingest_fingerprints.insert(fingerprint);
        true
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
                self.ui.focus = Focus::RoomInput;
                self.sync_input_agent_picker();
                true
            }
            ManagerEvent::ThreadStateChanged { thread_id, new, .. } => {
                let is_terminal_for_stream = !matches!(
                    new,
                    ThreadState::Starting | ThreadState::Running { .. } | ThreadState::Resuming
                );
                let thread_agent = self
                    .ui
                    .threads
                    .iter()
                    .find(|t| t.thread_id == thread_id)
                    .map(|thread| thread.agent);
                if let Some(entry) = self
                    .ui
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == thread_id)
                {
                    entry.state = new;
                }
                if is_terminal_for_stream {
                    if let Some(chat) = self.ui.chat_states.get_mut(&thread_id) {
                        chat.finish_all_streaming();
                    }
                }
                if thread_agent != Some(AgentName::Opencode) {
                    self.record_agent_group_result_if_done(&thread_id).await;
                }
                true
            }
            ManagerEvent::ThreadClosed { thread_id, reason } => {
                let thread_agent = self
                    .ui
                    .threads
                    .iter()
                    .find(|t| t.thread_id == thread_id)
                    .map(|thread| thread.agent);
                if let Some(entry) = self
                    .ui
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == thread_id)
                {
                    entry.state = ThreadState::Closed { reason };
                }
                if let Some(chat) = self.ui.chat_states.get_mut(&thread_id) {
                    chat.finish_all_streaming();
                }
                if thread_agent != Some(AgentName::Opencode) {
                    self.record_agent_group_result_if_done(&thread_id).await;
                }
                true
            }
            ManagerEvent::InstanceCrashed {
                reason,
                affected_threads,
                ..
            } => {
                for tid in affected_threads {
                    if let Some(entry) = self.ui.threads.iter_mut().find(|t| t.thread_id == tid) {
                        entry.state = ThreadState::Suspended {
                            reason: reason.clone(),
                        };
                    }
                    if let Some(chat) = self.ui.chat_states.get_mut(&tid) {
                        chat.finish_all_streaming();
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
            KeyCode::PageUp => return self.page_up_active_pane(),
            KeyCode::PageDown => return self.page_down_active_pane(),
            KeyCode::Home if !is_input_focus(self.ui.focus) => return self.home_active_pane(),
            KeyCode::End if !is_input_focus(self.ui.focus) => return self.end_active_pane(),
            KeyCode::Char('n')
                if !matches!(self.ui.focus, Focus::RoomInput | Focus::AgentInput) =>
            {
                return self.open_agent_picker();
            }
            _ => {}
        }

        match self.ui.focus {
            Focus::RoomInput => self.handle_room_input_key(key).await,
            Focus::AgentInput => self.handle_agent_input_key(key).await,
            Focus::RoomList => self.handle_room_list_key(key).await,
            Focus::RoomChat => self.handle_room_chat_key(key).await,
            Focus::AgentList => self.handle_agent_list_key(key).await,
            Focus::AgentChat => self.handle_agent_chat_key(key).await,
        }
    }

    fn handle_paste(&mut self, text: String) -> bool {
        let text = normalize_pasted_text(text.as_str());
        if text.is_empty() {
            return false;
        }

        match self.ui.focus {
            Focus::RoomInput => {
                self.ui.room_input.insert_str(text.as_str());
                self.sync_input_agent_picker();
                true
            }
            Focus::AgentInput => {
                self.ui.agent_input.insert_str(text.as_str());
                true
            }
            _ => false,
        }
    }

    async fn handle_room_input_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.ui.room_input.insert_char('\n');
                    self.sync_input_agent_picker();
                    true
                } else if self.ui.room_input.has_agent_picker() {
                    let candidates = self.ui.room_agent_mention_candidates();
                    self.ui
                        .room_input
                        .accept_agent_completion(candidates.as_slice());
                    self.sync_input_agent_picker();
                    true
                } else {
                    self.submit_room_input().await
                }
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let changed = self.ui.room_input.move_word_left();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let changed = self.ui.room_input.move_word_right();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Left => {
                let changed = self.ui.room_input.move_left();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Right => {
                let changed = self.ui.room_input.move_right();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let changed = self.ui.room_input.move_to_start();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Home => {
                let changed = self.ui.room_input.move_line_start();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let changed = self.ui.room_input.move_to_end();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::End => {
                let changed = self.ui.room_input.move_line_end();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let changed = match c.to_ascii_lowercase() {
                    'a' => self.ui.room_input.move_line_start(),
                    'b' => self.ui.room_input.move_left(),
                    'e' => self.ui.room_input.move_line_end(),
                    'f' => self.ui.room_input.move_right(),
                    'k' => self.ui.room_input.delete_to_line_end(),
                    'u' => self.ui.room_input.delete_to_line_start(),
                    'w' => self.ui.room_input.delete_prev_word(),
                    _ => false,
                };
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let changed = match c.to_ascii_lowercase() {
                    'b' => self.ui.room_input.move_word_left(),
                    'd' => self.ui.room_input.delete_next_word(),
                    'f' => self.ui.room_input.move_word_right(),
                    _ => false,
                };
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Char(c) if is_text_input_key(key) => {
                self.ui.room_input.insert_char(c);
                self.sync_input_agent_picker();
                true
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                let changed = self.ui.room_input.delete_prev_word();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Backspace => {
                self.ui.room_input.backspace();
                self.sync_input_agent_picker();
                true
            }
            KeyCode::Up if self.ui.room_input.has_agent_picker() => {
                self.ui.room_input.select_previous_agent();
                true
            }
            KeyCode::Down if self.ui.room_input.has_agent_picker() => {
                self.ui.room_input.select_next_agent();
                true
            }
            KeyCode::Up => {
                let changed = self.ui.room_input.move_up();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Down => {
                let changed = self.ui.room_input.move_down();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Delete
                if key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                let changed = self.ui.room_input.delete_next_word();
                self.sync_input_agent_picker();
                changed
            }
            KeyCode::Delete => {
                self.ui.room_input.delete_forward();
                self.sync_input_agent_picker();
                true
            }
            KeyCode::Tab => {
                self.cycle_focus();
                true
            }
            KeyCode::Esc => {
                if self.ui.room_input.has_agent_picker() {
                    self.ui.room_input.clear_agent_picker();
                    true
                } else {
                    self.handle_escape()
                }
            }
            _ => false,
        }
    }

    async fn handle_agent_input_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.ui.agent_input.insert_char('\n');
                    true
                } else {
                    self.submit_agent_input().await
                }
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.ui.agent_input.move_word_left()
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.ui.agent_input.move_word_right()
            }
            KeyCode::Left => self.ui.agent_input.move_left(),
            KeyCode::Right => self.ui.agent_input.move_right(),
            KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.ui.agent_input.move_to_start()
            }
            KeyCode::Home => self.ui.agent_input.move_line_start(),
            KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.ui.agent_input.move_to_end()
            }
            KeyCode::End => self.ui.agent_input.move_line_end(),
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                match c.to_ascii_lowercase() {
                    'a' => self.ui.agent_input.move_line_start(),
                    'b' => self.ui.agent_input.move_left(),
                    'e' => self.ui.agent_input.move_line_end(),
                    'f' => self.ui.agent_input.move_right(),
                    'k' => self.ui.agent_input.delete_to_line_end(),
                    'u' => self.ui.agent_input.delete_to_line_start(),
                    'w' => self.ui.agent_input.delete_prev_word(),
                    _ => false,
                }
            }
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                match c.to_ascii_lowercase() {
                    'b' => self.ui.agent_input.move_word_left(),
                    'd' => self.ui.agent_input.delete_next_word(),
                    'f' => self.ui.agent_input.move_word_right(),
                    _ => false,
                }
            }
            KeyCode::Char(c) if is_text_input_key(key) => {
                self.ui.agent_input.insert_char(c);
                true
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                self.ui.agent_input.delete_prev_word()
            }
            KeyCode::Backspace => {
                self.ui.agent_input.backspace();
                true
            }
            KeyCode::Up => self.ui.agent_input.move_up(),
            KeyCode::Down => self.ui.agent_input.move_down(),
            KeyCode::Delete
                if key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                self.ui.agent_input.delete_next_word()
            }
            KeyCode::Delete => {
                self.ui.agent_input.delete_forward();
                true
            }
            KeyCode::Tab => {
                self.cycle_focus();
                true
            }
            KeyCode::Esc => self.handle_escape(),
            _ => false,
        }
    }

    async fn handle_room_list_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                if let Some(selected) = self.ui.selected_room {
                    self.select_room(selected.saturating_sub(1));
                }
                true
            }
            KeyCode::Down => {
                if let Some(selected) = self.ui.selected_room {
                    let last = self.ui.rooms.len().saturating_sub(1);
                    self.select_room((selected + 1).min(last));
                }
                true
            }
            KeyCode::Enter => {
                self.ui.focus = Focus::RoomChat;
                true
            }
            KeyCode::Tab => {
                self.cycle_focus();
                true
            }
            KeyCode::Esc => self.handle_escape(),
            _ => false,
        }
    }

    async fn handle_room_chat_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => self.scroll_group_chat_up(1),
            KeyCode::Down => self.scroll_group_chat_down(1),
            KeyCode::PageUp => self.scroll_group_chat_up(5),
            KeyCode::PageDown => self.scroll_group_chat_down(5),
            KeyCode::Home => self.scroll_group_chat_to_top(),
            KeyCode::End => self.scroll_group_chat_to_bottom(),
            KeyCode::Enter => {
                self.ui.focus = Focus::RoomInput;
                true
            }
            KeyCode::Tab => {
                self.cycle_focus();
                true
            }
            KeyCode::Esc => self.handle_escape(),
            _ => false,
        }
    }

    async fn handle_agent_list_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                self.select_previous_thread();
                true
            }
            KeyCode::Down => {
                self.select_next_thread();
                true
            }
            KeyCode::Enter => {
                if self.ui.selected_thread.is_some() {
                    self.ui.agent_detail_visible = true;
                    self.ui.focus = Focus::AgentChat;
                    return true;
                }
                false
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

    async fn handle_agent_chat_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => self.scroll_current_chat_up(1),
            KeyCode::Down => self.scroll_current_chat_down(1),
            KeyCode::PageUp => self.scroll_current_chat_up(5),
            KeyCode::PageDown => self.scroll_current_chat_down(5),
            KeyCode::Home => self.scroll_current_chat_to_top(),
            KeyCode::End => self.scroll_current_chat_to_bottom(),
            KeyCode::Enter => {
                self.ui.focus = Focus::AgentInput;
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

        if rect_contains(self.ui.panel_areas.room_list, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.ui.focus = Focus::RoomList;
                    self.select_previous_room();
                    true
                }
                MouseEventKind::ScrollDown => {
                    self.ui.focus = Focus::RoomList;
                    self.select_next_room();
                    true
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::RoomList;
                    if let Some(index) = clicked_thread_index(
                        self.ui.panel_areas.room_list,
                        &self.ui.room_list_state,
                        mouse.row,
                        self.ui.rooms.len(),
                    ) {
                        self.select_room(index);
                    }
                    true
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.room_chat, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.ui.focus = Focus::RoomChat;
                    self.scroll_group_chat_up(3)
                }
                MouseEventKind::ScrollDown => {
                    self.ui.focus = Focus::RoomChat;
                    self.scroll_group_chat_down(3)
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::RoomChat;
                    true
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.agent_list, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.ui.focus = Focus::AgentList;
                    self.select_previous_thread();
                    true
                }
                MouseEventKind::ScrollDown => {
                    self.ui.focus = Focus::AgentList;
                    self.select_next_thread();
                    true
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::AgentList;
                    if let Some(index) = clicked_thread_index(
                        self.ui.panel_areas.agent_list,
                        &self.ui.agent_list_state,
                        mouse.row,
                        self.ui.threads.len(),
                    ) {
                        self.select_thread(index);
                        self.ui.agent_detail_visible = true;
                    }
                    true
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.agent_chat, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.ui.focus = Focus::AgentChat;
                    self.sync_input_agent_picker();
                    self.scroll_current_chat_up(3)
                }
                MouseEventKind::ScrollDown => {
                    self.ui.focus = Focus::AgentChat;
                    self.sync_input_agent_picker();
                    self.scroll_current_chat_down(3)
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::AgentChat;
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

        if rect_contains(self.ui.panel_areas.room_input, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::RoomInput;
                    self.sync_input_agent_picker();
                    true
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.agent_input, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.ui.focus = Focus::AgentInput;
                    true
                }
                _ => false,
            };
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.ui.focus = Focus::RoomChat;
                self.scroll_group_chat_up(3)
            }
            MouseEventKind::ScrollDown => {
                self.ui.focus = Focus::RoomChat;
                self.scroll_group_chat_down(3)
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
        let content_area = chat_content_area(self.ui.panel_areas.agent_chat);
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
        let content_area = chat_content_area(self.ui.panel_areas.agent_chat);
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
        self.thread_watermarks.remove(thread_id);
        self.applied_ingest_fingerprints
            .retain(|fingerprint| !fingerprint.starts_with(&format!("{thread_id}:")));

        if self.ui.threads.is_empty() {
            self.ui.selected_thread = None;
            self.ui.agent_list_state.select(None);
            self.ui.agent_detail_visible = false;
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
        self.ui.agent_list_state.select(Some(index));
    }

    fn select_room(&mut self, index: usize) {
        self.ui.selected_room = Some(index);
        self.ui.room_list_state.select(Some(index));
    }

    fn select_previous_room(&mut self) {
        if let Some(selected) = self.ui.selected_room {
            self.select_room(selected.saturating_sub(1));
        }
    }

    fn select_next_room(&mut self) {
        if let Some(selected) = self.ui.selected_room {
            let last = self.ui.rooms.len().saturating_sub(1);
            self.select_room((selected + 1).min(last));
        }
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
        let order = if self.ui.agent_detail_visible {
            [
                Focus::RoomChat,
                Focus::AgentList,
                Focus::AgentChat,
                Focus::RoomInput,
                Focus::AgentInput,
            ]
            .as_slice()
        } else {
            [
                Focus::RoomList,
                Focus::RoomChat,
                Focus::AgentList,
                Focus::RoomInput,
            ]
            .as_slice()
        };
        let current = order
            .iter()
            .position(|focus| *focus == self.ui.focus)
            .unwrap_or(0);
        self.ui.focus = order[(current + 1) % order.len()];
        self.sync_input_agent_picker();
    }

    fn handle_escape(&mut self) -> bool {
        if self.ui.agent_picker.is_some() {
            self.ui.agent_picker = None;
            return true;
        }

        if self.ui.room_input.has_agent_picker() {
            self.ui.room_input.clear_agent_picker();
            return true;
        }

        if self.ui.agent_detail_visible
            && matches!(self.ui.focus, Focus::AgentChat | Focus::AgentInput)
        {
            self.ui.agent_detail_visible = false;
            self.ui.focus = Focus::AgentList;
            self.sync_input_agent_picker();
            return true;
        }

        let fallback_focus = if self.ui.agent_detail_visible {
            Focus::RoomChat
        } else {
            Focus::RoomList
        };

        if self.ui.focus != fallback_focus {
            self.ui.focus = fallback_focus;
            self.sync_input_agent_picker();
            return true;
        }

        false
    }

    fn scroll_group_chat_up(&mut self, lines: u16) -> bool {
        self.ui.group_chat.scroll_up(lines);
        true
    }

    fn scroll_group_chat_down(&mut self, lines: u16) -> bool {
        self.ui.group_chat.scroll_down(lines);
        true
    }

    fn scroll_group_chat_to_top(&mut self) -> bool {
        self.ui.group_chat.auto_scroll = false;
        self.ui.group_chat.scroll_offset = 0;
        true
    }

    fn scroll_group_chat_to_bottom(&mut self) -> bool {
        self.ui.group_chat.auto_scroll = true;
        self.ui.group_chat.scroll_offset = 0;
        true
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
            chat.toggle_tool_expansion();
            true
        } else {
            false
        }
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

    fn page_up_active_pane(&mut self) -> bool {
        match self.ui.focus {
            Focus::RoomChat => self.scroll_group_chat_up(5),
            Focus::AgentChat => self.scroll_current_chat_up(5),
            _ => false,
        }
    }

    fn page_down_active_pane(&mut self) -> bool {
        match self.ui.focus {
            Focus::RoomChat => self.scroll_group_chat_down(5),
            Focus::AgentChat => self.scroll_current_chat_down(5),
            _ => false,
        }
    }

    fn home_active_pane(&mut self) -> bool {
        match self.ui.focus {
            Focus::RoomChat => self.scroll_group_chat_to_top(),
            Focus::AgentChat => self.scroll_current_chat_to_top(),
            _ => false,
        }
    }

    fn end_active_pane(&mut self) -> bool {
        match self.ui.focus {
            Focus::RoomChat => self.scroll_group_chat_to_bottom(),
            Focus::AgentChat => self.scroll_current_chat_to_bottom(),
            _ => false,
        }
    }

    fn active_room_target(&self) -> Option<MessageTarget> {
        if let Some(thread) = self
            .ui
            .selected_thread
            .and_then(|index| self.ui.threads.get(index))
        {
            return Some(if thread_can_receive_message(&thread.state) {
                MessageTarget::ExistingThread(thread.thread_id.clone())
            } else {
                MessageTarget::NewAgent(thread.agent)
            });
        }

        let mut candidates = self
            .ui
            .threads
            .iter()
            .filter(|thread| thread_can_receive_message(&thread.state));
        let first = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        Some(MessageTarget::ExistingThread(first.thread_id.clone()))
    }

    async fn submit_room_input(&mut self) -> bool {
        let text = self.ui.room_input.content.clone();
        if text.trim().is_empty() {
            self.ui.room_input.take_input();
            return true;
        }

        if let Some((target, body)) = parse_agent_routing(text.as_str()) {
            self.ui.room_input.take_input();
            if let Some(thread_short_id) = target.thread_short_id {
                return self
                    .dispatch_prompt_to_existing_agent(
                        target.agent,
                        thread_short_id,
                        body,
                        text.trim().to_owned(),
                    )
                    .await;
            }
            if body.trim().is_empty() {
                return self
                    .invite_agent_to_room(target.agent, text.trim().to_owned())
                    .await;
            }
            return self
                .dispatch_prompt_to_agent(target.agent, body, text.trim().to_owned())
                .await;
        }

        let Some(target) = self.active_room_target() else {
            self.ui
                .set_error("No agent selected. Use @agent or pick one from Agents.".into());
            return true;
        };
        self.ui.room_input.take_input();
        match target {
            MessageTarget::ExistingThread(thread_id) => {
                let group_text = self.group_user_text_for_thread(&thread_id, text.as_str());
                self.send_text_to_thread(thread_id, text, group_text).await
            }
            MessageTarget::NewAgent(agent) => {
                let group_text = format!("@{} {}", agent.bin_name(), text.trim());
                self.dispatch_prompt_to_agent(agent, text, group_text).await
            }
        }
    }

    async fn submit_agent_input(&mut self) -> bool {
        let text = self.ui.agent_input.content.clone();
        if text.trim().is_empty() {
            self.ui.agent_input.take_input();
            return true;
        }

        let Some(thread) = self
            .ui
            .selected_thread
            .and_then(|index| self.ui.threads.get(index))
        else {
            self.ui
                .set_error("No agent selected for direct chat.".into());
            return true;
        };
        let thread_id = thread.thread_id.clone();
        let agent = thread.agent;
        if !thread_can_receive_message(&thread.state) {
            self.ui.agent_input.take_input();
            let group_text = format!("@{} {}", agent.bin_name(), text.trim());
            return self.dispatch_prompt_to_agent(agent, text, group_text).await;
        }
        if let Some(pending) = self
            .ui
            .chat_states
            .get(&thread_id)
            .and_then(ChatState::active_pending_request)
            .cloned()
        {
            self.ui.agent_input.take_input();
            return self
                .submit_pending_agent_request(thread_id, pending.kind, text)
                .await;
        }
        let group_text = self.group_user_text_for_thread(&thread_id, text.as_str());
        self.ui.agent_input.take_input();
        self.send_text_to_thread(thread_id, text, group_text).await
    }

    async fn submit_pending_agent_request(
        &mut self,
        thread_id: String,
        pending: PendingAgentRequestKind,
        text: String,
    ) -> bool {
        let request_id = match &pending {
            PendingAgentRequestKind::CodexUserInput { request_id, .. }
            | PendingAgentRequestKind::CodexApproval { request_id, .. } => request_id.clone(),
            PendingAgentRequestKind::OpencodePermission { permission_id, .. } => {
                permission_id.clone()
            }
            PendingAgentRequestKind::OpencodeQuestion { question_id, .. } => question_id.clone(),
        };

        let result = match pending {
            PendingAgentRequestKind::CodexUserInput {
                request_id,
                question_ids,
            } => {
                let ids = if question_ids.is_empty() {
                    vec!["answer".to_owned()]
                } else {
                    question_ids
                };
                let decision = codex_user_input_decision(ids.as_slice(), text.as_str());
                self.backend
                    .send_approval_decision(&request_id, &thread_id, decision)
                    .await
            }
            PendingAgentRequestKind::CodexApproval { request_id, method } => {
                let decision = codex_approval_decision(&method, text.as_str());
                self.backend
                    .send_approval_decision(&request_id, &thread_id, decision)
                    .await
            }
            PendingAgentRequestKind::OpencodePermission {
                permission_id,
                approve_response,
                decline_response,
            } => {
                let response = opencode_permission_response(
                    text.as_str(),
                    &approve_response,
                    &decline_response,
                );
                self.backend
                    .respond_opencode_permission(&thread_id, &permission_id, &response)
                    .await
            }
            PendingAgentRequestKind::OpencodeQuestion {
                question_id,
                questions,
            } => {
                let answers = opencode_question_answers(questions.as_slice(), text.as_str());
                self.backend
                    .respond_opencode_question(&thread_id, &question_id, answers)
                    .await
            }
        };

        match result {
            Ok(()) => {
                if let Some(chat) = self.ui.chat_states.get_mut(&thread_id) {
                    chat.resolve_pending_request(&request_id);
                }
            }
            Err(error) => {
                self.ui
                    .set_error(format!("Failed to answer agent request: {error}"));
            }
        }
        true
    }

    async fn invite_agent_to_room(&mut self, agent: AgentName, group_text: String) -> bool {
        let thread_id = match self.start_new_thread(agent).await {
            Ok(thread_id) => thread_id,
            Err(error) => {
                self.ui.set_error(error);
                return true;
            }
        };

        if let Some(index) = self
            .ui
            .threads
            .iter()
            .position(|thread| thread.thread_id == thread_id)
        {
            self.select_thread(index);
        }
        self.record_user_group_message(&thread_id, group_text).await;
        true
    }

    async fn dispatch_prompt_to_existing_agent(
        &mut self,
        agent: AgentName,
        thread_short_id: String,
        text: String,
        group_text: String,
    ) -> bool {
        let Some(thread_id) = self.thread_id_for_agent_short_id(agent, &thread_short_id) else {
            self.ui.set_error(format!(
                "No existing {} session matches #{}",
                agent.bin_name(),
                thread_short_id
            ));
            return true;
        };
        if let Some(thread) = self
            .ui
            .threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)
            .filter(|thread| !thread_can_receive_message(&thread.state))
        {
            self.ui.set_error(format!(
                "{} session #{} is closed. Use @{} to start a new session.",
                thread.agent.bin_name(),
                short_thread_id(&thread.thread_id),
                thread.agent.bin_name()
            ));
            return true;
        }

        if text.trim().is_empty() {
            if let Some(index) = self
                .ui
                .threads
                .iter()
                .position(|thread| thread.thread_id == thread_id)
            {
                self.select_thread(index);
            }
            self.record_user_group_message(&thread_id, group_text).await;
            return true;
        }

        self.send_text_to_thread(thread_id, text, Some(group_text))
            .await
    }

    async fn dispatch_prompt_to_agent(
        &mut self,
        agent: AgentName,
        text: String,
        group_text: String,
    ) -> bool {
        if let Some(tx) = self.event_tx.clone() {
            if let Some(error) = self.agent_unavailability_error(agent) {
                self.ui.set_error(error);
                return true;
            }
            self.record_user_group_message_for_agent(agent, group_text)
                .await;
            let backend = Arc::clone(&self.backend);
            let workspace = self.workspace.clone();
            tokio::spawn(async move {
                match backend.start_agent(agent, workspace).await {
                    Ok(outcome) => {
                        let _ = tx.send(AppEvent::AgentStartedForPrompt {
                            agent,
                            thread_id: outcome.thread_id,
                            cwd: outcome.cwd,
                            text,
                        });
                    }
                    Err(error) => {
                        let error = format!(
                            "Failed to start {}: {}",
                            agent.bin_name(),
                            format_error_chain(&error)
                        );
                        tracing::warn!(
                            target: "minos_tui::app",
                            error = %error,
                            agent = %agent.bin_name(),
                            "background start_agent failed"
                        );
                        let _ = tx.send(AppEvent::SendMessageFailed {
                            thread_id: agent.bin_name().to_owned(),
                            error,
                        });
                    }
                }
            });
            return true;
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

        if let Some(group_text) = group_text {
            self.record_user_group_message(&thread_id, group_text).await;
        }

        if let Some(tx) = self.event_tx.clone() {
            let backend = Arc::clone(&self.backend);
            tokio::spawn(async move {
                if let Err(e) = backend.resume_thread(&thread_id).await {
                    tracing::debug!(
                        target: "minos_tui::app",
                        error = %e,
                        thread_id = %thread_id,
                        "resume_thread failed or not needed"
                    );
                }
                if let Err(error) = backend.send_message(&thread_id, &text).await {
                    let error = format_error_chain(&error);
                    tracing::warn!(
                        target: "minos_tui::app",
                        error = %error,
                        thread_id = %thread_id,
                        "background send_message failed"
                    );
                    let _ = tx.send(AppEvent::SendMessageFailed { thread_id, error });
                }
            });
            return true;
        }

        if let Err(e) = self.backend.resume_thread(&thread_id).await {
            tracing::debug!(
                target: "minos_tui::app",
                error = %e,
                thread_id = %thread_id,
                "resume_thread failed or not needed"
            );
        }
        if let Err(error) = self.backend.send_message(&thread_id, &text).await {
            self.ui.set_error(format!(
                "Failed to send message: {}",
                format_error_chain(&error)
            ));
        }
        true
    }

    async fn start_new_thread(&mut self, agent: AgentName) -> Result<String, String> {
        if let Some(error) = self.agent_unavailability_error(agent) {
            return Err(error);
        }
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
                Err(error) => Err(format!(
                    "Failed to start {}: {}",
                    agent.bin_name(),
                    format_error_chain(&error)
                )),
            },
            AgentStatus::Missing => Err(format!("{} is not installed on PATH", agent.bin_name())),
            AgentStatus::Error { reason } => {
                Err(format!("{} is unavailable: {reason}", agent.bin_name()))
            }
        }
    }

    fn agent_unavailability_error(&self, agent: AgentName) -> Option<String> {
        let Some(descriptor) = self.ui.status.agents.iter().find(|desc| desc.name == agent) else {
            return Some(format!("Unknown agent: {}", agent.bin_name()));
        };
        match &descriptor.status {
            AgentStatus::Ok => None,
            AgentStatus::Missing => Some(format!("{} is not installed on PATH", agent.bin_name())),
            AgentStatus::Error { reason } => {
                Some(format!("{} is unavailable: {reason}", agent.bin_name()))
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
            self.ui.focus = Focus::RoomInput;
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
        self.ui.focus = Focus::RoomInput;
        self.sync_input_agent_picker();
    }

    fn thread_id_for_agent_short_id(&self, agent: AgentName, short_id: &str) -> Option<String> {
        let short_id = short_id.to_ascii_lowercase();
        self.ui
            .threads
            .iter()
            .find(|thread| {
                thread.agent == agent
                    && (short_thread_id(&thread.thread_id).to_ascii_lowercase() == short_id
                        || thread.thread_id.to_ascii_lowercase().starts_with(&short_id))
            })
            .map(|thread| thread.thread_id.clone())
    }

    fn sync_input_agent_picker(&mut self) {
        let candidates = self.ui.room_agent_mention_candidates();
        self.ui.room_input.sync_agent_picker(
            candidates.as_slice(),
            matches!(self.ui.focus, Focus::RoomInput),
        );
    }

    async fn load_group_chat_history(&mut self) {
        if self.load_group_chat_history_from_backend().await {
            return;
        }

        let sessions = match self.group_chat_store.list_agent_sessions().await {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    "failed to load group chat agent sessions"
                );
                Vec::new()
            }
        };
        match self.group_chat_store.load_recent(500).await {
            Ok(messages) => {
                self.restore_agent_entries_from_group_sessions(sessions.as_slice());
                self.restore_agent_entries_from_group_messages(messages.as_slice());
                self.ui.group_chat.replace_messages(messages);
            }
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

    async fn load_group_chat_history_from_backend(&mut self) -> bool {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            return false;
        }

        let room_id = self.group_chat_store.room_id().to_owned();
        match self
            .backend
            .read_group_chat(&room_id, None, None, 500)
            .await
        {
            Ok(messages) => {
                self.restore_agent_entries_from_group_messages(messages.as_slice());
                self.ui.group_chat.replace_messages(messages);
                true
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    "failed to load daemon group chat history"
                );
                false
            }
        }
    }

    async fn refresh_group_chat_from_backend(&mut self) -> bool {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            return false;
        }
        let after_seq = self.ui.group_chat.last_seq();
        let room_id = self.group_chat_store.room_id().to_owned();
        let messages = match self
            .backend
            .read_group_chat(&room_id, Some(after_seq), None, 100)
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                tracing::debug!(
                    target: "minos_tui::app",
                    error = %error,
                    after_seq,
                    "failed to refresh daemon group chat"
                );
                return false;
            }
        };
        if messages.is_empty() {
            return false;
        }
        self.restore_agent_entries_from_group_messages(messages.as_slice());
        self.ui.group_chat.merge_messages(messages);
        true
    }

    fn restore_agent_entries_from_group_sessions(
        &mut self,
        sessions: &[minos_chat_store::ChatAgentSession],
    ) {
        for session in sessions {
            self.restore_agent_entry(
                session.agent,
                &session.thread_id,
                PathBuf::from(&session.workspace_root),
            );
        }
        if self.ui.selected_thread.is_none() && !self.ui.threads.is_empty() {
            self.select_thread(0);
        }
    }

    fn restore_agent_entries_from_group_messages(&mut self, messages: &[LocalGroupChatMessage]) {
        for message in messages {
            let (Some(agent), Some(thread_id)) = (message.agent, message.thread_id.as_deref())
            else {
                continue;
            };
            if thread_id.is_empty() {
                continue;
            }

            let workspace = message
                .workspace
                .as_deref()
                .filter(|workspace| !workspace.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| self.workspace.clone());
            self.restore_agent_entry(agent, thread_id, workspace);
        }

        if self.ui.selected_thread.is_none() && !self.ui.threads.is_empty() {
            self.select_thread(0);
        }
    }

    fn restore_agent_entry(&mut self, agent: AgentName, thread_id: &str, workspace: PathBuf) {
        if let Some(entry) = self
            .ui
            .threads
            .iter_mut()
            .find(|entry| entry.thread_id == thread_id)
        {
            entry.agent = agent;
            entry.workspace = workspace;
            self.ensure_chat_state_agent(thread_id, agent);
            return;
        }

        self.ui.threads.push(ThreadEntry {
            thread_id: thread_id.to_owned(),
            agent,
            workspace,
            state: ThreadState::Suspended {
                reason: minos_agent_runtime::PauseReason::DaemonRestart,
            },
        });
        self.ensure_chat_state_agent(thread_id, agent);
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

    async fn handle_mcp_tool_call(
        &mut self,
        request: SocketRequest,
    ) -> anyhow::Result<SocketResponse> {
        match request {
            SocketRequest::Ping => Ok(SocketResponse::Pong),
            SocketRequest::ListRoomMessages {
                room_id,
                before_seq,
                limit,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let page = self
                    .group_chat_store
                    .list_messages_desc(before_seq, limit)
                    .await?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(page)?),
                })
            }
            SocketRequest::DelegateToAgent {
                room_id,
                source_agent,
                target_agent,
                prompt,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let target_agent = parse_agent_name(&target_agent)
                    .ok_or_else(|| anyhow::anyhow!("unknown agent: {target_agent}"))?;
                if let Some(error) = self.agent_unavailability_error(target_agent) {
                    anyhow::bail!(error);
                }
                let prompt = prompt.trim().to_owned();
                anyhow::ensure!(!prompt.is_empty(), "delegate_to_agent prompt is empty");
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?;
                let delegation = self
                    .group_chat_store
                    .create_delegation(source_agent, target_agent, prompt.clone(), None)
                    .await?;
                let group_text = format!("@{} {prompt}", target_agent.bin_name());
                self.dispatch_prompt_to_agent(target_agent, prompt, group_text)
                    .await;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::json!({
                        "accepted": true,
                        "target_agent": target_agent.bin_name(),
                        "delegation": delegation,
                    })),
                })
            }
            SocketRequest::GetDelegationStatus {
                room_id,
                delegation_id,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let delegation = self
                    .group_chat_store
                    .get_delegation(&delegation_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("delegation not found: {delegation_id}"))?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(delegation)?),
                })
            }
            SocketRequest::CancelDelegation {
                room_id,
                delegation_id,
                reason,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let delegation = self
                    .group_chat_store
                    .cancel_delegation(&delegation_id, reason)
                    .await?;
                if let Some(thread_id) = delegation.thread_id.as_deref() {
                    let _ = self.backend.interrupt_thread(thread_id).await;
                }
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(delegation)?),
                })
            }
            SocketRequest::AskUserQuestion {
                room_id,
                source_agent,
                question,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let body = question.trim();
                anyhow::ensure!(!body.is_empty(), "ask_user_question question is empty");
                let text = if body.starts_with("@user") {
                    body.to_owned()
                } else {
                    format!("@user {body}")
                };
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?;
                let message = self
                    .append_group_chat_message_result(LocalGroupChatMessage {
                        seq: 0,
                        message_id: String::new(),
                        created_at_ms: chrono::Utc::now().timestamp_millis(),
                        kind: LocalGroupChatMessageKind::AgentResult,
                        text,
                        agent: source_agent,
                        thread_id: None,
                        thread_short_id: None,
                        workspace: Some(self.workspace.display().to_string()),
                    })
                    .await?;
                let feedback = self
                    .group_chat_store
                    .create_user_feedback(source_agent, body.to_owned(), message.seq)
                    .await?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(feedback)?),
                })
            }
            SocketRequest::CheckUserFeedback {
                room_id,
                feedback_id,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let feedback = self
                    .group_chat_store
                    .check_user_feedback(&feedback_id)
                    .await?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(feedback)?),
                })
            }
            SocketRequest::PostRoomUpdate {
                room_id,
                source_agent,
                message,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let body = message.trim();
                anyhow::ensure!(!body.is_empty(), "post_room_update message is empty");
                let text = if body.starts_with("@user") {
                    body.to_owned()
                } else {
                    format!("@user {body}")
                };
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?;
                self.append_group_chat_message_result(LocalGroupChatMessage {
                    seq: 0,
                    message_id: String::new(),
                    created_at_ms: chrono::Utc::now().timestamp_millis(),
                    kind: LocalGroupChatMessageKind::AgentResult,
                    text,
                    agent: source_agent,
                    thread_id: None,
                    thread_short_id: None,
                    workspace: Some(self.workspace.display().to_string()),
                })
                .await?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::json!({
                        "accepted": true,
                    })),
                })
            }
            SocketRequest::ReactToMessage {
                room_id,
                source_agent,
                message_id,
                message_seq,
                emoji,
                action,
            } => {
                self.ensure_mcp_room(&room_id)?;
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?;
                let reaction = self
                    .group_chat_store
                    .react_to_message(source_agent, message_id, message_seq, emoji, action)
                    .await?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(reaction)?),
                })
            }
        }
    }

    fn ensure_mcp_room(&self, room_id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            room_id == self.group_chat_store.room_id(),
            "MCP request room_id does not match this TUI room"
        );
        Ok(())
    }

    async fn record_user_group_message(&mut self, thread_id: &str, text: String) {
        let Some(message) = self.group_message(thread_id, LocalGroupChatMessageKind::User, text)
        else {
            return;
        };
        self.append_group_chat_message(message).await;
    }

    async fn record_user_group_message_for_agent(&mut self, agent: AgentName, text: String) {
        self.append_group_chat_message(LocalGroupChatMessage {
            seq: 0,
            message_id: String::new(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            kind: LocalGroupChatMessageKind::User,
            text,
            agent: Some(agent),
            thread_id: None,
            thread_short_id: None,
            workspace: Some(self.workspace.display().to_string()),
        })
        .await;
    }

    async fn record_agent_group_result_if_done(&mut self, thread_id: &str) {
        self.record_agent_group_result(thread_id, false).await;
    }

    async fn record_agent_group_result_if_ingest_done(
        &mut self,
        thread_id: &str,
        allow_ingest_done: bool,
    ) {
        let is_opencode = self
            .ui
            .threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)
            .is_some_and(|thread| thread.agent == AgentName::Opencode);
        if is_opencode && !allow_ingest_done {
            return;
        }
        self.record_agent_group_result(thread_id, allow_ingest_done)
            .await;
    }

    async fn record_agent_group_result(&mut self, thread_id: &str, allow_ingest_done: bool) {
        let Some(thread) = self
            .ui
            .threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)
        else {
            return;
        };
        if !thread_is_done(&thread.state) && !allow_ingest_done {
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
        if self.group_chat_has_agent_result(thread_id, &text) {
            self.recorded_agent_results
                .insert(thread_id.to_owned(), message_key);
            return;
        }
        let Some(message) =
            self.group_message(thread_id, LocalGroupChatMessageKind::AgentResult, text)
        else {
            return;
        };
        if self.append_group_chat_message(message).await {
            self.recorded_agent_results
                .insert(thread_id.to_owned(), message_key);
        }
    }

    fn group_chat_has_agent_result(&self, thread_id: &str, text: &str) -> bool {
        self.ui.group_chat.messages.iter().any(|message| {
            message.kind == LocalGroupChatMessageKind::AgentResult
                && message.thread_id.as_deref() == Some(thread_id)
                && message.text == text
        })
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

    async fn append_group_chat_message(&mut self, message: LocalGroupChatMessage) -> bool {
        match self.append_group_chat_message_result(message).await {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    "failed to append group chat message"
                );
                self.ui
                    .set_error(format!("Failed to record group chat message: {error}"));
                false
            }
        }
    }

    async fn append_group_chat_message_result(
        &mut self,
        message: LocalGroupChatMessage,
    ) -> anyhow::Result<LocalGroupChatMessage> {
        let message = self.group_chat_store.append(message).await?;
        self.ui.group_chat.push_message(message.clone());
        Ok(message)
    }
}

fn is_text_input_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(_))
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

fn is_input_focus(focus: Focus) -> bool {
    matches!(focus, Focus::RoomInput | Focus::AgentInput)
}

fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn thread_can_receive_message(state: &ThreadState) -> bool {
    !matches!(state, ThreadState::Closed { .. })
}

fn format_error_chain(error: &anyhow::Error) -> String {
    let mut parts = Vec::new();
    for cause in error.chain() {
        let text = cause.to_string();
        if parts.last() != Some(&text) {
            parts.push(text);
        }
    }
    parts.join(": ")
}

fn workspace_room_id(workspace: &std::path::Path) -> String {
    minos_chat_store::room_id_for_workspace(workspace)
}

fn default_room_title(workspace: &std::path::Path) -> String {
    minos_chat_store::room_title_for_workspace(workspace)
}

#[cfg(not(test))]
fn default_group_chat_store(workspace: &std::path::Path) -> GroupChatStore {
    match GroupChatStore::default_for_runtime(workspace) {
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
fn default_group_chat_store(_workspace: &std::path::Path) -> GroupChatStore {
    GroupChatStore::disabled()
}

fn thread_is_done(state: &ThreadState) -> bool {
    matches!(state, ThreadState::Idle | ThreadState::Closed { .. })
}

fn frame_marks_agent_result_done(frame: &minos_protocol::LocalIngestFrame) -> bool {
    frame.ui_events.iter().any(|event| {
        matches!(
            event,
            minos_ui_protocol::UiEventMessage::MessageCompleted { .. }
                | minos_ui_protocol::UiEventMessage::ThreadClosed { .. }
        )
    })
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentRouteTarget {
    agent: AgentName,
    thread_short_id: Option<String>,
}

fn parse_agent_routing(text: &str) -> Option<(AgentRouteTarget, String)> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('@')?;
    let split_at = rest
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let target = parse_agent_route_target(&rest[..split_at])?;
    let body = rest[split_at..].trim_start().to_owned();
    Some((target, body))
}

fn parse_agent_route_target(value: &str) -> Option<AgentRouteTarget> {
    let (agent_name, thread_short_id) = match value.split_once('#') {
        Some((agent_name, thread_short_id)) if !thread_short_id.is_empty() => {
            (agent_name, Some(thread_short_id.to_owned()))
        }
        Some(_) => return None,
        None => (value, None),
    };
    Some(AgentRouteTarget {
        agent: parse_agent_name(agent_name)?,
        thread_short_id,
    })
}

fn parse_agent_name(value: &str) -> Option<AgentName> {
    let normalized = value.to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
}

fn ingest_fingerprint(frame: &minos_protocol::LocalIngestFrame) -> String {
    if frame.seq > 0 {
        return format!("{}:seq:{}", frame.thread_id, frame.seq);
    }
    let payload = serde_json::to_string(&frame.ui_events).unwrap_or_default();
    format!("{}:{payload}", frame.thread_id)
}

fn codex_user_input_decision(question_ids: &[String], text: &str) -> serde_json::Value {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut answers = serde_json::Map::new();
    for (index, question_id) in question_ids.iter().enumerate() {
        let answer = if question_ids.len() > 1 {
            lines.get(index).copied().unwrap_or_else(|| text.trim())
        } else {
            text.trim()
        };
        answers.insert(
            question_id.clone(),
            serde_json::json!({ "answers": [answer] }),
        );
    }
    serde_json::json!({ "answers": answers })
}

fn codex_approval_decision(method: &str, text: &str) -> serde_json::Value {
    let approved = is_affirmative(text);
    match method {
        "applyPatchApproval" | "execCommandApproval" => {
            serde_json::json!({ "decision": if approved { "approved" } else { "denied" } })
        }
        "item/permissions/requestApproval" => {
            serde_json::json!({ "permissions": {}, "scope": "turn" })
        }
        _ => serde_json::json!({ "decision": if approved { "accept" } else { "decline" } }),
    }
}

fn opencode_permission_response(
    text: &str,
    approve_response: &str,
    decline_response: &str,
) -> String {
    if is_affirmative(text) {
        approve_response.to_owned()
    } else {
        decline_response.to_owned()
    }
}

fn opencode_question_answers(questions: &[PendingQuestionSpec], text: &str) -> Vec<Vec<String>> {
    if questions.is_empty() {
        return vec![vec![text.trim().to_owned()]];
    }

    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let answer_text = if questions.len() > 1 {
                lines.get(index).copied().unwrap_or_else(|| text.trim())
            } else {
                text.trim()
            };
            parse_opencode_question_answer(question, answer_text)
        })
        .collect()
}

fn parse_opencode_question_answer(question: &PendingQuestionSpec, text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let tokens = if question.multiple {
        trimmed
            .split(|ch| [',', ';'].contains(&ch))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>()
    } else {
        vec![trimmed]
    };
    if tokens.is_empty() {
        return vec![trimmed.to_owned()];
    }

    let mut answers = Vec::new();
    for token in tokens {
        if let Some(label) = resolve_opencode_question_option(question, token) {
            answers.push(label);
        } else {
            answers.push(token.to_owned());
        }
    }
    answers
}

fn resolve_opencode_question_option(question: &PendingQuestionSpec, token: &str) -> Option<String> {
    if let Ok(index) = token.parse::<usize>() {
        if (1..=question.options.len()).contains(&index) {
            return Some(question.options[index - 1].label.clone());
        }
    }

    question
        .options
        .iter()
        .find(|option| option.label.eq_ignore_ascii_case(token))
        .map(|option| option.label.clone())
}

fn is_affirmative(text: &str) -> bool {
    let normalized = text
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "y" | "yes" | "approve" | "approved" | "accept" | "allow" | "ok" | "true"
    )
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
    use minos_agent_runtime::StartAgentOutcome;
    use minos_domain::{AgentDescriptor, AgentName, AgentStatus};
    use minos_protocol::local_rpc::ReadThreadRawHistoryResponse;
    use minos_protocol::{LocalGroupChatMessage, LocalGroupChatMessageKind, LocalIngestFrame};
    use minos_ui_protocol::{MessageRole, UiEventMessage};
    use ratatui::layout::Rect;
    use tokio::sync::broadcast;

    use super::*;

    struct TestBackend {
        detected_agents: Vec<AgentDescriptor>,
        started: Mutex<Vec<AgentName>>,
        sent_messages: Mutex<Vec<(String, String)>>,
        approval_decisions: Mutex<Vec<(String, String, serde_json::Value)>>,
        opencode_permission_responses: Mutex<Vec<(String, String, String)>>,
        opencode_question_responses: Mutex<Vec<(String, String, Vec<Vec<String>>)>>,
        group_chat_pages: Mutex<VecDeque<Vec<LocalGroupChatMessage>>>,
        next_thread: Mutex<usize>,
        interrupted: Mutex<Vec<String>>,
        closed: Mutex<Vec<String>>,
        deleted: Mutex<Vec<String>>,
        listed_threads: Mutex<Vec<BackendThreadSnapshot>>,
        history_pages: Mutex<HashMap<String, VecDeque<ReadThreadRawHistoryResponse>>>,
        history_calls: Mutex<Vec<(String, Option<u64>, u32)>>,
        connection_state: BackendConnectionState,
        block_starts: bool,
        block_sends: bool,
        ingest_tx: broadcast::Sender<LocalIngestFrame>,
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
                approval_decisions: Mutex::new(Vec::new()),
                opencode_permission_responses: Mutex::new(Vec::new()),
                opencode_question_responses: Mutex::new(Vec::new()),
                group_chat_pages: Mutex::new(VecDeque::new()),
                next_thread: Mutex::new(0),
                interrupted: Mutex::new(Vec::new()),
                closed: Mutex::new(Vec::new()),
                deleted: Mutex::new(Vec::new()),
                listed_threads: Mutex::new(Vec::new()),
                history_pages: Mutex::new(HashMap::new()),
                history_calls: Mutex::new(Vec::new()),
                connection_state: BackendConnectionState::Embedded,
                block_starts: false,
                block_sends: false,
                ingest_tx,
                manager_tx,
            }
        }

        fn with_connection_state(mut self, connection_state: BackendConnectionState) -> Self {
            self.connection_state = connection_state;
            self
        }

        fn with_blocked_starts(mut self) -> Self {
            self.block_starts = true;
            self
        }

        fn with_blocked_sends(mut self) -> Self {
            self.block_sends = true;
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

        fn with_group_chat_pages(self, pages: Vec<Vec<LocalGroupChatMessage>>) -> Self {
            *self.group_chat_pages.lock().expect("group chat pages lock") = VecDeque::from(pages);
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
            if self.block_starts {
                std::future::pending::<()>().await;
            }
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
            if self.block_sends {
                std::future::pending::<()>().await;
            }
            self.sent_messages
                .lock()
                .expect("sent messages lock")
                .push((thread_id.to_owned(), text.to_owned()));
            Ok(())
        }

        async fn send_approval_decision(
            &self,
            request_id: &str,
            thread_id: &str,
            decision: serde_json::Value,
        ) -> Result<()> {
            self.approval_decisions
                .lock()
                .expect("approval decisions lock")
                .push((request_id.to_owned(), thread_id.to_owned(), decision));
            Ok(())
        }

        async fn respond_opencode_permission(
            &self,
            thread_id: &str,
            permission_id: &str,
            response: &str,
        ) -> Result<()> {
            self.opencode_permission_responses
                .lock()
                .expect("opencode permission responses lock")
                .push((
                    thread_id.to_owned(),
                    permission_id.to_owned(),
                    response.to_owned(),
                ));
            Ok(())
        }

        async fn respond_opencode_question(
            &self,
            thread_id: &str,
            question_id: &str,
            answers: Vec<Vec<String>>,
        ) -> Result<()> {
            self.opencode_question_responses
                .lock()
                .expect("opencode question responses lock")
                .push((thread_id.to_owned(), question_id.to_owned(), answers));
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

        async fn read_group_chat(
            &self,
            _room_id: &str,
            _after_seq: Option<u64>,
            _before_seq: Option<u64>,
            _limit: u32,
        ) -> Result<Vec<LocalGroupChatMessage>> {
            Ok(self
                .group_chat_pages
                .lock()
                .expect("group chat pages lock")
                .pop_front()
                .unwrap_or_default())
        }

        async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame> {
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
        press_with_modifiers(code, KeyModifiers::NONE)
    }

    fn press_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
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

    fn projected_frame(
        thread_id: &str,
        seq: u64,
        agent: AgentName,
        ui_events: Vec<UiEventMessage>,
    ) -> LocalIngestFrame {
        LocalIngestFrame {
            thread_id: thread_id.to_string(),
            seq,
            agent,
            ui_events,
            ts_ms: i64::try_from(seq).unwrap_or(0),
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
        app.ui.focus = Focus::RoomInput;
        app.sync_input_agent_picker();

        assert!(app.handle_key(press(KeyCode::Char('@'))).await);
        assert!(app.handle_key(press(KeyCode::Char('c'))).await);
        assert!(app.handle_key(press(KeyCode::Down)).await);
        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(app.ui.room_input.content, "@claude ");
        assert_eq!(app.ui.room_input.cursor_pos, "@claude ".len());
        assert!(!app.ui.room_input.has_agent_picker());
    }

    #[tokio::test]
    async fn input_shortcuts_edit_without_inserting_control_text() {
        let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Codex)]));
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Codex)]);
        app.ui.focus = Focus::RoomInput;
        app.ui.room_input.content = "hello brave world".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();

        assert!(
            app.handle_key(press_with_modifiers(
                KeyCode::Char('w'),
                KeyModifiers::CONTROL
            ))
            .await
        );
        assert_eq!(app.ui.room_input.content, "hello brave ");
        assert_eq!(app.ui.room_input.cursor_pos, "hello brave ".len());

        assert!(
            app.handle_key(press_with_modifiers(KeyCode::Char('b'), KeyModifiers::ALT))
                .await
        );
        assert_eq!(app.ui.room_input.cursor_pos, "hello ".len());

        assert!(app.handle_key(press(KeyCode::Right)).await);
        assert_eq!(app.ui.room_input.cursor_pos, "hello b".len());

        assert!(
            app.handle_key(press_with_modifiers(
                KeyCode::Char('a'),
                KeyModifiers::CONTROL
            ))
            .await
        );
        assert_eq!(app.ui.room_input.cursor_pos, 0);
        assert_eq!(app.ui.room_input.content, "hello brave ");
    }

    #[tokio::test]
    async fn room_input_paste_inserts_multiline_text_without_submitting() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Codex)]));
        let mut app =
            App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Codex)]);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-codex-1234".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.chat_states.insert(
            "thread-codex-1234".into(),
            ChatState::new("thread-codex-1234".into(), AgentName::Codex),
        );
        app.select_thread(0);
        app.ui.focus = Focus::RoomInput;

        assert!(
            app.handle_event(AppEvent::Paste("first\r\nsecond\nthird".into()))
                .await
        );

        assert_eq!(app.ui.room_input.content, "first\nsecond\nthird");
        assert_eq!(app.ui.room_input.cursor_pos, "first\nsecond\nthird".len());
        assert!(backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .is_empty());

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[(
                "thread-codex-1234".to_owned(),
                "first\nsecond\nthird".to_owned()
            )]
        );
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
        app.ui.focus = Focus::RoomInput;
        app.ui.room_input.content = "@gemini write tests".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
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
        assert_eq!(app.ui.room_input.content, "");
        assert_eq!(app.ui.selected_thread, Some(0));
        assert_eq!(app.ui.threads[0].agent, AgentName::Gemini);
    }

    #[tokio::test]
    async fn room_input_on_closed_selected_thread_starts_new_same_agent() {
        let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
            AgentName::Opencode,
        )]));
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Opencode)]);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-opencode-closed".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Closed {
                reason: minos_agent_runtime::CloseReason::UserClose,
            },
        });
        app.select_thread(0);
        app.ui.focus = Focus::RoomInput;
        app.ui.room_input.content = "are you there?".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(
            backend
                .started
                .lock()
                .expect("started list lock")
                .as_slice(),
            &[AgentName::Opencode]
        );
        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[("thread-1".to_owned(), "are you there?".to_owned())]
        );
        assert_eq!(app.ui.room_input.content, "");
        assert_eq!(app.ui.selected_thread, Some(1));
        assert_eq!(app.ui.threads[1].agent, AgentName::Opencode);
    }

    #[tokio::test]
    async fn agent_input_on_closed_selected_thread_starts_new_same_agent() {
        let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
            AgentName::Opencode,
        )]));
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Opencode)]);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-opencode-closed".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Closed {
                reason: minos_agent_runtime::CloseReason::UserClose,
            },
        });
        app.select_thread(0);
        app.ui.focus = Focus::AgentInput;
        app.ui.agent_input.content = "continue".into();
        app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(
            backend
                .started
                .lock()
                .expect("started list lock")
                .as_slice(),
            &[AgentName::Opencode]
        );
        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[("thread-1".to_owned(), "continue".to_owned())]
        );
        assert_eq!(app.ui.agent_input.content, "");
        assert_eq!(app.ui.selected_thread, Some(1));
        assert_eq!(app.ui.threads[1].agent, AgentName::Opencode);
    }

    #[tokio::test]
    async fn routed_prompt_to_closed_thread_reports_error_without_sending() {
        let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
            AgentName::Opencode,
        )]));
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Opencode)]);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-opencode-closed".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Closed {
                reason: minos_agent_runtime::CloseReason::UserClose,
            },
        });
        app.ui.focus = Focus::RoomInput;
        app.ui.room_input.content = "@opencode#thread-o hello".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
        app.sync_input_agent_picker();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert!(backend
            .started
            .lock()
            .expect("started list lock")
            .is_empty());
        assert!(backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .is_empty());
        let error = app
            .ui
            .error_flash
            .as_ref()
            .map(|(message, _)| message.as_str())
            .unwrap_or("");
        assert!(error.contains("session #thread-o is closed"));
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
        app.ui.focus = Focus::RoomInput;
        app.ui.room_input.content = "@gemini write tests".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
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

        let persisted = app
            .group_chat_store
            .load_recent(10)
            .await
            .expect("group chat DB should be readable");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].kind, LocalGroupChatMessageKind::User);
        assert_eq!(persisted[0].text, "@gemini write tests");
    }

    #[tokio::test]
    async fn routed_prompt_echoes_in_group_chat_before_backend_send_finishes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(
            TestBackend::with_agents(vec![ok_agent(AgentName::Gemini)]).with_blocked_sends(),
        );
        let mut app =
            App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        app.set_event_sender(tx);
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Gemini)]);
        app.ui.focus = Focus::RoomInput;
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-gemini-1234".into(),
            agent: AgentName::Gemini,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.ui.chat_states.insert(
            "thread-gemini-1234".into(),
            ChatState::new("thread-gemini-1234".into(), AgentName::Gemini),
        );
        app.select_thread(0);
        app.ui.room_input.content = "@gemini#thread-g write tests".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
        app.sync_input_agent_picker();

        let handled = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            app.handle_key(press(KeyCode::Enter)),
        )
        .await
        .expect("room submit should not wait for backend send");

        assert!(handled);
        assert_eq!(app.ui.group_chat.messages.len(), 1);
        assert_eq!(
            app.ui.group_chat.messages[0].kind,
            LocalGroupChatMessageKind::User
        );
        assert_eq!(
            app.ui.group_chat.messages[0].text,
            "@gemini#thread-g write tests"
        );
        assert_eq!(
            app.ui.group_chat.messages[0].thread_id.as_deref(),
            Some("thread-gemini-1234")
        );
        assert_eq!(app.ui.room_input.content, "");
    }

    #[tokio::test]
    async fn routed_prompt_echoes_in_group_chat_before_agent_start_finishes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(
            TestBackend::with_agents(vec![ok_agent(AgentName::Gemini)]).with_blocked_starts(),
        );
        let mut app =
            App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        app.set_event_sender(tx);
        app.ui
            .status
            .update_agents(vec![ok_agent(AgentName::Gemini)]);
        app.ui.focus = Focus::RoomInput;
        app.ui.room_input.content = "@gemini write tests".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
        app.sync_input_agent_picker();

        let handled = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            app.handle_key(press(KeyCode::Enter)),
        )
        .await
        .expect("room submit should not wait for agent startup");

        assert!(handled);
        assert_eq!(app.ui.group_chat.messages.len(), 1);
        assert_eq!(
            app.ui.group_chat.messages[0].kind,
            LocalGroupChatMessageKind::User
        );
        assert_eq!(app.ui.group_chat.messages[0].text, "@gemini write tests");
        assert_eq!(app.ui.group_chat.messages[0].agent, Some(AgentName::Gemini));
        assert_eq!(app.ui.group_chat.messages[0].thread_id, None);
        assert_eq!(app.ui.room_input.content, "");
    }

    #[tokio::test]
    async fn agent_started_prompt_event_creates_chat_state_before_sending() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));

        assert!(
            app.handle_event(AppEvent::AgentStartedForPrompt {
                agent: AgentName::Gemini,
                thread_id: "thread-gemini-1234".into(),
                cwd: PathBuf::from("/tmp"),
                text: "write tests".into(),
            })
            .await
        );

        assert_eq!(app.ui.threads.len(), 1);
        assert!(app.ui.chat_states.contains_key("thread-gemini-1234"));
        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[("thread-gemini-1234".to_owned(), "write tests".to_owned())]
        );
    }

    #[tokio::test]
    async fn loading_group_history_restores_agent_list_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        group_store
            .append(LocalGroupChatMessage {
                seq: 0,
                message_id: String::new(),
                created_at_ms: 10,
                kind: LocalGroupChatMessageKind::User,
                text: "@codex inspect the repo".into(),
                agent: Some(AgentName::Codex),
                thread_id: Some("thread-codex-1234".into()),
                thread_short_id: Some("thread-c".into()),
                workspace: Some("/tmp/minos-a".into()),
            })
            .await
            .expect("append codex message");
        group_store
            .append(LocalGroupChatMessage {
                seq: 0,
                message_id: String::new(),
                created_at_ms: 20,
                kind: LocalGroupChatMessageKind::AgentResult,
                text: "done".into(),
                agent: Some(AgentName::Gemini),
                thread_id: Some("thread-gemini-5678".into()),
                thread_short_id: Some("thread-g".into()),
                workspace: Some("/tmp/minos-b".into()),
            })
            .await
            .expect("append gemini message");
        let backend = Arc::new(TestBackend::new());
        let mut app =
            App::with_group_chat_store(backend, false, PathBuf::from("/tmp/default"), group_store);

        app.load_group_chat_history().await;

        assert_eq!(app.ui.group_chat.messages.len(), 2);
        assert_eq!(app.ui.threads.len(), 2);
        assert_eq!(app.ui.selected_thread, Some(0));
        assert_eq!(app.ui.threads[0].thread_id, "thread-codex-1234");
        assert_eq!(app.ui.threads[0].agent, AgentName::Codex);
        assert_eq!(app.ui.threads[0].workspace, PathBuf::from("/tmp/minos-a"));
        assert!(matches!(
            app.ui.threads[0].state,
            ThreadState::Suspended {
                reason: minos_agent_runtime::PauseReason::DaemonRestart
            }
        ));
        assert_eq!(app.ui.threads[1].thread_id, "thread-gemini-5678");
        assert_eq!(app.ui.threads[1].agent, AgentName::Gemini);
        assert!(app.ui.chat_states.contains_key("thread-codex-1234"));
        assert!(app.ui.chat_states.contains_key("thread-gemini-5678"));
    }

    #[tokio::test]
    async fn daemon_group_history_loads_from_backend_and_restores_agent_entries() {
        let backend = Arc::new(
            TestBackend::new()
                .with_connection_state(BackendConnectionState::Connected {
                    endpoint: "ws://127.0.0.1:1".into(),
                })
                .with_group_chat_pages(vec![vec![LocalGroupChatMessage {
                    seq: 7,
                    message_id: "m-daemon-1".into(),
                    created_at_ms: 20,
                    kind: LocalGroupChatMessageKind::User,
                    text: "@opencode inspect this".into(),
                    agent: Some(AgentName::Opencode),
                    thread_id: Some("thread-opencode-1234".into()),
                    thread_short_id: Some("thread-o".into()),
                    workspace: Some("/tmp/daemon-ws".into()),
                }]]),
        );
        let mut app = App::new(backend, false, PathBuf::from("/tmp/default"));

        app.load_group_chat_history().await;

        assert_eq!(app.ui.group_chat.messages.len(), 1);
        assert_eq!(app.ui.group_chat.messages[0].seq, 7);
        assert_eq!(app.ui.threads.len(), 1);
        assert_eq!(app.ui.threads[0].thread_id, "thread-opencode-1234");
        assert_eq!(app.ui.threads[0].agent, AgentName::Opencode);
        assert_eq!(app.ui.threads[0].workspace, PathBuf::from("/tmp/daemon-ws"));
    }

    #[tokio::test]
    async fn agent_input_answers_pending_question_without_group_echo_or_prompt() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-codex-1234".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        let mut chat = ChatState::new("thread-codex-1234".into(), AgentName::Codex);
        chat.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "approval/request".into(),
            payload_json: serde_json::json!({
                "request_id": "req-1",
                "method": "item/tool/requestUserInput",
                "params": {
                    "questions": [{
                        "header": "Need input",
                        "id": "q1",
                        "question": "Pick one"
                    }]
                }
            })
            .to_string(),
        }]);
        app.ui.chat_states.insert("thread-codex-1234".into(), chat);
        app.select_thread(0);
        app.ui.focus = Focus::AgentInput;
        app.ui.agent_input.content = "blue".into();
        app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert!(app.ui.group_chat.messages.is_empty());
        assert!(backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .is_empty());
        let decisions = backend
            .approval_decisions
            .lock()
            .expect("approval decisions lock");
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].0, "req-1");
        assert_eq!(decisions[0].1, "thread-codex-1234");
        assert_eq!(
            decisions[0].2,
            serde_json::json!({ "answers": { "q1": { "answers": ["blue"] } } })
        );
        assert!(app
            .ui
            .chat_states
            .get("thread-codex-1234")
            .expect("chat state")
            .pending_requests
            .is_empty());
    }

    #[tokio::test]
    async fn agent_input_answers_opencode_permission() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-opencode-1234".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        let mut chat = ChatState::new("thread-opencode-1234".into(), AgentName::Opencode);
        chat.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "opencode/permission.updated".into(),
            payload_json: serde_json::json!({
                "permissionID": "perm-1",
                "title": "Run shell",
                "options": [
                    {"optionId": "proceed_once", "kind": "allow_once"},
                    {"optionId": "cancel", "kind": "reject_once"}
                ]
            })
            .to_string(),
        }]);
        app.ui
            .chat_states
            .insert("thread-opencode-1234".into(), chat);
        app.select_thread(0);
        app.ui.focus = Focus::AgentInput;
        app.ui.agent_input.content = "yes".into();
        app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert!(app.ui.group_chat.messages.is_empty());
        let responses = backend
            .opencode_permission_responses
            .lock()
            .expect("permission responses lock");
        assert_eq!(
            responses.as_slice(),
            &[(
                "thread-opencode-1234".to_owned(),
                "perm-1".to_owned(),
                "proceed_once".to_owned()
            )]
        );
    }

    #[tokio::test]
    async fn agent_input_answers_opencode_question_with_selected_option() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-opencode-1234".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        let mut chat = ChatState::new("thread-opencode-1234".into(), AgentName::Opencode);
        chat.apply_ui_events(vec![UiEventMessage::Raw {
            kind: "opencode/question.asked".into(),
            payload_json: serde_json::json!({
                "type": "question.asked",
                "properties": {
                    "id": "que-1",
                    "questions": [{
                        "header": "Core",
                        "question": "Pick a direction",
                        "options": [
                            {"label": "Fast", "description": "Ship quickly"},
                            {"label": "Robust", "description": "Prefer durability"}
                        ]
                    }]
                }
            })
            .to_string(),
        }]);
        app.ui
            .chat_states
            .insert("thread-opencode-1234".into(), chat);
        app.select_thread(0);
        app.ui.focus = Focus::AgentInput;
        app.ui.agent_input.content = "2".into();
        app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert!(app.ui.group_chat.messages.is_empty());
        let responses = backend
            .opencode_question_responses
            .lock()
            .expect("question responses lock");
        assert_eq!(
            responses.as_slice(),
            &[(
                "thread-opencode-1234".to_owned(),
                "que-1".to_owned(),
                vec![vec!["Robust".to_owned()]]
            )]
        );
        assert!(app
            .ui
            .chat_states
            .get("thread-opencode-1234")
            .expect("chat state")
            .pending_requests
            .is_empty());
    }

    #[tokio::test]
    async fn room_can_invite_second_agent_after_first_routed_prompt() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(TestBackend::with_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Gemini),
        ]));
        let mut app =
            App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
        app.ui.status.update_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Gemini),
        ]);
        app.ui.focus = Focus::RoomInput;

        app.ui.room_input.content = "@codex inspect the repo".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
        app.sync_input_agent_picker();
        assert!(app.handle_key(press(KeyCode::Enter)).await);

        app.ui.room_input.content = "@gemini".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
        app.sync_input_agent_picker();
        assert!(app.ui.room_input.has_agent_picker());
        assert!(app.handle_key(press(KeyCode::Enter)).await);
        assert_eq!(app.ui.room_input.content, "@gemini ");
        assert_eq!(
            backend
                .started
                .lock()
                .expect("started list lock")
                .as_slice(),
            &[AgentName::Codex]
        );
        assert_eq!(app.ui.group_chat.messages.len(), 1);

        assert!(app.handle_key(press(KeyCode::Enter)).await);

        assert_eq!(
            backend
                .started
                .lock()
                .expect("started list lock")
                .as_slice(),
            &[AgentName::Codex, AgentName::Gemini]
        );
        assert_eq!(
            backend
                .sent_messages
                .lock()
                .expect("sent messages lock")
                .as_slice(),
            &[("thread-1".to_owned(), "inspect the repo".to_owned())]
        );
        assert_eq!(app.ui.group_chat.messages.len(), 2);
        assert_eq!(
            app.ui.group_chat.messages[0].text,
            "@codex inspect the repo"
        );
        assert_eq!(app.ui.group_chat.messages[0].agent, Some(AgentName::Codex));
        assert_eq!(app.ui.group_chat.messages[1].text, "@gemini");
        assert_eq!(app.ui.group_chat.messages[1].agent, Some(AgentName::Gemini));
        assert_eq!(app.ui.room_input.content, "");
        assert_eq!(app.ui.selected_thread, Some(1));
    }

    #[tokio::test]
    async fn picker_can_route_prompt_to_existing_agent_session() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(TestBackend::with_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
        ]));
        let mut app =
            App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
        app.ui.status.update_agents(vec![
            ok_agent(AgentName::Codex),
            ok_agent(AgentName::Claude),
        ]);
        app.ui.focus = Focus::RoomInput;
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-codex".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp"),
            state: ThreadState::Idle,
        });
        app.select_thread(0);
        app.ui.chat_states.insert(
            "thread-codex-1234".into(),
            ChatState::new("thread-codex-1234".into(), AgentName::Codex),
        );
        app.ui.threads[0].thread_id = "thread-codex-1234".into();
        app.ui.room_input.content = "@codex".into();
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
        app.sync_input_agent_picker();

        let tokens: Vec<String> = app
            .ui
            .room_agent_mention_candidates()
            .into_iter()
            .map(|candidate| candidate.token)
            .collect();
        assert!(tokens.contains(&"codex".to_owned()));
        assert!(tokens.contains(&"codex#thread-c".to_owned()));
        assert!(app.handle_key(press(KeyCode::Down)).await);
        assert!(app.handle_key(press(KeyCode::Enter)).await);
        assert_eq!(app.ui.room_input.content, "@codex#thread-c ");

        app.ui.room_input.content.push_str("explain the diff");
        app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
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
            &[(
                "thread-codex-1234".to_owned(),
                "explain the diff".to_owned()
            )]
        );
        assert_eq!(app.ui.group_chat.messages.len(), 1);
        assert_eq!(
            app.ui.group_chat.messages[0].text,
            "@codex#thread-c explain the diff"
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
    async fn failed_agent_group_result_append_is_retried_on_tick() {
        let temp = tempfile::tempdir().expect("tempdir");
        let failing_store = GroupChatStore::failing();
        let backend = Arc::new(TestBackend::new());
        let mut app =
            App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), failing_store);
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

        assert!(
            app.handle_event(AppEvent::ManagerEvent(ManagerEvent::ThreadStateChanged {
                thread_id: "thread-gemini-1234".into(),
                old: ThreadState::Running {
                    turn_started_at_ms: 0,
                },
                new: ThreadState::Idle,
                at_ms: 3,
            }))
            .await
        );

        assert!(app.ui.group_chat.messages.is_empty());
        assert!(!app
            .recorded_agent_results
            .contains_key("thread-gemini-1234"));

        app.group_chat_store = GroupChatStore::at_path(temp.path().join("group.sqlite"));
        assert!(app.handle_event(AppEvent::Tick).await);

        assert_eq!(app.ui.group_chat.messages.len(), 1);
        let message = &app.ui.group_chat.messages[0];
        assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
        assert_eq!(message.text, "The module handles auth.");
        assert_eq!(
            app.recorded_agent_results
                .get("thread-gemini-1234")
                .map(String::as_str),
            Some("assistant-1")
        );
    }

    #[tokio::test]
    async fn opencode_session_idle_ingest_records_group_result_without_manager_idle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(TestBackend::new());
        let mut app =
            App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-opencode-1234".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp/ws"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        app.ui.chat_states.insert(
            "thread-opencode-1234".into(),
            ChatState::new("thread-opencode-1234".into(), AgentName::Opencode),
        );

        assert!(
            app.handle_event(AppEvent::Ingest(projected_frame(
                "thread-opencode-1234",
                1,
                AgentName::Opencode,
                vec![
                    UiEventMessage::MessageStarted {
                        message_id: "msg-assistant-1".into(),
                        role: MessageRole::Assistant,
                        started_at_ms: 1,
                    },
                    UiEventMessage::TextDelta {
                        message_id: "msg-assistant-1".into(),
                        text: "在的！有什么可以帮你的？".into(),
                    },
                ],
            )))
            .await
        );
        assert!(app.ui.group_chat.messages.is_empty());

        assert!(
            app.handle_event(AppEvent::Ingest(projected_frame(
                "thread-opencode-1234",
                2,
                AgentName::Opencode,
                vec![UiEventMessage::MessageCompleted {
                    message_id: "msg-assistant-1".into(),
                    finished_at_ms: 2,
                }],
            )))
            .await
        );

        assert_eq!(app.ui.group_chat.messages.len(), 1);
        let message = &app.ui.group_chat.messages[0];
        assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
        assert_eq!(message.text, "在的！有什么可以帮你的？");
        assert_eq!(message.agent, Some(AgentName::Opencode));
        assert_eq!(message.thread_id.as_deref(), Some("thread-opencode-1234"));
    }

    #[tokio::test]
    async fn opencode_manager_idle_does_not_record_partial_result_before_final_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(TestBackend::new());
        let mut app =
            App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-opencode-1234".into(),
            agent: AgentName::Opencode,
            workspace: PathBuf::from("/tmp/ws"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        app.ui.chat_states.insert(
            "thread-opencode-1234".into(),
            ChatState::new("thread-opencode-1234".into(), AgentName::Opencode),
        );

        assert!(
            app.handle_event(AppEvent::Ingest(projected_frame(
                "thread-opencode-1234",
                1,
                AgentName::Opencode,
                vec![
                    UiEventMessage::MessageStarted {
                        message_id: "msg-assistant-1".into(),
                        role: MessageRole::Assistant,
                        started_at_ms: 1,
                    },
                    UiEventMessage::TextDelta {
                        message_id: "msg-assistant-1".into(),
                        text: "Gemini 说了以下内容：它详细介绍了".into(),
                    },
                ],
            )))
            .await
        );

        assert!(
            app.handle_event(AppEvent::ManagerEvent(ManagerEvent::ThreadStateChanged {
                thread_id: "thread-opencode-1234".into(),
                old: ThreadState::Running {
                    turn_started_at_ms: 0,
                },
                new: ThreadState::Idle,
                at_ms: 2,
            }))
            .await
        );
        assert!(app.ui.group_chat.messages.is_empty());

        assert!(
            app.handle_event(AppEvent::Ingest(projected_frame(
                "thread-opencode-1234",
                3,
                AgentName::Opencode,
                vec![UiEventMessage::TextReplace {
                    message_id: "msg-assistant-1".into(),
                    text: "Gemini 说了以下内容：它详细介绍了自己的能力。".into(),
                }],
            )))
            .await
        );
        assert!(app.ui.group_chat.messages.is_empty());

        assert!(
            app.handle_event(AppEvent::Ingest(projected_frame(
                "thread-opencode-1234",
                4,
                AgentName::Opencode,
                vec![UiEventMessage::MessageCompleted {
                    message_id: "msg-assistant-1".into(),
                    finished_at_ms: 4,
                }],
            )))
            .await
        );

        assert_eq!(app.ui.group_chat.messages.len(), 1);
        assert_eq!(
            app.ui.group_chat.messages[0].text,
            "Gemini 说了以下内容：它详细介绍了自己的能力。"
        );
        assert_eq!(
            app.ui.group_chat.messages[0].text,
            "Gemini 说了以下内容：它详细介绍了自己的能力。"
        );
    }

    #[tokio::test]
    async fn daemon_tick_replays_history_and_records_opencode_result_when_live_ingest_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
        let backend = Arc::new(
            TestBackend::new()
                .with_connection_state(BackendConnectionState::Connected {
                    endpoint: "ws://127.0.0.1:43123".into(),
                })
                .with_listed_threads(vec![BackendThreadSnapshot {
                    thread_id: "thread-opencode-1234".into(),
                    agent: Some(AgentName::Opencode),
                    workspace: PathBuf::from("/tmp/ws"),
                    state: ThreadState::Idle,
                }])
                .with_history_pages(
                    "thread-opencode-1234",
                    vec![ReadThreadRawHistoryResponse {
                        events: vec![
                            projected_frame(
                                "thread-opencode-1234",
                                1,
                                AgentName::Opencode,
                                vec![UiEventMessage::MessageStarted {
                                    message_id: "msg-assistant-1".into(),
                                    role: MessageRole::Assistant,
                                    started_at_ms: 1,
                                }],
                            ),
                            projected_frame(
                                "thread-opencode-1234",
                                2,
                                AgentName::Opencode,
                                vec![UiEventMessage::TextDelta {
                                    message_id: "msg-assistant-1".into(),
                                    text: "在的，有什么可以帮你的？".into(),
                                }],
                            ),
                            projected_frame(
                                "thread-opencode-1234",
                                3,
                                AgentName::Opencode,
                                vec![UiEventMessage::MessageCompleted {
                                    message_id: "msg-assistant-1".into(),
                                    finished_at_ms: 3,
                                }],
                            ),
                        ],
                        next_seq: None,
                    }],
                ),
        );
        let mut app =
            App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);

        assert!(app.handle_event(AppEvent::Tick).await);

        assert_eq!(app.ui.group_chat.messages.len(), 1);
        let message = &app.ui.group_chat.messages[0];
        assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
        assert_eq!(message.text, "在的，有什么可以帮你的？");
        assert_eq!(message.agent, Some(AgentName::Opencode));
        assert_eq!(message.thread_id.as_deref(), Some("thread-opencode-1234"));
    }

    #[tokio::test]
    async fn idle_thread_state_finishes_streaming_assistant_cursor() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.threads.push(ThreadEntry {
            thread_id: "thread-codex-1234".into(),
            agent: AgentName::Codex,
            workspace: PathBuf::from("/tmp/ws"),
            state: ThreadState::Running {
                turn_started_at_ms: 0,
            },
        });
        let mut chat = ChatState::new("thread-codex-1234".into(), AgentName::Codex);
        chat.apply_ui_events(vec![
            UiEventMessage::MessageStarted {
                message_id: "assistant-1".into(),
                role: MessageRole::Assistant,
                started_at_ms: 1,
            },
            UiEventMessage::TextDelta {
                message_id: "assistant-1".into(),
                text: "partial".into(),
            },
        ]);
        app.ui.chat_states.insert("thread-codex-1234".into(), chat);

        assert!(
            app.handle_event(AppEvent::ManagerEvent(ManagerEvent::ThreadStateChanged {
                thread_id: "thread-codex-1234".into(),
                old: ThreadState::Running {
                    turn_started_at_ms: 0,
                },
                new: ThreadState::Idle,
                at_ms: 2,
            }))
            .await
        );

        match &app.ui.chat_states["thread-codex-1234"].items[0] {
            ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn esc_moves_focus_without_quitting() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.focus = Focus::RoomChat;

        let redraw = app.handle_key(press(KeyCode::Esc)).await;

        assert!(redraw);
        assert_eq!(app.ui.focus, Focus::RoomList);
        assert!(!app.should_quit());
    }

    #[tokio::test]
    async fn enter_on_agent_list_opens_detail_and_esc_from_agent_chat_closes_it() {
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
        app.ui.focus = Focus::AgentList;

        assert!(app.handle_key(press(KeyCode::Enter)).await);
        assert!(app.ui.agent_detail_visible);
        assert_eq!(app.ui.focus, Focus::AgentChat);

        assert!(app.handle_key(press(KeyCode::Esc)).await);
        assert!(!app.ui.agent_detail_visible);
        assert_eq!(app.ui.focus, Focus::AgentList);
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
        app.ui.focus = Focus::RoomInput;
        app.ui.agent_detail_visible = true;
        app.ui.panel_areas.agent_chat = Rect::new(20, 0, 60, 20);
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
        assert_eq!(app.ui.focus, Focus::AgentChat);
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
        app.ui.panel_areas.agent_list = Rect::new(0, 0, 20, 10);

        let redraw = app
            .handle_mouse(MouseEvent {
                row: 2,
                column: 1,
                ..scroll(MouseEventKind::ScrollDown)
            })
            .await;

        assert!(redraw);
        assert_eq!(app.ui.focus, Focus::AgentList);
        assert_eq!(app.ui.selected_thread, Some(1));
    }

    #[tokio::test]
    async fn clicking_thread_list_blank_area_focuses_thread_list() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend, false, PathBuf::from("/tmp"));
        app.ui.panel_areas.room_list = Rect::new(0, 0, 20, 10);
        app.ui.focus = Focus::RoomChat;

        let redraw = app
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 8,
                modifiers: KeyModifiers::NONE,
            })
            .await;

        assert!(redraw);
        assert_eq!(app.ui.focus, Focus::RoomList);
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
        app.ui.agent_detail_visible = true;
        app.ui.panel_areas.agent_chat = Rect::new(0, 0, 40, 10);
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
        app.ui.focus = Focus::AgentList;

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
        app.ui.focus = Focus::AgentList;

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
