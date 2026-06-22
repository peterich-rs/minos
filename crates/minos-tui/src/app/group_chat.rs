use super::*;

impl App {
    pub(super) async fn load_group_chat_history(&mut self) {
        if self.load_group_chat_history_from_backend().await {
            return;
        }

        let sessions = match self.state.group_chat_store.list_agent_sessions().await {
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
        match self.state.group_chat_store.load_recent(500).await {
            Ok(messages) => {
                let messages = self.filter_group_messages_for_current_workspace(messages);
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

    pub(super) async fn load_group_chat_history_from_backend(&mut self) -> bool {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            return false;
        }

        let room_id = self.state.group_chat_store.room_id().to_owned();
        match self
            .backend
            .read_group_chat(&room_id, None, None, 500)
            .await
        {
            Ok(messages) => {
                let messages = self.filter_group_messages_for_current_workspace(messages);
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

    pub(super) async fn refresh_group_chat_from_backend(&mut self) -> bool {
        if !matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            return false;
        }
        let after_seq = self.ui.group_chat.last_seq();
        let room_id = self.state.group_chat_store.room_id().to_owned();
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
        let messages = self.filter_group_messages_for_current_workspace(messages);
        if messages.is_empty() {
            return false;
        }
        self.restore_agent_entries_from_group_messages(messages.as_slice());
        self.ui.group_chat.merge_messages(messages);
        true
    }

    pub(super) fn restore_agent_entries_from_group_sessions(
        &mut self,
        sessions: &[minos_chat_store::ChatAgentSession],
    ) {
        for session in sessions {
            if !self.workspace_path_belongs_to_current_workspace(Path::new(&session.workspace_root))
            {
                continue;
            }
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

    pub(super) fn restore_agent_entries_from_group_messages(
        &mut self,
        messages: &[LocalGroupChatMessage],
    ) {
        for message in messages {
            let (Some(agent), Some(thread_id)) = (message.agent, message.thread_id.as_deref())
            else {
                continue;
            };
            if thread_id.is_empty() {
                continue;
            }
            if !self.group_message_belongs_to_current_workspace(message) {
                continue;
            }

            let workspace = message
                .workspace
                .as_deref()
                .filter(|workspace| !workspace.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| self.state.workspace.clone());
            self.restore_agent_entry(agent, thread_id, workspace);
        }

        if self.ui.selected_thread.is_none() && !self.ui.threads.is_empty() {
            self.select_thread(0);
        }
    }

    pub(super) fn restore_agent_entry(
        &mut self,
        agent: AgentName,
        thread_id: &str,
        workspace: PathBuf,
    ) {
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
            parent_thread_id: None,
        });
        self.ensure_chat_state_agent(thread_id, agent);
    }
}

impl App {
    pub(super) async fn record_user_group_message(&mut self, thread_id: &str, text: String) {
        let Some(message) = self.group_message(thread_id, LocalGroupChatMessageKind::User, text)
        else {
            return;
        };
        self.append_group_chat_message(message).await;
    }

    pub(super) async fn record_user_group_message_for_agent(
        &mut self,
        agent: AgentName,
        text: String,
    ) {
        self.append_group_chat_message(LocalGroupChatMessage {
            seq: 0,
            message_id: String::new(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            kind: LocalGroupChatMessageKind::User,
            text,
            agent: Some(agent),
            thread_id: None,
            thread_short_id: None,
            workspace: Some(self.state.workspace.display().to_string()),
        })
        .await;
    }

    pub(super) async fn record_agent_group_result_if_done(&mut self, thread_id: &str) {
        self.record_agent_group_result(thread_id, false).await;
    }

    pub(super) async fn record_agent_group_result_if_ingest_done(
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

    pub(super) async fn record_agent_group_result(
        &mut self,
        thread_id: &str,
        allow_ingest_done: bool,
    ) {
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
            .state
            .recorded_agent_results
            .get(thread_id)
            .is_some_and(|recorded| recorded == &message_key)
        {
            return;
        }
        if self.group_chat_has_agent_result(thread_id, &text) {
            self.state
                .recorded_agent_results
                .insert(thread_id.to_owned(), message_key);
            return;
        }
        let message_id = Some(self.group_message_id_for_agent_result(thread_id, &message_key));
        let Some(message) = self.group_message_with_id(
            thread_id,
            LocalGroupChatMessageKind::AgentResult,
            text,
            message_id,
        ) else {
            return;
        };
        if self.upsert_group_chat_message(message).await {
            self.state
                .recorded_agent_results
                .insert(thread_id.to_owned(), message_key);
        }
    }

    pub(super) fn group_chat_has_agent_result(&self, thread_id: &str, text: &str) -> bool {
        self.ui.group_chat.messages.iter().any(|message| {
            message.kind == LocalGroupChatMessageKind::AgentResult
                && message.thread_id.as_deref() == Some(thread_id)
                && message.text == text
        })
    }

    pub(super) fn group_message(
        &self,
        thread_id: &str,
        kind: LocalGroupChatMessageKind,
        text: String,
    ) -> Option<LocalGroupChatMessage> {
        self.group_message_with_id(thread_id, kind, text, None)
    }

    pub(super) fn group_message_with_id(
        &self,
        thread_id: &str,
        kind: LocalGroupChatMessageKind,
        text: String,
        message_id: Option<String>,
    ) -> Option<LocalGroupChatMessage> {
        let thread = self
            .ui
            .threads
            .iter()
            .find(|thread| thread.thread_id == thread_id)?;
        Some(LocalGroupChatMessage {
            seq: 0,
            message_id: message_id.unwrap_or_default(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            kind,
            text,
            agent: Some(thread.agent),
            thread_id: Some(thread.thread_id.clone()),
            thread_short_id: Some(short_thread_id(&thread.thread_id)),
            workspace: Some(thread.workspace.display().to_string()),
        })
    }

    pub(super) async fn append_group_chat_message(
        &mut self,
        message: LocalGroupChatMessage,
    ) -> bool {
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

    pub(super) async fn append_group_chat_message_result(
        &mut self,
        message: LocalGroupChatMessage,
    ) -> anyhow::Result<LocalGroupChatMessage> {
        let message = self.state.group_chat_store.append(message).await?;
        self.ui.group_chat.push_message(message.clone());
        Ok(message)
    }

    pub(super) async fn upsert_group_chat_message_result(
        &mut self,
        message: LocalGroupChatMessage,
    ) -> anyhow::Result<LocalGroupChatMessage> {
        let message = self.state.group_chat_store.upsert(message).await?;
        self.ui.group_chat.push_message(message.clone());
        Ok(message)
    }

    pub(super) async fn upsert_group_chat_message(
        &mut self,
        message: LocalGroupChatMessage,
    ) -> bool {
        match self.upsert_group_chat_message_result(message).await {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    "failed to upsert group chat message"
                );
                self.ui
                    .set_error(format!("Failed to update group chat message: {error}"));
                false
            }
        }
    }

    pub(super) fn group_message_id_for_agent_result(
        &self,
        thread_id: &str,
        source_message_id: &str,
    ) -> String {
        group_agent_result_message_id(
            self.state.group_chat_store.room_id(),
            thread_id,
            source_message_id,
        )
    }
}
