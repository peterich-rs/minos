use super::*;

impl App {
    pub(super) async fn submit_pending_agent_request(
        &mut self,
        session_id: String,
        pending: PendingAgentRequestKind,
        text: String,
    ) -> bool {
        let request_id = match &pending {
            PendingAgentRequestKind::CodexUserInput { request_id, .. }
            | PendingAgentRequestKind::CodexApproval { request_id, .. }
            | PendingAgentRequestKind::GrokPlanApproval { request_id }
            | PendingAgentRequestKind::GrokUserQuestion { request_id, .. } => request_id.clone(),
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
                    .send_approval_decision(&request_id, &session_id, decision)
                    .await
            }
            PendingAgentRequestKind::CodexApproval { request_id, method } => {
                let decision = codex_approval_decision(&method, text.as_str());
                self.backend
                    .send_approval_decision(&request_id, &session_id, decision)
                    .await
            }
            PendingAgentRequestKind::GrokPlanApproval { request_id } => {
                let decision = grok_plan_approval_decision(text.as_str());
                self.backend
                    .send_approval_decision(&request_id, &session_id, decision)
                    .await
            }
            PendingAgentRequestKind::GrokUserQuestion {
                request_id,
                questions,
            } => {
                let decision = grok_user_question_decision(questions.as_slice(), text.as_str());
                self.backend
                    .send_approval_decision(&request_id, &session_id, decision)
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
                    .respond_opencode_permission(&session_id, &permission_id, &response)
                    .await
            }
            PendingAgentRequestKind::OpencodeQuestion {
                question_id,
                questions,
            } => {
                let answers = opencode_question_answers(questions.as_slice(), text.as_str());
                self.backend
                    .respond_opencode_question(&session_id, &question_id, answers)
                    .await
            }
        };

        match result {
            Ok(()) => {
                if let Some(chat) = self.ui.session_panel.chat_states.get_mut(&session_id) {
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

    pub(super) async fn dispatch_prompt_to_agent(
        &mut self,
        agent: AgentName,
        text: String,
        message_body: String,
    ) -> bool {
        if let Some(tx) = self.event_tx.clone() {
            if let Some(error) = self.agent_unavailability_error(agent) {
                self.ui.set_error(error);
                return true;
            }
            let backend = Arc::clone(&self.backend);
            let workspace = self.current_project_workspace();
            tokio::spawn(async move {
                match backend.start_agent(agent, workspace).await {
                    Ok(outcome) => {
                        let _ = tx.send(AppEvent::AgentStartedForPrompt {
                            agent,
                            session_id: outcome.session_id,
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
                            session_id: agent.bin_name().to_owned(),
                            error,
                        });
                    }
                }
            });
            return true;
        }

        match self.start_new_thread(agent).await {
            Ok(session_id) => {
                self.send_text_to_thread(session_id, text, Some(message_body))
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
        session_id: String,
        text: String,
        message_body: Option<String>,
    ) -> bool {
        if let Some(index) = self
            .ui
            .session_panel
            .list
            .items
            .iter()
            .position(|thread| thread.session_id == session_id)
        {
            self.select_thread(index);
        }
        if let Some(chat) = self.ui.current_chat_mut() {
            chat.scroll_to_bottom();
        }

        self.hydrate_thread_if_needed(&session_id).await;

        let conversation_message = message_body.as_ref().and_then(|body| {
            self.ui
                .nav_level()
                .conversation_id()
                .map(|conversation_id| (conversation_id.to_owned(), body.clone()))
        });
        if let Some(tx) = self.event_tx.clone() {
            let backend = Arc::clone(&self.backend);
            let conversation_message = conversation_message.clone();
            tokio::spawn(async move {
                if let Some((conversation_id, body)) = conversation_message {
                    if let Err(error) = backend
                        .append_conversation_message(
                            &conversation_id,
                            None,
                            None,
                            "user",
                            None,
                            &body,
                        )
                        .await
                    {
                        let error = format!("Failed to save conversation message: {error}");
                        tracing::warn!(
                            target: "minos_tui::app",
                            error = %error,
                            conversation_id = %conversation_id,
                            session_id = %session_id,
                            "append_conversation_message failed before send"
                        );
                        let _ = tx.send(AppEvent::SendMessageFailed {
                            session_id: session_id.clone(),
                            error,
                        });
                        return;
                    }
                }
                if let Err(e) = backend.resume_session(&session_id, false).await {
                    tracing::debug!(
                        target: "minos_tui::app",
                        error = %e,
                        session_id = %session_id,
                        "resume_session failed or not needed"
                    );
                }
                if let Err(error) = backend.send_message(&session_id, &text).await {
                    let error = format_error_chain(&error);
                    tracing::warn!(
                        target: "minos_tui::app",
                        error = %error,
                        session_id = %session_id,
                        "background send_message failed"
                    );
                    let _ = tx.send(AppEvent::SendMessageFailed { session_id, error });
                }
            });
            return true;
        }

        if let Some((conversation_id, body)) = conversation_message {
            if let Err(error) = self
                .backend
                .append_conversation_message(&conversation_id, None, None, "user", None, &body)
                .await
            {
                tracing::warn!(
                    target: "minos_tui::app",
                    error = %error,
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    "append_conversation_message failed before send"
                );
                self.ui
                    .set_error(format!("Failed to record conversation message: {error}"));
            }
        }
        if let Err(e) = self.backend.resume_session(&session_id, false).await {
            tracing::debug!(
                target: "minos_tui::app",
                error = %e,
                session_id = %session_id,
                "resume_session failed or not needed"
            );
        }
        if let Err(error) = self.backend.send_message(&session_id, &text).await {
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
                .start_agent(agent, self.current_project_workspace())
                .await
            {
                Ok(outcome) => {
                    let session_id = outcome.session_id.clone();
                    self.ensure_thread_visible(session_id.clone(), agent, outcome.cwd);
                    Ok(session_id)
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

    fn current_project_workspace(&self) -> PathBuf {
        let Some(project_id) = self.ui.nav_level().project_id() else {
            return self.state.workspace.clone();
        };
        self.ui
            .projects
            .items
            .iter()
            .find(|project| project.project_id == project_id)
            .map(|project| project.workspace_path.clone())
            .unwrap_or_else(|| self.state.workspace.clone())
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
        session_id: String,
        agent: AgentName,
        workspace: PathBuf,
    ) {
        if let Some(index) = self
            .ui
            .session_panel
            .list
            .items
            .iter()
            .position(|thread| thread.session_id == session_id)
        {
            if let Some(entry) = self.ui.session_panel.list.items.get_mut(index) {
                entry.agent = agent;
                entry.workspace = workspace;
            }
            self.ensure_chat_state_agent(&session_id, agent);
            self.select_thread(index);
            self.ui.focus.focus(PaneId::Input);
            self.sync_input_agent_picker();
            return;
        }

        self.ui.session_panel.list.items.push(SessionEntry {
            session_id: session_id.clone(),
            agent,
            workspace,
            state: SessionState::Starting,
            parent_session_id: None,
        });
        self.ensure_chat_state_agent(&session_id, agent);
        self.select_thread(self.ui.session_panel.list.items.len().saturating_sub(1));
        self.ui.focus.focus(PaneId::Input);
        self.sync_input_agent_picker();
    }

    // Used by unit tests and the test-only in-process MCP handlers.
    #[allow(dead_code)]
    pub(super) fn session_id_for_agent_short_id(
        &self,
        agent: AgentName,
        short_id: &str,
    ) -> Option<String> {
        let short_id = short_id.to_ascii_lowercase();
        self.ui.nav_level().conversation_id()?;
        self.ui
            .conversation
            .agent_sessions
            .items
            .iter()
            .filter(|session| session.parent_session_id.is_none())
            .filter(|session| thread_can_receive_message(&session.state))
            .find(|session| {
                session.agent == agent
                    && (short_session_id(&session.session_id).to_ascii_lowercase() == short_id
                        || session
                            .session_id
                            .to_ascii_lowercase()
                            .starts_with(&short_id))
            })
            .map(|session| session.session_id.clone())
    }

    pub(super) fn sync_input_agent_picker(&mut self) {
        let candidates = self.ui.conversation_agent_mention_candidates();
        self.ui
            .inputs
            .conversation
            .sync_agent_picker(candidates.as_slice(), self.ui.focus.is(PaneId::Input));
    }
}
