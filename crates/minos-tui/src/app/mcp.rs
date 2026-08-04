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
            SocketRequest::ListConversationRoster { conversation_id } => {
                self.ensure_mcp_conversation(&conversation_id)?;
                // Production path is daemon-owned; unit tests get an empty roster.
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::json!({
                        "conversation_id": conversation_id,
                        "members": [],
                    })),
                })
            }
            SocketRequest::DelegateToAgent {
                conversation_id,
                source_agent,
                source_session_id,
                target_agent,
                profile_id,
                target_profile,
                prompt,
            } => {
                // Production MCP is daemon-owned; this in-process path is for unit tests.
                // Profile name/`profile_id` resolution + launch merge stay on the daemon.
                // Here we only resolve agent identity and pass profile_id through start.
                self.ensure_mcp_conversation(&conversation_id)?;
                let prompt = prompt.trim().to_owned();
                anyhow::ensure!(!prompt.is_empty(), "delegate_to_agent prompt is empty");
                let (target_agent, resolved_profile_id) = self
                    .resolve_in_process_delegate_target(
                        target_agent.as_deref(),
                        profile_id.as_deref(),
                        target_profile.as_deref(),
                    )
                    .await?;
                if let Some(error) = self.agent_unavailability_error(target_agent) {
                    anyhow::bail!(error);
                }
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?;
                self.validate_mcp_source_session(
                    &conversation_id,
                    source_agent,
                    source_session_id.as_deref(),
                )
                .await?;
                self.state
                    .teamwork_store
                    .ensure_delegate_target_allowed(
                        &conversation_id,
                        source_session_id.as_deref(),
                        target_agent,
                    )
                    .await?;
                let workspace = self.workspace_for_conversation(&conversation_id);
                let outcome = self
                    .backend
                    .start_agent_in_conversation(
                        &conversation_id,
                        target_agent,
                        workspace,
                        resolved_profile_id.clone(),
                    )
                    .await?;
                self.ensure_thread_visible(
                    outcome.session_id.clone(),
                    target_agent,
                    outcome.cwd.clone(),
                );
                self.refresh_current_conversation_sessions(&conversation_id)
                    .await;
                self.backend
                    .send_message(&outcome.session_id, &prompt, None)
                    .await?;
                let delegation = self
                    .state
                    .teamwork_store
                    .create_delegation(
                        &conversation_id,
                        source_agent,
                        source_session_id.clone(),
                        target_agent,
                        prompt,
                        Some(outcome.session_id.clone()),
                    )
                    .await?;
                let visible_prompt = delegation_visible_message(
                    target_agent,
                    &outcome.session_id,
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
                        source_session_id.as_deref(),
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
                        "profile_id": resolved_profile_id,
                        "session_id": outcome.session_id,
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
                if let Some(session_id) = delegation.session_id.as_deref() {
                    let _ = self.backend.interrupt_session(session_id).await;
                }
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(delegation)?),
                })
            }
            SocketRequest::PostConversationUpdate {
                conversation_id,
                source_agent,
                source_session_id,
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
                self.validate_mcp_source_session(
                    &conversation_id,
                    source_agent,
                    source_session_id.as_deref(),
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
                        source_session_id.as_deref(),
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
            SocketRequest::PostGitUpdate {
                conversation_id,
                source_agent,
                source_session_id,
                activity,
            } => {
                self.ensure_mcp_conversation(&conversation_id)?;
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?;
                self.validate_mcp_source_session(
                    &conversation_id,
                    source_agent,
                    source_session_id.as_deref(),
                )
                .await?;
                let activity: minos_protocol::GitActivity = serde_json::from_value(activity)
                    .map_err(|e| anyhow::anyhow!("invalid git activity payload: {e}"))?;
                // Embed structured payload the same way daemon git::activity does.
                let json = serde_json::to_string(&activity)?;
                let body = format!("Git activity\n\n<!--minos-git-activity:{json}-->");
                let sender_role = if source_agent.is_some() {
                    "agent"
                } else {
                    "user"
                };
                self.backend
                    .append_conversation_message(
                        &conversation_id,
                        None,
                        source_session_id.as_deref(),
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
            SocketRequest::ReactToMessage {
                conversation_id,
                source_agent,
                source_session_id,
                message_id,
                emoji,
            } => {
                // Production path is daemon-owned; unit-test stub enforces the
                // same hard mention gate and returns accepted without durable store.
                self.ensure_mcp_conversation(&conversation_id)?;
                let source_agent = source_agent
                    .as_deref()
                    .map(|agent| {
                        parse_agent_name(agent)
                            .ok_or_else(|| anyhow::anyhow!("unknown source agent: {agent}"))
                    })
                    .transpose()?
                    .ok_or_else(|| anyhow::anyhow!("react_to_message requires source_agent"))?;
                self.validate_mcp_source_session(
                    &conversation_id,
                    Some(source_agent),
                    source_session_id.as_deref(),
                )
                .await?;
                let emoji = emoji.trim();
                anyhow::ensure!(!emoji.is_empty(), "emoji must not be empty");
                let messages = self
                    .backend
                    .list_conversation_messages(&conversation_id)
                    .await?;
                let target = messages
                    .iter()
                    .find(|m| m.message_id == message_id)
                    .ok_or_else(|| anyhow::anyhow!("message not found: {message_id}"))?;
                let body = target.body.as_str();
                let mentions_me = body
                    .to_ascii_lowercase()
                    .contains(&format!("@{}", source_agent.bin_name()));
                anyhow::ensure!(
                    mentions_me,
                    "react_to_message is only allowed on messages that @mention this agent ({})",
                    source_agent.bin_name()
                );
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::json!({
                        "accepted": true,
                        "added": true,
                        "message_id": message_id,
                        "emoji": emoji,
                    })),
                })
            }
        }
    }

    async fn validate_mcp_source_session(
        &self,
        conversation_id: &str,
        source_agent: Option<AgentName>,
        source_session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            source_agent.is_none() || source_session_id.is_some(),
            "MCP source_session_id is required when source_agent is set"
        );
        let Some(source_session_id) = source_session_id else {
            return Ok(());
        };
        let sessions = self
            .backend
            .list_conversation_agent_sessions(conversation_id)
            .await?;
        let Some(session) = sessions
            .iter()
            .find(|session| session.session_id == source_session_id)
        else {
            anyhow::bail!(
                "MCP source thread {source_session_id} does not belong to conversation {conversation_id}"
            );
        };
        if let Some(source_agent) = source_agent {
            anyhow::ensure!(
                session.agent == source_agent,
                "MCP source thread {source_session_id} belongs to {}, not {}",
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
        let profiles = self.ui.mention_profiles();
        let Some((target, prompt)) = parse_agent_routing(body, &profiles) else {
            return Ok(body.to_owned());
        };
        let prompt = prompt.trim().to_owned();
        if prompt.is_empty() {
            return Ok(body.to_owned());
        }
        if let Some(session_short_id) = target.session_short_id {
            let session_id = self
                .session_id_for_agent_short_id(target.agent, &session_short_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No existing {} session matches #{}",
                        target.agent.bin_name(),
                        session_short_id
                    )
                })?;
            self.backend
                .send_message(&session_id, &prompt, None)
                .await?;
            return Ok(body.to_owned());
        }
        if let Some(error) = self.agent_unavailability_error(target.agent) {
            anyhow::bail!(error);
        }
        let workspace = self.workspace_for_conversation(conversation_id);
        // Bare @agent / profile mentions: pass profile_id when route has one;
        // bare agent uses newest profile convenience (same as conversation submit).
        let profile_id = target
            .profile_id
            .clone()
            .or_else(|| self.newest_profile_id_for_agent(target.agent));
        let outcome = self
            .backend
            .start_agent_in_conversation(conversation_id, target.agent, workspace, profile_id)
            .await?;
        self.ensure_thread_visible(outcome.session_id.clone(), target.agent, outcome.cwd);
        self.refresh_current_conversation_sessions(conversation_id)
            .await;
        self.backend
            .send_message(&outcome.session_id, &prompt, None)
            .await?;
        Ok(delegation_visible_message(
            target.agent,
            &outcome.session_id,
            &prompt,
        ))
    }

    /// Resolve agent + profile for the in-process MCP test path (daemon is SSOT in prod).
    async fn resolve_in_process_delegate_target(
        &self,
        target_agent: Option<&str>,
        profile_id: Option<&str>,
        target_profile: Option<&str>,
    ) -> anyhow::Result<(AgentName, Option<String>)> {
        let requested = target_agent
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|name| {
                parse_agent_name(name).ok_or_else(|| anyhow::anyhow!("unknown agent: {name}"))
            })
            .transpose()?;

        if let Some(pid) = profile_id.map(str::trim).filter(|s| !s.is_empty()) {
            let profile = self
                .ui
                .agent_profiles
                .iter()
                .find(|p| p.id == pid)
                .ok_or_else(|| anyhow::anyhow!("agent profile not found: {pid}"))?;
            if let Some(requested) = requested {
                anyhow::ensure!(
                    requested == profile.runtime_agent,
                    "agent mismatch for profile {pid}: request agent is {}, profile runtime is {}",
                    requested.bin_name(),
                    profile.runtime_agent.bin_name()
                );
            }
            return Ok((profile.runtime_agent, Some(profile.id.clone())));
        }

        if let Some(name) = target_profile.map(str::trim).filter(|s| !s.is_empty()) {
            let key = name.to_ascii_lowercase();
            let matches: Vec<_> = self
                .ui
                .agent_profiles
                .iter()
                .filter(|p| p.name.trim().to_ascii_lowercase() == key)
                .collect();
            match matches.as_slice() {
                [] => anyhow::bail!("agent profile not found by name: {name}"),
                [only] => {
                    if let Some(requested) = requested {
                        anyhow::ensure!(
                            requested == only.runtime_agent,
                            "agent mismatch for profile {}: request agent is {}, profile runtime is {}",
                            only.id,
                            requested.bin_name(),
                            only.runtime_agent.bin_name()
                        );
                    }
                    return Ok((only.runtime_agent, Some(only.id.clone())));
                }
                _ => anyhow::bail!(
                    "agent profile name is ambiguous ({} matches): {name}; use profile_id",
                    matches.len()
                ),
            }
        }

        let Some(agent) = requested else {
            anyhow::bail!("delegate_to_agent requires target_agent, profile_id, or target_profile");
        };
        Ok((agent, self.newest_profile_id_for_agent(agent)))
    }

    fn newest_profile_id_for_agent(&self, agent: AgentName) -> Option<String> {
        self.ui
            .agent_profiles
            .iter()
            .filter(|p| p.runtime_agent == agent)
            .max_by_key(|p| p.updated_at_ms)
            .map(|p| p.id.clone())
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
                "session_id": message.session_id,
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
    target_session_id: &str,
    prompt: &str,
) -> String {
    format!(
        "@{}#{} {}",
        target_agent.bin_name(),
        short_session_id(target_session_id),
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
