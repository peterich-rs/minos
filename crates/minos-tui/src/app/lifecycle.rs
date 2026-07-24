use super::*;

impl App {
    pub async fn init(&mut self) -> anyhow::Result<()> {
        let agents = self.backend.detect_clis().await?;
        self.ui.status.update_agents(agents);
        self.refresh_agent_profiles().await;
        self.sync_input_agent_picker();
        if matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            self.hydrate_daemon_threads().await;
        }
        self.resolve_startup_project().await;
        Ok(())
    }

    /// Refresh host agent profiles for @-routing / mention candidates.
    pub(super) async fn refresh_agent_profiles(&mut self) {
        match self.backend.list_agent_profiles().await {
            Ok(profiles) => {
                self.ui.agent_profiles = profiles;
            }
            Err(e) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %e,
                    "list_agent_profiles failed; profile mentions disabled until refresh"
                );
            }
        }
    }

    async fn resolve_startup_project(&mut self) {
        let projects = match self.backend.list_projects().await {
            Ok(p) => p,
            Err(e) => {
                self.ui.set_error(format!("Failed to load projects: {e}"));
                return;
            }
        };
        let cwd = self.state.workspace.clone();
        let matched_index = projects.iter().position(|p| {
            crate::state::workspace_path_belongs_to_current_workspace(&cwd, &p.workspace_path)
        });
        self.ui.projects.items = projects;
        self.ui.projects.selected = matched_index.or({
            if self.ui.projects.items.is_empty() {
                None
            } else {
                Some(0)
            }
        });
        self.ui
            .projects
            .list_state
            .select(self.ui.projects.selected);

        match matched_index {
            Some(index) => {
                let project_id = self.ui.projects.items[index].project_id.clone();
                let conversations = match self.backend.list_conversations(&project_id).await {
                    Ok(conversations) => conversations,
                    Err(error) => {
                        self.ui
                            .set_error(format!("Failed to load conversations: {error}"));
                        Vec::new()
                    }
                };
                self.ui.conversations.items = conversations;
                self.ui.conversations.selected = if self.ui.conversations.items.is_empty() {
                    None
                } else {
                    Some(0)
                };
                self.ui
                    .conversations
                    .list_state
                    .select(self.ui.conversations.selected);
                self.ui.nav.stack = vec![
                    crate::nav::NavLevel::Projects,
                    crate::nav::NavLevel::Conversations { project_id },
                ];
            }
            None => {
                let dir_name = cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "workspace".to_owned());
                self.ui.overlays.project_create = Some(crate::ui::ProjectCreateDialogState {
                    name: dir_name,
                    path: cwd.to_string_lossy().into_owned(),
                    editing_name: true,
                });
            }
        }
    }

    pub async fn handle_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Ingest(ingest) => {
                let redraw = self.handle_ingest(ingest).await;
                if redraw {
                    self.request_frame_streaming();
                }
                redraw
            }
            AppEvent::ManagerEvent(event) => {
                let redraw = self.handle_manager_event(event).await;
                if redraw {
                    self.request_frame();
                }
                redraw
            }
            AppEvent::AgentStartedForPrompt {
                agent,
                session_id,
                cwd,
                text,
            } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::AgentStarted {
                        agent,
                        session_id,
                        cwd,
                        text,
                    },
                ))
                .await
            }
            AppEvent::SendMessageFailed { session_id, error } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::SendFailed { session_id, error },
                ))
                .await
            }
            AppEvent::ProjectCreated(project) => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ProjectCreated(project),
                ))
                .await
            }
            AppEvent::PathCandidatesResolved {
                target,
                sequence,
                candidates,
            } => {
                let input = match target {
                    InputTarget::Conversation => &mut self.ui.inputs.conversation,
                    InputTarget::Agent => &mut self.ui.inputs.agent,
                };
                if input.apply_path_candidates(sequence, candidates) {
                    self.request_frame();
                    true
                } else {
                    false
                }
            }
            AppEvent::ConversationsLoaded {
                project_id,
                conversations,
            } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ConversationsLoaded {
                        project_id,
                        conversations,
                    },
                ))
                .await
            }
            AppEvent::ConversationOpened {
                project_id,
                conversation_id,
                messages,
                sessions,
            } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ConversationOpened {
                        project_id,
                        conversation_id,
                        messages,
                        sessions,
                    },
                ))
                .await
            }
            AppEvent::ConversationAgentStarted {
                conversation_id,
                agent,
                session_id,
                cwd,
                text,
            } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ConversationAgentStarted {
                        conversation_id,
                        agent,
                        session_id,
                        cwd,
                        text,
                    },
                ))
                .await
            }
            AppEvent::ConversationMessageAppended {
                conversation_id,
                message_seq,
            } => {
                self.refresh_current_conversation_messages(&conversation_id)
                    .await;
                tracing::debug!(
                    target: "minos_tui::app",
                    conversation_id,
                    message_seq,
                    "conversation messages refreshed after daemon append event"
                );
                self.request_frame();
                true
            }
            AppEvent::DaemonThreadsListed { sessions } => {
                // Metadata-only: never await history on the poll path (scroll).
                let redraw = self.apply_daemon_thread_metadata(sessions);
                if redraw {
                    self.request_frame();
                }
                redraw
            }
            AppEvent::ProjectFailed(error) => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ProjectFailed(error),
                ))
                .await
            }
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Paste(text) => self.handle_paste(text).await,
            AppEvent::Mouse(mouse) => self.handle_mouse(mouse).await,
            AppEvent::Tick => self.apply_action(Action::Global(GlobalAction::Tick)).await,
            AppEvent::Resize(_, _) => {
                // Debounce rapid resize reflow (Codex TRANSCRIPT_REFLOW_DEBOUNCE-style).
                self.request_frame_in(std::time::Duration::from_millis(50));
                true
            }
        }
    }

    pub(super) async fn handle_ingest(&mut self, ingest: minos_protocol::LocalIngestFrame) -> bool {
        if !self
            .ui
            .session_panel
            .chat_states
            .contains_key(&ingest.session_id)
        {
            debug!(
                agent = %ingest.agent.bin_name(),
                session_id = %ingest.session_id,
                "creating chat state for ingest frame",
            );
            self.ui.session_panel.chat_states.insert(
                ingest.session_id.clone(),
                ChatState::new(ingest.session_id.clone(), ingest.agent),
            );
        }
        if !self.mark_ingest_applied(&ingest) {
            return false;
        }
        let marks_done = frame_marks_agent_result_done(&ingest);
        let sessions_changed = sync_subagent_sessions(
            &mut self.state,
            &mut self.ui,
            &ingest.ui_events,
            ingest.ts_ms,
        );
        sync_subagent_info(&mut self.ui, &ingest.ui_events);
        let session_id = ingest.session_id;
        let agent = ingest.agent;
        let ui_events = ingest.ui_events;
        if let Some(chat) = self.ui.session_panel.chat_states.get_mut(&session_id) {
            debug!(
                agent = %agent.bin_name(),
                session_id = %session_id,
                event_count = ui_events.len(),
                "applying projected ingest frame"
            );
            chat.apply_ui_events(ui_events);
            self.record_agent_conversation_result_if_ingest_done(&session_id, marks_done)
                .await;
            return true;
        }
        sessions_changed
    }

    pub(super) async fn handle_tick(&mut self) -> bool {
        let mut redraw = false;
        // Never await daemon RPCs on the tick path when the event pump is live —
        // blocking here freezes scroll/input for the whole round-trip.
        if self.event_tx.is_some() {
            self.schedule_daemon_thread_list_if_due();
        } else if self.sync_daemon_threads_if_due().await {
            // Tests / headless without a frame event loop still sync inline.
            redraw = true;
        }
        if let Some((_, instant)) = self.ui.error_flash {
            if instant.elapsed() > crate::ui::UiState::ERROR_FLASH_TTL {
                self.ui.error_flash = None;
                redraw = true;
            }
        }
        if let Some(instant) = self.ui.flash_copied {
            if instant.elapsed() > crate::ui::UiState::COPIED_FLASH_TTL {
                self.ui.flash_copied = None;
                redraw = true;
            }
        }
        self.ui
            .status
            .update_backend_state(self.backend.connection_state());
        if self.retry_pending_agent_conversation_results_if_due() {
            redraw = true;
        }
        redraw
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub async fn shutdown(&self) {
        // Daemon owns agent process lifecycle; TUI only disconnects.
    }

    pub fn ui(&mut self) -> &mut UiState {
        &mut self.ui
    }

    pub fn set_event_sender(&mut self, tx: tokio::sync::mpsc::UnboundedSender<AppEvent>) {
        self.event_tx = Some(tx);
    }

    pub fn set_frame_requester(&mut self, requester: crate::frame::FrameRequester) {
        self.frame_requester = Some(requester);
    }

    pub(super) fn request_frame(&self) {
        if let Some(requester) = &self.frame_requester {
            requester.schedule_frame();
        }
    }

    /// Public wrapper for the main loop (viewport follow-up frames).
    pub fn request_frame_public(&self) {
        self.request_frame();
    }

    /// Higher-churn path (agent ingest streaming). Same coalescer; explicit intent.
    pub(super) fn request_frame_streaming(&self) {
        if let Some(requester) = &self.frame_requester {
            requester.schedule_frame_streaming();
        }
    }

    pub(super) fn request_frame_in(&self, delay: std::time::Duration) {
        if let Some(requester) = &self.frame_requester {
            requester.schedule_frame_in(delay);
        }
    }

    pub(super) async fn hydrate_daemon_threads(&mut self) {
        let _ = self.sync_daemon_threads_from_backend(false).await;
    }

    pub(super) async fn sync_daemon_threads_if_due(&mut self) -> bool {
        if !self.daemon_thread_sync_due() {
            return false;
        }
        self.state.last_daemon_history_sync = Some(Instant::now());
        self.sync_daemon_threads_from_backend(true).await
    }

    /// Fire-and-forget list_sessions so the main loop can keep scrolling.
    fn schedule_daemon_thread_list_if_due(&mut self) {
        if !self.daemon_thread_sync_due() {
            return;
        }
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        self.state.last_daemon_history_sync = Some(Instant::now());
        let backend = self.backend.clone();
        tokio::spawn(async move {
            match backend.list_sessions().await {
                Ok(sessions) => {
                    let _ = tx.send(AppEvent::DaemonThreadsListed { sessions });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "minos_tui::app",
                        error = %e,
                        "background list_sessions failed"
                    );
                }
            }
        });
    }

    fn daemon_thread_sync_due(&self) -> bool {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            return false;
        }
        let now = Instant::now();
        self.state
            .last_daemon_history_sync
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(2))
    }

    pub(super) async fn sync_daemon_threads_from_backend(&mut self, incremental: bool) -> bool {
        match self.backend.list_sessions().await {
            Ok(sessions) => {
                self.apply_daemon_thread_snapshots(sessions, incremental)
                    .await
            }
            Err(e) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %e,
                    "hydrate_daemon_threads failed"
                );
                false
            }
        }
    }

    /// Apply daemon `list_sessions` snapshots and hydrate unknown sessions.
    ///
    /// Used by init and the headless (no event pump) tick path. The live main
    /// loop must **not** call this for `DaemonThreadsListed` — that path uses
    /// [`Self::apply_daemon_thread_metadata`] only so scroll never awaits
    /// history RPC. Already-hydrated sessions still rely on live ingest.
    pub(super) async fn apply_daemon_thread_snapshots(
        &mut self,
        sessions: Vec<crate::backend::BackendSessionSnapshot>,
        _incremental: bool,
    ) -> bool {
        let mut changed = self.apply_daemon_thread_metadata(sessions);
        let pending: Vec<String> = self
            .ui
            .session_panel
            .list
            .items
            .iter()
            .map(|thread| thread.session_id.clone())
            .filter(|session_id| !self.state.hydrated_threads.contains(session_id))
            .collect();
        for session_id in pending {
            if self.hydrate_thread_if_needed(&session_id).await {
                changed = true;
            }
        }
        changed
    }

    /// Sync-only metadata merge for daemon thread snapshots.
    ///
    /// No history RPC, no conversation-result writeback. Safe to run on the
    /// main loop when `DaemonThreadsListed` arrives.
    pub(super) fn apply_daemon_thread_metadata(
        &mut self,
        sessions: Vec<crate::backend::BackendSessionSnapshot>,
    ) -> bool {
        let mut changed = false;
        if self.prune_external_threads() {
            changed = true;
        }

        // Batch workspace membership so each path is canonicalized once, not
        // once per (thread × known-workspace) comparison.
        let mut matcher = state::WorkspaceMatcher::from_state(&self.state, &self.ui);
        let mut belonging = Vec::with_capacity(sessions.len());
        for snap in sessions {
            if matcher.contains(&snap.workspace) {
                belonging.push(snap);
            } else if self.remove_thread_local_state(&snap.session_id) {
                changed = true;
            }
        }

        // O(1) updates by session_id — avoid O(threads²) linear finds on the
        // 2s poll path.
        let mut index: std::collections::HashMap<String, usize> = self
            .ui
            .session_panel
            .list
            .items
            .iter()
            .enumerate()
            .map(|(i, thread)| (thread.session_id.clone(), i))
            .collect();

        for snap in belonging {
            let agent = snap.agent.unwrap_or(AgentName::Codex);
            if let Some(&i) = index.get(&snap.session_id) {
                let entry = &mut self.ui.session_panel.list.items[i];
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
                if entry.parent_session_id != snap.parent_session_id {
                    entry.parent_session_id = snap.parent_session_id.clone();
                    changed = true;
                }
            } else {
                let session_id = snap.session_id.clone();
                self.ui.session_panel.list.items.push(SessionEntry {
                    session_id: session_id.clone(),
                    agent,
                    workspace: snap.workspace.clone(),
                    state: snap.state.clone(),
                    parent_session_id: snap.parent_session_id.clone(),
                });
                index.insert(session_id, self.ui.session_panel.list.items.len() - 1);
                changed = true;
            }

            // Only touch chat_states when missing or agent identity changed.
            let needs_chat = match self.ui.session_panel.chat_states.get(&snap.session_id) {
                None => true,
                Some(chat) => chat.agent != agent,
            };
            if needs_chat {
                self.ensure_chat_state_agent(&snap.session_id, agent);
            }
        }

        if !self.ui.session_panel.list.items.is_empty()
            && self.ui.session_panel.list.selected.is_none()
        {
            self.select_thread(0);
            changed = true;
        }
        changed
    }

    pub(super) async fn replay_thread_history_from(
        &mut self,
        session_id: &str,
        mut from_seq: Option<u64>,
        mark_hydrated: bool,
    ) -> bool {
        let mut changed = false;
        loop {
            let response = match self
                .backend
                .read_session_raw_history(session_id, from_seq, 1000)
                .await
            {
                Ok(response) => response,
                Err(e) => {
                    tracing::warn!(
                        target: "minos_tui::app",
                        error = %e,
                        session_id = %session_id,
                        "replay_thread_history failed"
                    );
                    return changed;
                }
            };

            for frame in response.events {
                if !self.mark_ingest_applied(&frame) {
                    continue;
                }
                if sync_subagent_sessions(
                    &mut self.state,
                    &mut self.ui,
                    &frame.ui_events,
                    frame.ts_ms,
                ) {
                    changed = true;
                }
                sync_subagent_info(&mut self.ui, &frame.ui_events);
                if let Some(chat) = self.ui.session_panel.chat_states.get_mut(session_id) {
                    if !frame.ui_events.is_empty() {
                        changed = true;
                    }
                    chat.apply_ui_events(frame.ui_events);
                }
            }

            let Some(next_seq) = response.next_seq else {
                if mark_hydrated {
                    self.state.hydrated_threads.insert(session_id.to_owned());
                }
                return changed;
            };
            from_seq = Some(next_seq.saturating_sub(1));
        }
    }

    pub(super) fn retry_pending_agent_conversation_results_if_due(&mut self) -> bool {
        // Daemon owns agent-result writeback; TUI path is a permanent no-op.
        // Do not walk the session list on every tick.
        let _ = self;
        false
    }

    pub(super) async fn hydrate_thread_if_needed(&mut self, session_id: &str) -> bool {
        if self.state.hydrated_threads.contains(session_id) {
            return false;
        }
        self.replay_thread_history_from(session_id, None, true)
            .await
    }

    pub(super) async fn ensure_conversation_agent_session_visible(&mut self, session_id: &str) {
        let was_visible = self
            .ui
            .session_panel
            .list
            .items
            .iter()
            .any(|t| t.session_id == session_id);
        if !was_visible {
            if let Some((agent, state)) = self
                .ui
                .conversation
                .agent_sessions
                .items
                .iter()
                .find(|s| s.session_id == session_id)
                .map(|session| (session.agent, session.state.clone()))
            {
                let workspace = self
                    .ui
                    .nav_level()
                    .project_id()
                    .and_then(|project_id| {
                        self.ui
                            .projects
                            .items
                            .iter()
                            .find(|project| project.project_id == project_id)
                    })
                    .map(|project| project.workspace_path.clone())
                    .unwrap_or_else(|| self.state.workspace.clone());
                self.ensure_thread_visible(session_id.to_owned(), agent, workspace);
                if let Some(entry) = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter_mut()
                    .find(|thread| thread.session_id == session_id)
                {
                    entry.state = state;
                }
            }
        }
        if !self.state.hydrated_threads.contains(session_id) {
            self.hydrate_thread_if_needed(session_id).await;
        }
    }

    pub(super) fn ensure_chat_state_agent(&mut self, session_id: &str, agent: AgentName) {
        match self
            .ui
            .session_panel
            .chat_states
            .entry(session_id.to_owned())
        {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().agent != agent {
                    entry.insert(ChatState::new(session_id.to_owned(), agent));
                    self.state.hydrated_threads.remove(session_id);
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(ChatState::new(session_id.to_owned(), agent));
            }
        }
    }

    pub(super) fn workspace_path_belongs_to_current_workspace(&self, workspace: &Path) -> bool {
        state::workspace_path_belongs_to_known_workspace(&self.state, &self.ui, workspace)
    }

    pub(super) fn prune_external_threads(&mut self) -> bool {
        if !state::prune_external_threads(&mut self.state, &mut self.ui) {
            return false;
        }
        self.sync_input_agent_picker();
        true
    }

    pub(super) fn remove_thread_local_state(&mut self, session_id: &str) -> bool {
        if !state::remove_thread_local_state(&mut self.state, &mut self.ui, session_id) {
            return false;
        }
        self.sync_input_agent_picker();
        true
    }

    pub(super) fn mark_ingest_applied(&mut self, frame: &minos_protocol::LocalIngestFrame) -> bool {
        state::mark_ingest_applied(&mut self.state, frame)
    }

    pub(super) async fn handle_manager_event(&mut self, event: ManagerEvent) -> bool {
        match event {
            ManagerEvent::SessionAdded {
                session_id,
                workspace,
                agent,
                parent_session_id,
            } => {
                let subagent_parent = parent_session_id.clone();
                if !self.workspace_path_belongs_to_current_workspace(&workspace) {
                    return self.remove_thread_local_state(&session_id);
                }
                if let Some(parent_session_id) = subagent_parent.as_deref() {
                    if let Some(conversation_id) =
                        conversation_id_for_parent(&self.state, &self.ui, parent_session_id)
                    {
                        self.state
                            .session_conversations
                            .insert(session_id.clone(), conversation_id);
                    }
                }
                if let Some(index) = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter()
                    .position(|t| t.session_id == session_id)
                {
                    if let Some(entry) = self.ui.session_panel.list.items.get_mut(index) {
                        entry.agent = agent;
                        entry.workspace = workspace;
                        entry.parent_session_id = parent_session_id;
                    }
                    self.ensure_chat_state_agent(&session_id, agent);
                    if let Some(parent_session_id) = subagent_parent.as_deref() {
                        upsert_subagent_session(
                            &mut self.ui,
                            parent_session_id,
                            &session_id,
                            agent,
                            None,
                            0,
                        );
                    }
                    return true;
                }

                let entry = SessionEntry {
                    session_id: session_id.clone(),
                    agent,
                    workspace,
                    state: SessionState::Starting,
                    parent_session_id: parent_session_id.clone(),
                };
                self.ui.session_panel.list.items.push(entry);
                self.ui.session_panel.chat_states.insert(
                    session_id.clone(),
                    ChatState::new(session_id.clone(), agent),
                );
                if let Some(parent_session_id) = parent_session_id.as_deref() {
                    upsert_subagent_session(
                        &mut self.ui,
                        parent_session_id,
                        &session_id,
                        agent,
                        None,
                        0,
                    );
                }
                self.select_thread(self.ui.session_panel.list.items.len().saturating_sub(1));
                self.ui.focus.focus(PaneId::Input);
                self.sync_input_agent_picker();
                true
            }
            ManagerEvent::SessionStateChanged {
                session_id, new, ..
            } => {
                let known_thread = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter()
                    .any(|t| t.session_id == session_id);
                let known_session = self
                    .ui
                    .conversation
                    .agent_sessions
                    .items
                    .iter()
                    .any(|s| s.session_id == session_id);
                if !known_thread && !known_session {
                    return false;
                }
                if let Some(session) = self
                    .ui
                    .conversation
                    .agent_sessions
                    .items
                    .iter_mut()
                    .find(|s| s.session_id == session_id)
                {
                    session.state = new.clone();
                }
                let is_terminal_for_stream = !matches!(
                    new,
                    SessionState::Starting | SessionState::Running { .. } | SessionState::Resuming
                );
                let thread_agent = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter()
                    .find(|t| t.session_id == session_id)
                    .map(|thread| thread.agent);
                if let Some(entry) = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter_mut()
                    .find(|t| t.session_id == session_id)
                {
                    entry.state = new;
                }
                if is_terminal_for_stream {
                    if let Some(chat) = self.ui.session_panel.chat_states.get_mut(&session_id) {
                        chat.finish_all_streaming();
                    }
                }
                if thread_agent != Some(AgentName::Opencode) {
                    self.record_agent_conversation_result_if_done(&session_id)
                        .await;
                }
                true
            }
            ManagerEvent::SessionClosed { session_id, reason } => {
                let known_thread = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter()
                    .any(|t| t.session_id == session_id);
                let known_session = self
                    .ui
                    .conversation
                    .agent_sessions
                    .items
                    .iter()
                    .any(|s| s.session_id == session_id);
                if !known_thread && !known_session {
                    return false;
                }
                let closed_state = SessionState::Closed { reason };
                if let Some(session) = self
                    .ui
                    .conversation
                    .agent_sessions
                    .items
                    .iter_mut()
                    .find(|s| s.session_id == session_id)
                {
                    session.state = closed_state.clone();
                }
                let thread_agent = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter()
                    .find(|t| t.session_id == session_id)
                    .map(|thread| thread.agent);
                if let Some(entry) = self
                    .ui
                    .session_panel
                    .list
                    .items
                    .iter_mut()
                    .find(|t| t.session_id == session_id)
                {
                    entry.state = closed_state;
                }
                if let Some(chat) = self.ui.session_panel.chat_states.get_mut(&session_id) {
                    chat.finish_all_streaming();
                }
                if thread_agent != Some(AgentName::Opencode) {
                    self.record_agent_conversation_result_if_done(&session_id)
                        .await;
                }
                true
            }
            ManagerEvent::InstanceCrashed {
                workspace,
                reason,
                affected_threads,
            } => {
                if !self.workspace_path_belongs_to_current_workspace(&workspace) {
                    return false;
                }
                for tid in affected_threads {
                    let suspended_state = SessionState::Suspended {
                        reason: reason.clone(),
                    };
                    if let Some(session) = self
                        .ui
                        .conversation
                        .agent_sessions
                        .items
                        .iter_mut()
                        .find(|s| s.session_id == tid)
                    {
                        session.state = suspended_state.clone();
                    }
                    if let Some(entry) = self
                        .ui
                        .session_panel
                        .list
                        .items
                        .iter_mut()
                        .find(|t| t.session_id == tid)
                    {
                        entry.state = suspended_state;
                    }
                    if let Some(chat) = self.ui.session_panel.chat_states.get_mut(&tid) {
                        chat.finish_all_streaming();
                    }
                }
                true
            }
        }
    }
}

fn sync_subagent_sessions(
    state: &mut AppState,
    ui: &mut UiState,
    events: &[minos_ui_protocol::UiEventMessage],
    ts_ms: i64,
) -> bool {
    let mut changed = false;
    for event in events {
        if let minos_ui_protocol::UiEventMessage::SubagentSpawned {
            parent_session_id,
            sub_session_id,
            agent,
            title,
            ..
        } = event
        {
            if let Some(conversation_id) = conversation_id_for_parent(state, ui, parent_session_id)
            {
                state
                    .session_conversations
                    .insert(sub_session_id.clone(), conversation_id);
            }
            changed |= upsert_subagent_session(
                ui,
                parent_session_id,
                sub_session_id,
                *agent,
                title.clone(),
                ts_ms,
            );
        }
    }
    changed
}

fn conversation_id_for_parent(
    state: &AppState,
    ui: &UiState,
    parent_session_id: &str,
) -> Option<String> {
    state
        .session_conversations
        .get(parent_session_id)
        .cloned()
        .or_else(|| {
            ui.nav_level()
                .conversation_id()
                .filter(|_| {
                    ui.conversation
                        .agent_sessions
                        .items
                        .iter()
                        .any(|session| session.session_id == parent_session_id)
                })
                .map(str::to_owned)
        })
}

fn upsert_subagent_session(
    ui: &mut UiState,
    parent_session_id: &str,
    sub_session_id: &str,
    agent: AgentName,
    title: Option<String>,
    ts_ms: i64,
) -> bool {
    if sub_session_id.is_empty()
        || !ui
            .conversation
            .agent_sessions
            .items
            .iter()
            .any(|session| session.session_id == parent_session_id)
    {
        return false;
    }

    if let Some(session) = ui
        .conversation
        .agent_sessions
        .items
        .iter_mut()
        .find(|session| session.session_id == sub_session_id)
    {
        let mut changed = false;
        if session.parent_session_id.as_deref() != Some(parent_session_id) {
            session.parent_session_id = Some(parent_session_id.to_owned());
            changed = true;
        }
        if session.agent != agent {
            session.agent = agent;
            changed = true;
        }
        if title.is_some() && session.title != title {
            session.title = title;
            changed = true;
        }
        return changed;
    }

    ui.conversation
        .agent_sessions
        .items
        .push(crate::backend::SessionSummaryEntry {
            session_id: sub_session_id.to_owned(),
            agent,
            title,
            first_ts_ms: ts_ms,
            last_ts_ms: ts_ms,
            message_count: 0,
            ended_at_ms: None,
            parent_session_id: Some(parent_session_id.to_owned()),
            state: SessionState::Idle,
            needs_continue: false,
        });
    if ui.conversation.agent_sessions.selected.is_none() {
        ui.conversation.agent_sessions.select(Some(0));
    }
    true
}

fn sync_subagent_info(ui: &mut UiState, events: &[minos_ui_protocol::UiEventMessage]) {
    for event in events {
        match event {
            minos_ui_protocol::UiEventMessage::SubagentSpawned {
                parent_session_id,
                sub_session_id,
                tool_call_id,
                agent,
                model,
                prompt,
                title,
            } => {
                ui.conversation.subagent_info.insert(
                    sub_session_id.clone(),
                    crate::ui::SubagentInfo {
                        parent_session_id: parent_session_id.clone(),
                        tool_call_id: tool_call_id.clone(),
                        agent: *agent,
                        model: model.clone(),
                        prompt: prompt.clone(),
                        title: title.clone(),
                        status: minos_ui_protocol::SubagentStatus::Running,
                    },
                );
            }
            minos_ui_protocol::UiEventMessage::SubagentStatusUpdated {
                sub_session_id,
                status,
            } => {
                if let Some(info) = ui.conversation.subagent_info.get_mut(sub_session_id) {
                    info.status = *status;
                }
            }
            _ => {}
        }
    }
}
