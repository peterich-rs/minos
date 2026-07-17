use super::*;

impl App {
    pub(super) async fn record_agent_conversation_result_if_done(&mut self, thread_id: &str) {
        self.record_agent_conversation_result(thread_id, false)
            .await;
    }

    pub(super) async fn record_agent_conversation_result_if_ingest_done(
        &mut self,
        thread_id: &str,
        allow_ingest_done: bool,
    ) {
        let is_opencode = self
            .ui
            .thread_panel.list.items
            .iter()
            .find(|thread| thread.thread_id == thread_id)
            .is_some_and(|thread| thread.agent == AgentName::Opencode);
        if is_opencode && !allow_ingest_done {
            return;
        }
        self.record_agent_conversation_result(thread_id, allow_ingest_done)
            .await;
    }

    async fn record_agent_conversation_result(&mut self, thread_id: &str, allow_ingest_done: bool) {
        // Daemon owns agent-result writeback and delegation completion so TUI
        // offline still closes the loop. Keep this path as a no-op to avoid
        // double-writing conversation messages.
        let _ = (thread_id, allow_ingest_done);
        return;

        #[allow(unreachable_code)]
        let Some(thread) = self
            .ui
            .thread_panel.list.items
            .iter()
            .find(|thread| thread.thread_id == thread_id)
        else {
            return;
        };
        if thread.parent_thread_id.is_some() {
            return;
        }
        if !thread_is_done(&thread.state) && !allow_ingest_done {
            return;
        }
        let Some(chat) = self.ui.thread_panel.chat_states.get(thread_id) else {
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
        if self
            .append_agent_result_to_conversation(thread_id, &message_key, &text)
            .await
        {
            self.state
                .recorded_agent_results
                .insert(thread_id.to_owned(), message_key);
        }
    }

    async fn append_agent_result_to_conversation(
        &mut self,
        thread_id: &str,
        message_key: &str,
        text: &str,
    ) -> bool {
        let Some(conversation_id) = self.conversation_id_for_thread(thread_id) else {
            return false;
        };
        let message_id =
            conversation_agent_result_message_id(&conversation_id, thread_id, message_key);
        let delegation = self
            .state
            .teamwork_store
            .running_delegation_for_thread(&conversation_id, thread_id)
            .await
            .unwrap_or(None);
        let body = delegation
            .as_ref()
            .and_then(|delegation| delegation_result_visible_message(delegation, text))
            .unwrap_or_else(|| text.to_owned());
        let visible = self.ui.nav_level().conversation_id() == Some(conversation_id.as_str());
        if visible
            && self
                .ui
                .conversation.messages
                .iter()
                .any(|message| message.message_id == message_id)
        {
            if let Some(delegation) = delegation.as_ref() {
                self.complete_and_deliver_delegation_result(
                    &conversation_id,
                    thread_id,
                    &message_id,
                    text,
                    delegation,
                    &body,
                )
                .await;
            }
            return true;
        }
        let agent = self
            .ui
            .thread_panel.list.items
            .iter()
            .find(|thread| thread.thread_id == thread_id)
            .map(|thread| thread.agent);
        if let Err(error) = self
            .backend
            .append_conversation_message(
                &conversation_id,
                Some(&message_id),
                Some(thread_id),
                "agent",
                agent,
                &body,
            )
            .await
        {
            tracing::warn!(
                target: "minos_tui::app",
                error = %error,
                conversation_id = %conversation_id,
                thread_id = %thread_id,
                message_id = %message_id,
                "append agent result to conversation failed"
            );
            return false;
        }
        if !visible {
            if let Some(delegation) = delegation.as_ref() {
                self.complete_and_deliver_delegation_result(
                    &conversation_id,
                    thread_id,
                    &message_id,
                    text,
                    delegation,
                    &body,
                )
                .await;
            }
            return true;
        }
        let now = chrono::Utc::now().timestamp_millis();
        self.ui
            .conversation.messages
            .push(ConversationMessageEntry {
                message_seq: now,
                message_id: message_id.clone(),
                conversation_id: conversation_id.clone(),
                thread_id: Some(thread_id.to_owned()),
                created_at_ms: now,
                sender_role: "agent".into(),
                agent,
                body: body.clone(),
                reply_to_message_id: None,
                delegation_id: None,
                mentions: Vec::new(),
            });
        self.ui.conversation.auto_scroll = true;
        if let Some(conversation) = self
            .ui
            .conversations
            .items.iter_mut()
            .find(|conversation| conversation.conversation_id == conversation_id)
        {
            conversation.message_count = conversation.message_count.saturating_add(1);
            conversation.last_message_preview = Some(body.chars().take(120).collect());
            conversation.updated_at_ms = now;
        }
        if let Some(delegation) = delegation.as_ref() {
            self.complete_and_deliver_delegation_result(
                &conversation_id,
                thread_id,
                &message_id,
                text,
                delegation,
                &body,
            )
            .await;
        }
        true
    }

    async fn complete_and_deliver_delegation_result(
        &self,
        conversation_id: &str,
        thread_id: &str,
        message_id: &str,
        text: &str,
        delegation: &minos_chat_store::TeamworkDelegation,
        visible_body: &str,
    ) {
        match self
            .state
            .teamwork_store
            .complete_delegation_for_thread(conversation_id, thread_id, Some(message_id), text)
            .await
        {
            Ok(Some(_)) => {
                self.deliver_delegation_result_to_source(delegation, thread_id, visible_body)
                    .await;
            }
            Ok(None) => {}
            Err(error) => tracing::debug!(
                target: "minos_tui::app",
                error = %error,
                conversation_id,
                thread_id,
                message_id,
                "complete delegation for agent result skipped"
            ),
        }
    }

    async fn deliver_delegation_result_to_source(
        &self,
        delegation: &minos_chat_store::TeamworkDelegation,
        target_thread_id: &str,
        visible_body: &str,
    ) {
        let Some(source_thread_id) = delegation.source_thread_id.as_deref() else {
            return;
        };
        if source_thread_id == target_thread_id {
            return;
        }
        let source_body =
            delegation_result_source_message(delegation, target_thread_id, visible_body);
        if let Err(error) = self
            .backend
            .send_message(source_thread_id, &source_body)
            .await
        {
            tracing::warn!(
                target: "minos_tui::app",
                error = %error,
                conversation_id = %delegation.conversation_id,
                source_thread_id,
                target_thread_id,
                delegation_id = %delegation.delegation_id,
                "failed to deliver delegation result to source thread"
            );
        }
    }

    fn conversation_id_for_thread(&self, thread_id: &str) -> Option<String> {
        self.state
            .thread_conversations
            .get(thread_id)
            .cloned()
            .or_else(|| {
                self.ui
                    .nav_level()
                    .conversation_id()
                    .filter(|_| {
                        self.ui
                            .conversation.agent_sessions.items
                            .iter()
                            .any(|session| session.thread_id == thread_id)
                    })
                    .map(str::to_owned)
            })
    }
}

fn delegation_result_visible_message(
    delegation: &minos_chat_store::TeamworkDelegation,
    text: &str,
) -> Option<String> {
    let source_agent = delegation.source_agent?;
    let source_thread_id = delegation.source_thread_id.as_deref()?;
    Some(format!(
        "@{}#{} {}",
        source_agent.bin_name(),
        short_thread_id(source_thread_id),
        text.trim()
    ))
}

fn delegation_result_source_message(
    delegation: &minos_chat_store::TeamworkDelegation,
    target_thread_id: &str,
    visible_body: &str,
) -> String {
    format!(
        "[{}#{}] {}",
        delegation.target_agent.bin_name(),
        short_thread_id(target_thread_id),
        visible_body
    )
}
