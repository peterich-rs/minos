use super::*;

// Production Teamwork MCP is owned by the daemon. These in-process handlers are
// retained for unit tests of conversation/delegation edge cases without a live
// daemon socket. They are intentionally not wired into AppEvent/Effect.
#[allow(dead_code)]
impl App {
    pub(super) async fn handle_mcp_tool_call(
        &mut self,
        request: SocketRequest,
    ) -> anyhow::Result<SocketResponse> {
        match request {
            SocketRequest::Ping => Ok(SocketResponse::Pong),
            SocketRequest::ListConversationMessages {
                conversation_id,
                before_seq,
                limit,
            } => {
                self.ensure_mcp_conversation(&conversation_id)?;
                let messages = self
                    .backend
                    .list_conversation_messages(&conversation_id)
                    .await?;
                let page = conversation_message_page(conversation_id, messages, before_seq, limit);
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(page)?),
                })
            }
            SocketRequest::DelegateToAgent {
                conversation_id,
                source_agent,
                source_thread_id,
                target_agent,
                prompt,
            } => {
                self.ensure_mcp_conversation(&conversation_id)?;
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
                self.validate_mcp_source_thread(
                    &conversation_id,
                    source_agent,
                    source_thread_id.as_deref(),
                )
                .await?;
                self.state
                    .teamwork_store
                    .ensure_delegate_target_allowed(
                        &conversation_id,
                        source_thread_id.as_deref(),
                        target_agent,
                    )
                    .await?;
                let workspace = self.workspace_for_conversation(&conversation_id);
                let outcome = self
                    .backend
                    .start_agent_in_conversation(&conversation_id, target_agent, workspace)
                    .await?;
                self.ensure_thread_visible(
                    outcome.thread_id.clone(),
                    target_agent,
                    outcome.cwd.clone(),
                );
                self.refresh_current_conversation_sessions(&conversation_id)
                    .await;
                self.backend
                    .send_message(&outcome.thread_id, &prompt)
                    .await?;
                let delegation = self
                    .state
                    .teamwork_store
                    .create_delegation(
                        &conversation_id,
                        source_agent,
                        source_thread_id.clone(),
                        target_agent,
                        prompt,
                        Some(outcome.thread_id.clone()),
                    )
                    .await?;
                let visible_prompt = delegation_visible_message(
                    target_agent,
                    &outcome.thread_id,
                    &delegation.prompt,
                );
                let sender_role = if source_agent.is_some() {
                    "agent"
                } else {
                    "user"
                };
                self.backend
                    .append_conversation_message(
                        &conversation_id,
                        None,
                        source_thread_id.as_deref(),
                        sender_role,
                        source_agent,
                        &visible_prompt,
                    )
                    .await?;
                self.refresh_current_conversation_messages(&conversation_id)
                    .await;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::json!({
                        "accepted": true,
                        "target_agent": target_agent.bin_name(),
                        "thread_id": outcome.thread_id,
                        "delegation": delegation,
                    })),
                })
            }
            SocketRequest::GetDelegationStatus {
                conversation_id,
                delegation_id,
            } => {
                self.ensure_mcp_conversation(&conversation_id)?;
                let delegation = self
                    .state
                    .teamwork_store
                    .get_delegation(&conversation_id, &delegation_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("delegation not found: {delegation_id}"))?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(delegation)?),
                })
            }
            SocketRequest::WaitDelegation {
                conversation_id,
                delegation_id,
                timeout_ms,
            } => {
                self.ensure_mcp_conversation(&conversation_id)?;
                let timeout = std::time::Duration::from_millis(timeout_ms as u64);
                let poll = std::time::Duration::from_millis(200);
                let (delegation, timed_out) = self
                    .state
                    .teamwork_store
                    .wait_delegation(&conversation_id, &delegation_id, timeout, poll)
                    .await?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::json!({
                        "status": delegation.status,
                        "timed_out": timed_out,
                        "result_text": delegation.result_text,
                        "error": delegation.error,
                        "result_message_id": delegation.result_message_id,
                        "delegation": delegation,
                    })),
                })
            }
            SocketRequest::CancelDelegation {
                conversation_id,
                delegation_id,
                reason,
            } => {
                self.ensure_mcp_conversation(&conversation_id)?;
                let delegation = self
                    .state
                    .teamwork_store
                    .cancel_delegation(&conversation_id, &delegation_id, reason)
                    .await?;
                if let Some(thread_id) = delegation.thread_id.as_deref() {
                    let _ = self.backend.interrupt_thread(thread_id).await;
                }
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(delegation)?),
                })
            }
            SocketRequest::PostConversationUpdate {
                conversation_id,
                source_agent,
                source_thread_id,
                message,
            } => {
                self.ensure_mcp_conversation(&conversation_id)?;
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?;
                self.validate_mcp_source_thread(
                    &conversation_id,
                    source_agent,
                    source_thread_id.as_deref(),
                )
                .await?;
                let body = mcp_visible_message(message.trim())?;
                let body = self
                    .deliver_post_conversation_update_target(&conversation_id, &body)
                    .await?;
                let sender_role = if source_agent.is_some() {
                    "agent"
                } else {
                    "user"
                };
                self.backend
                    .append_conversation_message(
                        &conversation_id,
                        None,
                        source_thread_id.as_deref(),
                        sender_role,
                        source_agent,
                        &body,
                    )
                    .await?;
                self.refresh_current_conversation_messages(&conversation_id)
                    .await;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::json!({ "accepted": true })),
                })
            }
        }
    }

    async fn validate_mcp_source_thread(
        &self,
        conversation_id: &str,
        source_agent: Option<AgentName>,
        source_thread_id: Option<&str>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            source_agent.is_none() || source_thread_id.is_some(),
            "MCP source_thread_id is required when source_agent is set"
        );
        let Some(source_thread_id) = source_thread_id else {
            return Ok(());
        };
        let sessions = self
            .backend
            .list_conversation_agent_sessions(conversation_id)
            .await?;
        let Some(session) = sessions
            .iter()
            .find(|session| session.thread_id == source_thread_id)
        else {
            anyhow::bail!(
                "MCP source thread {source_thread_id} does not belong to conversation {conversation_id}"
            );
        };
        if let Some(source_agent) = source_agent {
            anyhow::ensure!(
                session.agent == source_agent,
                "MCP source thread {source_thread_id} belongs to {}, not {}",
                session.agent.bin_name(),
                source_agent.bin_name()
            );
        }
        Ok(())
    }

    async fn deliver_post_conversation_update_target(
        &mut self,
        conversation_id: &str,
        body: &str,
    ) -> anyhow::Result<String> {
        let Some((target, prompt)) = parse_agent_routing(body) else {
            return Ok(body.to_owned());
        };
        let prompt = prompt.trim().to_owned();
        if prompt.is_empty() {
            return Ok(body.to_owned());
        }
        if let Some(thread_short_id) = target.thread_short_id {
            let thread_id = self
                .thread_id_for_agent_short_id(target.agent, &thread_short_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No existing {} session matches #{}",
                        target.agent.bin_name(),
                        thread_short_id
                    )
                })?;
            self.backend.send_message(&thread_id, &prompt).await?;
            return Ok(body.to_owned());
        }
        if let Some(error) = self.agent_unavailability_error(target.agent) {
            anyhow::bail!(error);
        }
        let workspace = self.workspace_for_conversation(conversation_id);
        let outcome = self
            .backend
            .start_agent_in_conversation(conversation_id, target.agent, workspace)
            .await?;
        self.ensure_thread_visible(outcome.thread_id.clone(), target.agent, outcome.cwd);
        self.refresh_current_conversation_sessions(conversation_id)
            .await;
        self.backend
            .send_message(&outcome.thread_id, &prompt)
            .await?;
        Ok(delegation_visible_message(
            target.agent,
            &outcome.thread_id,
            &prompt,
        ))
    }

    fn ensure_mcp_conversation(&self, conversation_id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.ui
                .conversations
                .items
                .iter()
                .any(|conversation| conversation.conversation_id == conversation_id)
                || self.ui.nav_level().conversation_id() == Some(conversation_id),
            "MCP request conversation_id does not match a loaded TUI conversation"
        );
        Ok(())
    }

    fn workspace_for_conversation(&self, conversation_id: &str) -> PathBuf {
        let Some(conversation) = self
            .ui
            .conversations
            .items
            .iter()
            .find(|conversation| conversation.conversation_id == conversation_id)
        else {
            return self.state.workspace.clone();
        };
        self.ui
            .projects
            .items
            .iter()
            .find(|project| project.project_id == conversation.project_id)
            .map(|project| project.workspace_path.clone())
            .unwrap_or_else(|| self.state.workspace.clone())
    }

    pub(crate) async fn refresh_current_conversation_messages(&mut self, conversation_id: &str) {
        if self.ui.nav_level().conversation_id() != Some(conversation_id) {
            return;
        }
        match self
            .backend
            .list_conversation_messages(conversation_id)
            .await
        {
            Ok(messages) => {
                self.ui.conversation.set_messages(messages);
                self.ui.conversation.auto_scroll = true;
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::mcp",
                    error = %error,
                    conversation_id,
                    "failed to refresh conversation messages after MCP tool call"
                );
            }
        }
    }

    async fn refresh_current_conversation_sessions(&mut self, conversation_id: &str) {
        if self.ui.nav_level().conversation_id() != Some(conversation_id) {
            return;
        }
        match self
            .backend
            .list_conversation_agent_sessions(conversation_id)
            .await
        {
            Ok(sessions) => {
                self.ui.conversation.agent_sessions.items = sessions;
                self.ui.conversation.agent_sessions.select(
                    if self.ui.conversation.agent_sessions.items.is_empty() {
                        None
                    } else {
                        Some(0)
                    },
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui::mcp",
                    error = %error,
                    conversation_id,
                    "failed to refresh conversation sessions after MCP delegation"
                );
            }
        }
    }
}

#[allow(dead_code)] // used by test-only in-process MCP handlers
fn conversation_message_page(
    conversation_id: String,
    mut messages: Vec<ConversationMessageEntry>,
    before_seq: Option<u64>,
    limit: Option<u32>,
) -> serde_json::Value {
    messages.sort_by_key(|message| message.message_seq);
    let mut descending = messages
        .into_iter()
        .filter(|message| {
            before_seq.is_none_or(|before_seq| message.message_seq < before_seq as i64)
        })
        .rev()
        .collect::<Vec<_>>();
    let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
    let has_more = descending.len() > limit;
    descending.truncate(limit);
    let next_before_seq = if has_more {
        descending.last().map(|message| message.message_seq)
    } else {
        None
    };
    let messages = descending
        .into_iter()
        .map(|message| {
            serde_json::json!({
                "message_seq": message.message_seq,
                "message_id": message.message_id,
                "conversation_id": message.conversation_id,
                "thread_id": message.thread_id,
                "created_at_ms": message.created_at_ms,
                "sender_role": message.sender_role,
                "agent": message.agent.map(|agent| agent.bin_name().to_owned()),
                "body": message.body,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "conversation_id": conversation_id,
        "messages": messages,
        "next_before_seq": next_before_seq,
        "has_more": has_more,
    })
}

#[allow(dead_code)] // used by test-only in-process MCP handlers
fn mcp_visible_message(body: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !body.is_empty(),
        "post_conversation_update message is empty"
    );
    Ok(body.to_owned())
}

#[allow(dead_code)] // used by test-only in-process MCP handlers
fn delegation_visible_message(
    target_agent: AgentName,
    target_thread_id: &str,
    prompt: &str,
) -> String {
    format!(
        "@{}#{} {}",
        target_agent.bin_name(),
        short_thread_id(target_thread_id),
        prompt.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegation_visible_message_matches_room_mention() {
        assert_eq!(
            delegation_visible_message(AgentName::Opencode, "thread-opencode-1234", " fix docs "),
            "@opencode#thread-o fix docs"
        );
    }
}
