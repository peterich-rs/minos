use super::*;

impl App {
    pub(super) async fn handle_mcp_tool_call(
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
                    .state
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
                    .state
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
                    .state
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
                    .state
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
                        workspace: Some(self.state.workspace.display().to_string()),
                    })
                    .await?;
                let feedback = self
                    .state
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
                    .state
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
                    workspace: Some(self.state.workspace.display().to_string()),
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
                    .state
                    .group_chat_store
                    .react_to_message(source_agent, message_id, message_seq, emoji, action)
                    .await?;
                Ok(SocketResponse::Ok {
                    data: Some(serde_json::to_value(reaction)?),
                })
            }
        }
    }

    pub(super) fn ensure_mcp_room(&self, room_id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            room_id == self.state.group_chat_store.room_id(),
            "MCP request room_id does not match this TUI room"
        );
        Ok(())
    }
}
