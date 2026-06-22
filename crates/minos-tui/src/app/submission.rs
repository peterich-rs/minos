use super::*;

impl App {
    pub(super) async fn submit_pending_agent_request(
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

    pub(super) async fn invite_agent_to_room(
        &mut self,
        agent: AgentName,
        group_text: String,
    ) -> bool {
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

    pub(super) async fn dispatch_prompt_to_existing_agent(
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

    pub(super) async fn dispatch_prompt_to_agent(
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
            let workspace = self.state.workspace.clone();
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

    pub(super) async fn send_text_to_thread(
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

        let conversation_message = group_text.as_ref().and_then(|body| {
            self.ui
                .nav_level()
                .conversation_id()
                .map(|conversation_id| (conversation_id.to_owned(), body.clone()))
        });
        if conversation_message.is_none() {
            if let Some(group_text) = group_text.as_ref() {
                self.record_user_group_message(&thread_id, group_text.clone())
                    .await;
            }
        }

        if let Some(tx) = self.event_tx.clone() {
            let backend = Arc::clone(&self.backend);
            let conversation_message = conversation_message.clone();
            tokio::spawn(async move {
                if let Some((conversation_id, body)) = conversation_message {
                    if let Err(error) = backend
                        .append_conversation_message(&conversation_id, None, "user", None, &body)
                        .await
                    {
                        tracing::warn!(
                            target: "minos_tui::app",
                            error = %error,
                            conversation_id = %conversation_id,
                            thread_id = %thread_id,
                            "append_conversation_message failed before send"
                        );
                    }
                }
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

        if let Some((conversation_id, body)) = conversation_message {
            if let Err(error) = self
                .backend
                .append_conversation_message(&conversation_id, None, "user", None, &body)
                .await
            {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    conversation_id = %conversation_id,
                    thread_id = %thread_id,
                    "append_conversation_message failed before send"
                );
                self.ui
                    .set_error(format!("Failed to record conversation message: {error}"));
            }
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

    pub(super) async fn start_new_thread(&mut self, agent: AgentName) -> Result<String, String> {
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
                .start_agent(agent, self.state.workspace.clone())
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

    pub(super) fn agent_unavailability_error(&self, agent: AgentName) -> Option<String> {
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

    pub(super) fn ensure_thread_visible(
        &mut self,
        thread_id: String,
        agent: AgentName,
        workspace: PathBuf,
    ) {
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
            self.ui.focus.focus(PaneId::Input);
            self.sync_input_agent_picker();
            return;
        }

        self.ui.threads.push(ThreadEntry {
            thread_id: thread_id.clone(),
            agent,
            workspace,
            state: ThreadState::Starting,
            parent_thread_id: None,
        });
        self.ensure_chat_state_agent(&thread_id, agent);
        self.select_thread(self.ui.threads.len().saturating_sub(1));
        self.ui.focus.focus(PaneId::Input);
        self.sync_input_agent_picker();
    }

    pub(super) fn thread_id_for_agent_short_id(
        &self,
        agent: AgentName,
        short_id: &str,
    ) -> Option<String> {
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

    pub(super) fn sync_input_agent_picker(&mut self) {
        let candidates = self.ui.room_agent_mention_candidates();
        self.ui
            .room_input
            .sync_agent_picker(candidates.as_slice(), self.ui.focus.is(PaneId::Input));
    }
}
