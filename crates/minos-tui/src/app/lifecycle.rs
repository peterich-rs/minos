use super::*;

impl App {
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
        self.resolve_startup_project().await;
        Ok(())
    }

    async fn resolve_startup_project(&mut self) {
        let projects = match self.backend.list_projects().await {
            Ok(p) => p,
            Err(e) => {
                self.ui.set_error(format!("Failed to load projects: {e}"));
                return;
            }
        };
        self.ui.projects = projects.clone();
        self.ui.selected_project = if self.ui.projects.is_empty() {
            None
        } else {
            Some(0)
        };
        self.ui.project_list_state.select(self.ui.selected_project);

        let cwd = &self.state.workspace;
        let matched = projects.into_iter().find(|p| {
            crate::state::workspace_path_belongs_to_current_workspace(
                cwd,
                &p.workspace_path,
            )
        });

        match matched {
            Some(project) => {
                let threads = self
                    .backend
                    .list_project_threads(&project.project_id)
                    .await
                    .unwrap_or_default();
                self.ui.project_sessions = threads;
                self.ui.selected_thread =
                    if self.ui.project_sessions.is_empty() { None } else { Some(0) };
                self.ui.room_list_state.select(self.ui.selected_thread);
                self.ui.selected_project = self
                    .ui
                    .projects
                    .iter()
                    .position(|p| p.project_id == project.project_id);
                self.ui.project_list_state.select(self.ui.selected_project);
                self.ui.nav_level = crate::nav::NavLevel::Sessions {
                    project_id: project.project_id,
                };
            }
            None => {
                let dir_name = cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "workspace".to_owned());
                self.ui.startup_create_prompt = Some(crate::ui::StartupCreatePromptState {
                    dir_name,
                    path: cwd.to_string_lossy().into_owned(),
                });
            }
        }
    }

    pub async fn handle_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Ingest(ingest) => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::IngestArrived(ingest),
                ))
                .await
            }
            AppEvent::ManagerEvent(event) => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ManagerEvent(event),
                ))
                .await
            }
            AppEvent::AgentStartedForPrompt {
                agent,
                thread_id,
                cwd,
                text,
            } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::AgentStarted {
                        agent,
                        thread_id,
                        cwd,
                        text,
                    },
                ))
                .await
            }
            AppEvent::SendMessageFailed { thread_id, error } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::SendFailed { thread_id, error },
                ))
                .await
            }
            AppEvent::ProjectsLoaded(projects) => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ProjectsLoaded(projects),
                ))
                .await
            }
            AppEvent::ProjectCreated(project) => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ProjectCreated(project),
                ))
                .await
            }
            AppEvent::ProjectThreadsLoaded { project_id, threads } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ProjectThreadsLoaded { project_id, threads },
                ))
                .await
            }
            AppEvent::ProjectSessionStarted {
                project_id,
                agent,
                thread_id,
                cwd,
                text,
            } => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ProjectSessionStarted {
                        project_id,
                        agent,
                        thread_id,
                        cwd,
                        text,
                    },
                ))
                .await
            }
            AppEvent::ProjectFailed(error) => {
                self.apply_action(Action::EffectCompleted(
                    crate::action::EffectResult::ProjectFailed(error),
                ))
                .await
            }
            AppEvent::McpToolCall(event) => {
                self.apply_action(Action::Global(GlobalAction::McpToolCall(event)))
                    .await
            }
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Paste(text) => self.handle_paste(text).await,
            AppEvent::Mouse(mouse) => self.handle_mouse(mouse).await,
            AppEvent::Tick => self.apply_action(Action::Global(GlobalAction::Tick)).await,
            AppEvent::Resize(_, _) => {
                self.request_frame();
                true
            }
        }
    }

    pub(super) async fn handle_ingest(&mut self, ingest: minos_protocol::LocalIngestFrame) -> bool {
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

    pub(super) async fn handle_tick(&mut self) -> bool {
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

    pub fn set_frame_requester(&mut self, requester: crate::frame::FrameRequester) {
        self.frame_requester = Some(requester);
    }

    pub(super) fn request_frame(&self) {
        if let Some(requester) = &self.frame_requester {
            requester.schedule_frame();
        }
    }

    pub(super) async fn hydrate_daemon_threads(&mut self) {
        let _ = self.sync_daemon_threads_from_backend(false).await;
    }

    pub(super) async fn sync_daemon_threads_if_due(&mut self) -> bool {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            return false;
        }
        let now = Instant::now();
        if self
            .state
            .last_daemon_history_sync
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return false;
        }
        self.state.last_daemon_history_sync = Some(now);
        self.sync_daemon_threads_from_backend(true).await
    }

    pub(super) async fn sync_daemon_threads_from_backend(&mut self, incremental: bool) -> bool {
        let mut changed = false;
        match self.backend.list_threads().await {
            Ok(threads) => {
                if self.prune_external_threads() {
                    changed = true;
                }
                for snap in threads {
                    if !self.workspace_path_belongs_to_current_workspace(&snap.workspace) {
                        if self.remove_thread_local_state(&snap.thread_id) {
                            changed = true;
                        }
                        continue;
                    }
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
                    if incremental && self.state.hydrated_threads.contains(&snap.thread_id) {
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

    pub(super) async fn replay_thread_history_from(
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
                    self.state.hydrated_threads.insert(thread_id.to_owned());
                }
                return changed;
            };
            from_seq = Some(next_seq.saturating_sub(1));
        }
    }

    pub(super) async fn replay_thread_history_after_watermark(&mut self, thread_id: &str) -> bool {
        let from_seq = self.state.thread_watermarks.get(thread_id).copied();
        self.replay_thread_history_from(thread_id, from_seq, false)
            .await
    }

    pub(super) async fn retry_pending_agent_group_results_if_due(&mut self) -> bool {
        let now = Instant::now();
        if self
            .state
            .last_group_result_retry
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return false;
        }
        self.state.last_group_result_retry = Some(now);

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

    pub(super) async fn hydrate_thread_if_needed(&mut self, thread_id: &str) -> bool {
        if self.state.hydrated_threads.contains(thread_id) {
            return false;
        }
        self.replay_thread_history_from(thread_id, None, true).await
    }

    pub(super) async fn ensure_project_session_visible(&mut self, thread_id: &str) {
        if !self.ui.threads.iter().any(|t| t.thread_id == thread_id) {
            if let Some(session) = self
                .ui
                .project_sessions
                .iter()
                .find(|s| s.thread_id == thread_id)
            {
                self.ui.threads.push(crate::ui::ThreadEntry {
                    thread_id: thread_id.to_owned(),
                    agent: session.agent,
                    workspace: self.state.workspace.clone(),
                    state: minos_agent_runtime::ThreadState::Idle,
                });
            }
        }
        if !self.state.hydrated_threads.contains(thread_id) {
            self.hydrate_thread_if_needed(thread_id).await;
        }
    }

    pub(super) fn ensure_chat_state_agent(&mut self, thread_id: &str, agent: AgentName) {
        match self.ui.chat_states.entry(thread_id.to_owned()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().agent != agent {
                    entry.insert(ChatState::new(thread_id.to_owned(), agent));
                    self.state.hydrated_threads.remove(thread_id);
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(ChatState::new(thread_id.to_owned(), agent));
            }
        }
    }

    pub(super) fn workspace_path_belongs_to_current_workspace(&self, workspace: &Path) -> bool {
        state::workspace_path_belongs_to_current_workspace(&self.state.workspace, workspace)
    }

    pub(super) fn group_message_belongs_to_current_workspace(
        &self,
        message: &LocalGroupChatMessage,
    ) -> bool {
        state::group_message_belongs_to_current_workspace(&self.state.workspace, message)
    }

    pub(super) fn filter_group_messages_for_current_workspace(
        &self,
        messages: Vec<LocalGroupChatMessage>,
    ) -> Vec<LocalGroupChatMessage> {
        state::filter_group_messages_for_current_workspace(&self.state.workspace, messages)
    }

    pub(super) fn prune_external_threads(&mut self) -> bool {
        if !state::prune_external_threads(&mut self.state, &mut self.ui) {
            return false;
        }
        self.sync_input_agent_picker();
        true
    }

    pub(super) fn remove_thread_local_state(&mut self, thread_id: &str) -> bool {
        if !state::remove_thread_local_state(&mut self.state, &mut self.ui, thread_id) {
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
            ManagerEvent::ThreadAdded {
                thread_id,
                workspace,
                agent,
            } => {
                if !self.workspace_path_belongs_to_current_workspace(&workspace) {
                    return self.remove_thread_local_state(&thread_id);
                }
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
                self.ui.focus.focus(PaneId::RoomInput);
                self.sync_input_agent_picker();
                true
            }
            ManagerEvent::ThreadStateChanged { thread_id, new, .. } => {
                if !self.ui.threads.iter().any(|t| t.thread_id == thread_id) {
                    return false;
                }
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
                if !self.ui.threads.iter().any(|t| t.thread_id == thread_id) {
                    return false;
                }
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
                workspace,
                reason,
                affected_threads,
            } => {
                if !self.workspace_path_belongs_to_current_workspace(&workspace) {
                    return false;
                }
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
}
