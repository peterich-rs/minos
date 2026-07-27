use super::{
    AgentBackend, BackendConnectionState, BackendSessionSnapshot, ConversationEntry,
    ConversationMessageEntry, ConversationMessageEvent, ProjectEntry, SessionSummaryEntry,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use minos_agent_runtime::{
    CloseReason as RuntimeCloseReason, ManagerEvent, PauseReason as RuntimePauseReason,
    SessionState as RuntimeSessionState, StartAgentOutcome,
};
use minos_domain::{AgentDescriptor, AgentName};
use minos_protocol::{
    AppendConversationMessageParams, ApprovalDecisionRequest, CloseReason as ProtoCloseReason,
    CloseSessionRequest, CreateConversationParams, InterruptSessionRequest, ListClisResponse,
    ListConversationAgentSessionsParams, ListConversationMessagesParams, ListConversationsParams,
    LocalConversationEvent, LocalIngestFrame, LocalManagerEvent, LocalSessionSnapshot,
    PauseReason as ProtoPauseReason, ReadSessionParams, ReadSessionRawHistoryResponse,
    RespondOpencodePermissionRequest, RespondOpencodeQuestionRequest, SendUserMessageRequest,
    SessionState as ProtoSessionState, StartAgentInConversationRequest, StartAgentRequest,
    StartAgentResponse,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::broadcast;
use tracing::warn;

pub struct DaemonBackend {
    client: Arc<WsClient>,
    endpoint: String,
    ingest_tx: broadcast::Sender<LocalIngestFrame>,
    manager_tx: broadcast::Sender<ManagerEvent>,
    conversation_message_tx: broadcast::Sender<ConversationMessageEvent>,
    state: Arc<StdMutex<BackendConnectionState>>,
}

impl DaemonBackend {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = WsClientBuilder::default()
            .build(url)
            .await
            .context(format!("failed to connect to daemon at {url}"))?;

        let (ingest_tx, _) = broadcast::channel(256);
        let (manager_tx, _) = broadcast::channel(64);
        let (conversation_message_tx, _) = broadcast::channel(64);

        let endpoint = url.to_owned();
        let state = Arc::new(StdMutex::new(BackendConnectionState::Connected {
            endpoint: endpoint.clone(),
        }));

        let client = Arc::new(client);

        Self::start_ingest_pump(
            client.clone(),
            ingest_tx.clone(),
            state.clone(),
            endpoint.clone(),
        );
        Self::start_manager_event_pump(
            client.clone(),
            manager_tx.clone(),
            state.clone(),
            endpoint.clone(),
        );
        Self::start_conversation_event_pump(
            client.clone(),
            conversation_message_tx.clone(),
            state.clone(),
            endpoint.clone(),
        );

        Ok(Self {
            client,
            endpoint,
            ingest_tx,
            manager_tx,
            conversation_message_tx,
            state,
        })
    }

    fn mark_disconnected(
        state: &Arc<StdMutex<BackendConnectionState>>,
        endpoint: &str,
        last_error: Option<String>,
    ) {
        if let Ok(mut snapshot) = state.lock() {
            *snapshot = BackendConnectionState::Disconnected {
                endpoint: endpoint.to_owned(),
                last_error,
            };
        }
    }

    fn start_ingest_pump(
        client: Arc<WsClient>,
        tx: broadcast::Sender<LocalIngestFrame>,
        state: Arc<StdMutex<BackendConnectionState>>,
        endpoint: String,
    ) {
        tokio::spawn(async move {
            let sub = match client
                .subscribe::<LocalIngestFrame, ArrayParams>(
                    "minos_local_subscribe_ingest",
                    ArrayParams::new(),
                    "minos_local_unsubscribe_ingest",
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!("ingest subscription failed: {e}");
                    Self::mark_disconnected(&state, &endpoint, Some(e.to_string()));
                    return;
                }
            };

            let mut stream = sub.into_stream();
            while let Some(result) = stream.next().await {
                match result {
                    Ok(frame) => {
                        let _ = tx.send(frame);
                    }
                    Err(e) => {
                        warn!("ingest subscription error: {e}");
                        Self::mark_disconnected(&state, &endpoint, Some(e.to_string()));
                        return;
                    }
                }
            }
            warn!("ingest subscription ended");
            Self::mark_disconnected(&state, &endpoint, Some("ingest subscription ended".into()));
        });
    }

    fn start_manager_event_pump(
        client: Arc<WsClient>,
        tx: broadcast::Sender<ManagerEvent>,
        state: Arc<StdMutex<BackendConnectionState>>,
        endpoint: String,
    ) {
        tokio::spawn(async move {
            let sub = match client
                .subscribe::<LocalManagerEvent, ArrayParams>(
                    "minos_local_subscribe_manager_events",
                    ArrayParams::new(),
                    "minos_local_unsubscribe_manager_events",
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!("manager event subscription failed: {e}");
                    Self::mark_disconnected(&state, &endpoint, Some(e.to_string()));
                    return;
                }
            };

            let mut stream = sub.into_stream();
            while let Some(result) = stream.next().await {
                match result {
                    Ok(event) => {
                        let rt_event = local_manager_to_runtime(event);
                        let _ = tx.send(rt_event);
                    }
                    Err(e) => {
                        warn!("manager event subscription error: {e}");
                        Self::mark_disconnected(&state, &endpoint, Some(e.to_string()));
                        return;
                    }
                }
            }
            warn!("manager event subscription ended");
            Self::mark_disconnected(
                &state,
                &endpoint,
                Some("manager event subscription ended".into()),
            );
        });
    }

    fn start_conversation_event_pump(
        client: Arc<WsClient>,
        tx: broadcast::Sender<ConversationMessageEvent>,
        state: Arc<StdMutex<BackendConnectionState>>,
        endpoint: String,
    ) {
        tokio::spawn(async move {
            let sub = match client
                .subscribe::<LocalConversationEvent, ArrayParams>(
                    "minos_local_subscribe_conversation_events",
                    ArrayParams::new(),
                    "minos_local_unsubscribe_conversation_events",
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!("conversation event subscription failed: {e}");
                    Self::mark_disconnected(&state, &endpoint, Some(e.to_string()));
                    return;
                }
            };

            let mut stream = sub.into_stream();
            while let Some(result) = stream.next().await {
                match result {
                    Ok(LocalConversationEvent::ConversationMessageAppended {
                        conversation_id,
                        message_seq,
                    }) => {
                        let _ = tx.send(ConversationMessageEvent {
                            conversation_id,
                            message_seq,
                        });
                    }
                    // Reactions are desktop-only for now; TUI has no reaction UI.
                    Ok(LocalConversationEvent::ConversationReactionToggled { .. }) => {}
                    Err(e) => {
                        warn!("conversation event subscription error: {e}");
                        Self::mark_disconnected(&state, &endpoint, Some(e.to_string()));
                        return;
                    }
                }
            }
            warn!("conversation event subscription ended");
            Self::mark_disconnected(
                &state,
                &endpoint,
                Some("conversation event subscription ended".into()),
            );
        });
    }
}

fn create_project_request(
    name: &str,
    workspace_path: &Path,
) -> minos_protocol::CreateProjectRequest {
    let workspace_str = workspace_path.to_string_lossy().into_owned();
    let slug = workspace_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_lowercase().replace(' ', "-"));
    minos_protocol::CreateProjectRequest {
        name: name.to_owned(),
        workspace_slug: slug,
        workspace_path: Some(workspace_str),
    }
}

fn list_conversations_request(project_id: &str) -> ListConversationsParams {
    ListConversationsParams {
        project_id: project_id.to_owned(),
        limit: Some(100),
        before_updated_at_ms: None,
    }
}

fn create_conversation_request(
    project_id: &str,
    title: &str,
    agents: Vec<String>,
) -> CreateConversationParams {
    CreateConversationParams {
        project_id: project_id.to_owned(),
        title: title.to_owned(),
        priority: None,
        agents,
        git_mode: None,
    }
}

fn list_conversation_messages_request(conversation_id: &str) -> ListConversationMessagesParams {
    ListConversationMessagesParams {
        conversation_id: conversation_id.to_owned(),
        before_seq: None,
        limit: Some(500),
    }
}

fn list_conversation_agent_sessions_request(
    conversation_id: &str,
) -> ListConversationAgentSessionsParams {
    ListConversationAgentSessionsParams {
        conversation_id: conversation_id.to_owned(),
    }
}

fn start_agent_in_conversation_request(
    conversation_id: &str,
    agent: AgentName,
    workspace: &Path,
    profile_id: Option<String>,
) -> StartAgentInConversationRequest {
    // When profile_id is set, leave model/effort/instructions None so the daemon
    // resolves them from the profile (explicit fields would override).
    StartAgentInConversationRequest {
        conversation_id: conversation_id.to_owned(),
        agent,
        workspace: workspace.to_string_lossy().into_owned(),
        profile_id,
        model: None,
        reasoning_effort: None,
        instructions: None,
    }
}

fn append_conversation_message_request(
    conversation_id: &str,
    message_id: Option<&str>,
    session_id: Option<&str>,
    sender_role: &str,
    agent: Option<AgentName>,
    body: &str,
) -> AppendConversationMessageParams {
    AppendConversationMessageParams {
        conversation_id: conversation_id.to_owned(),
        message_id: message_id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("tui-{}-{}", conversation_id, now_ms())),
        session_id: session_id.map(str::to_owned),
        sender_role: sender_role.to_owned(),
        agent,
        body: body.to_owned(),
        reply_to_message_id: None,
        delegation_id: None,
        mentions: Vec::new(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl AgentBackend for DaemonBackend {
    async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>> {
        let response: ListClisResponse = self
            .client
            .request("minos_local_list_clis", ArrayParams::new())
            .await
            .context("RPC minos_local_list_clis failed")?;
        Ok(response)
    }

    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome> {
        let request = StartAgentRequest {
            agent,
            workspace: workspace.to_string_lossy().into_owned(),
            mode: None,
            profile_id: None,
            model: None,
            reasoning_effort: None,
            instructions: None,
        };
        let response: StartAgentResponse = self
            .client
            .request("minos_local_start_agent", [request])
            .await
            .context("RPC minos_local_start_agent failed")?;
        Ok(StartAgentOutcome {
            session_id: response.session_id,
            cwd: PathBuf::from(response.cwd),
            provider_session_id: None,
        })
    }

    async fn send_message(&self, session_id: &str, text: &str) -> Result<()> {
        let request = SendUserMessageRequest {
            session_id: session_id.to_owned(),
            text: text.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_send_user_message", [request])
            .await
            .context("RPC minos_local_send_user_message failed")?;
        Ok(())
    }

    async fn send_approval_decision(
        &self,
        request_id: &str,
        session_id: &str,
        decision: Value,
    ) -> Result<()> {
        let request = ApprovalDecisionRequest {
            request_id: request_id.to_owned(),
            session_id: session_id.to_owned(),
            decision,
        };
        self.client
            .request::<(), _>("minos_local_approval_decision", [request])
            .await
            .context("RPC minos_local_approval_decision failed")?;
        Ok(())
    }

    async fn respond_opencode_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<()> {
        let request = RespondOpencodePermissionRequest {
            session_id: session_id.to_owned(),
            permission_id: permission_id.to_owned(),
            response: response.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_respond_opencode_permission", [request])
            .await
            .context("RPC minos_local_respond_opencode_permission failed")?;
        Ok(())
    }

    async fn respond_opencode_question(
        &self,
        session_id: &str,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> Result<()> {
        let request = RespondOpencodeQuestionRequest {
            session_id: session_id.to_owned(),
            question_id: question_id.to_owned(),
            answers,
        };
        self.client
            .request::<(), _>("minos_local_respond_opencode_question", [request])
            .await
            .context("RPC minos_local_respond_opencode_question failed")?;
        Ok(())
    }

    async fn interrupt_session(&self, session_id: &str) -> Result<()> {
        let request = InterruptSessionRequest {
            session_id: session_id.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_interrupt_session", [request])
            .await
            .context("RPC minos_local_interrupt_session failed")?;
        Ok(())
    }

    async fn close_session(&self, session_id: &str) -> Result<()> {
        let request = CloseSessionRequest {
            session_id: session_id.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_close_session", [request])
            .await
            .context("RPC minos_local_close_session failed")?;
        Ok(())
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        let request = CloseSessionRequest {
            session_id: session_id.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_delete_session", [request])
            .await
            .context("RPC minos_local_delete_session failed")?;
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<BackendSessionSnapshot>> {
        let snapshots: Vec<LocalSessionSnapshot> = self
            .client
            .request("minos_local_list_local_sessions", ArrayParams::new())
            .await
            .context("RPC minos_local_list_local_sessions failed")?;
        Ok(snapshots
            .into_iter()
            .map(|s| BackendSessionSnapshot {
                session_id: s.session_id,
                agent: Some(s.agent),
                workspace: PathBuf::from(s.workspace),
                state: proto_state_to_runtime(&s.state),
                parent_session_id: s.parent_session_id,
            })
            .collect())
    }

    async fn resume_session(
        &self,
        session_id: &str,
        auto_continue: bool,
    ) -> Result<StartAgentOutcome> {
        let request = minos_protocol::ResumeSessionRequest {
            session_id: session_id.to_owned(),
            auto_continue,
        };
        let response: StartAgentResponse = self
            .client
            .request("minos_local_resume_session", [request])
            .await
            .context("RPC minos_local_resume_session failed")?;
        Ok(StartAgentOutcome {
            session_id: response.session_id,
            cwd: PathBuf::from(response.cwd),
            provider_session_id: None,
        })
    }

    async fn list_projects(&self) -> Result<Vec<ProjectEntry>> {
        let response: minos_protocol::ListProjectsResponse = self
            .client
            .request("minos_local_list_projects", ArrayParams::new())
            .await
            .context("RPC minos_local_list_projects failed")?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(response
            .projects
            .iter()
            .map(|p| ProjectEntry::from_summary(p, &cwd))
            .collect())
    }

    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry> {
        let response: minos_protocol::CreateProjectResponse = self
            .client
            .request(
                "minos_local_create_project",
                [create_project_request(name, workspace_path)],
            )
            .await
            .context("RPC minos_local_create_project failed")?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(ProjectEntry::from_summary(&response.project, &cwd))
    }

    async fn list_conversations(&self, project_id: &str) -> Result<Vec<ConversationEntry>> {
        let response: minos_protocol::ListConversationsResponse = self
            .client
            .request(
                "minos_local_list_conversations",
                [list_conversations_request(project_id)],
            )
            .await
            .context("RPC minos_local_list_conversations failed")?;
        Ok(response
            .conversations
            .iter()
            .map(ConversationEntry::from_summary)
            .collect())
    }

    async fn create_conversation(
        &self,
        project_id: &str,
        title: &str,
        agents: &[AgentName],
    ) -> Result<ConversationEntry> {
        let agent_labels = agents
            .iter()
            .map(|a| a.bin_name().to_owned())
            .collect::<Vec<_>>();
        let response: minos_protocol::CreateConversationResponse = self
            .client
            .request(
                "minos_local_create_conversation",
                [create_conversation_request(project_id, title, agent_labels)],
            )
            .await
            .context("RPC minos_local_create_conversation failed")?;
        Ok(ConversationEntry::from_summary(&response.conversation))
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationMessageEntry>> {
        let response: minos_protocol::ListConversationMessagesResponse = self
            .client
            .request(
                "minos_local_list_conversation_messages",
                [list_conversation_messages_request(conversation_id)],
            )
            .await
            .context("RPC minos_local_list_conversation_messages failed")?;
        let mut messages = response
            .messages
            .iter()
            .map(ConversationMessageEntry::from_message)
            .collect::<Vec<_>>();
        messages.reverse();
        Ok(messages)
    }

    async fn list_conversation_agent_sessions(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<SessionSummaryEntry>> {
        let response: minos_protocol::ListConversationAgentSessionsResponse = self
            .client
            .request(
                "minos_local_list_conversation_agent_sessions",
                [list_conversation_agent_sessions_request(conversation_id)],
            )
            .await
            .context("RPC minos_local_list_conversation_agent_sessions failed")?;
        Ok(response
            .sessions
            .iter()
            .map(SessionSummaryEntry::from_summary)
            .collect())
    }

    async fn start_agent_in_conversation(
        &self,
        conversation_id: &str,
        agent: AgentName,
        workspace: PathBuf,
        profile_id: Option<String>,
    ) -> Result<StartAgentOutcome> {
        let response: StartAgentResponse = self
            .client
            .request(
                "minos_local_start_agent_in_conversation",
                [start_agent_in_conversation_request(
                    conversation_id,
                    agent,
                    &workspace,
                    profile_id,
                )],
            )
            .await
            .context("RPC minos_local_start_agent_in_conversation failed")?;
        Ok(StartAgentOutcome {
            session_id: response.session_id,
            cwd: PathBuf::from(response.cwd),
            provider_session_id: None,
        })
    }

    async fn list_agent_profiles(&self) -> Result<Vec<minos_protocol::AgentProfileSummary>> {
        let response: minos_protocol::ListAgentProfilesResponse = self
            .client
            .request("minos_local_list_agent_profiles", ArrayParams::new())
            .await
            .context("RPC minos_local_list_agent_profiles failed")?;
        Ok(response.profiles)
    }

    async fn append_conversation_message(
        &self,
        conversation_id: &str,
        message_id: Option<&str>,
        session_id: Option<&str>,
        sender_role: &str,
        agent: Option<AgentName>,
        body: &str,
    ) -> Result<()> {
        let request = append_conversation_message_request(
            conversation_id,
            message_id,
            session_id,
            sender_role,
            agent,
            body,
        );
        self.client
            .request::<minos_protocol::AppendConversationMessageResponse, _>(
                "minos_local_append_conversation_message",
                [request],
            )
            .await
            .context("RPC minos_local_append_conversation_message failed")?;
        Ok(())
    }

    async fn read_session_raw_history(
        &self,
        session_id: &str,
        from_seq: Option<u64>,
        limit: u32,
    ) -> Result<ReadSessionRawHistoryResponse> {
        let request = ReadSessionParams {
            session_id: session_id.to_owned(),
            from_seq,
            limit,
        };
        self.client
            .request("minos_local_read_session_raw_history", [request])
            .await
            .context("RPC minos_local_read_session_raw_history failed")
    }

    async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame> {
        self.ingest_tx.subscribe()
    }

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager_tx.subscribe()
    }

    async fn subscribe_conversation_message_events(
        &self,
    ) -> broadcast::Receiver<ConversationMessageEvent> {
        self.conversation_message_tx.subscribe()
    }

    fn connection_state(&self) -> BackendConnectionState {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(BackendConnectionState::Disconnected {
                endpoint: self.endpoint.clone(),
                last_error: Some("state lock poisoned".into()),
            })
    }
}

fn local_manager_to_runtime(event: LocalManagerEvent) -> ManagerEvent {
    match event {
        LocalManagerEvent::SessionAdded {
            session_id,
            workspace,
            agent,
            parent_session_id,
        } => ManagerEvent::SessionAdded {
            session_id,
            workspace: PathBuf::from(workspace),
            agent,
            parent_session_id,
        },
        LocalManagerEvent::SessionStateChanged {
            session_id,
            old,
            new,
            at_ms,
        } => ManagerEvent::SessionStateChanged {
            session_id,
            old: proto_state_to_runtime(&old),
            new: proto_state_to_runtime(&new),
            at_ms,
        },
        LocalManagerEvent::SessionClosed { session_id, reason } => ManagerEvent::SessionClosed {
            session_id,
            reason: proto_close_reason_to_runtime(&reason),
        },
        LocalManagerEvent::InstanceCrashed {
            workspace,
            affected_threads,
            reason,
        } => ManagerEvent::InstanceCrashed {
            workspace: PathBuf::from(workspace),
            affected_threads,
            reason: proto_pause_reason_to_runtime(&reason),
        },
    }
}

fn proto_state_to_runtime(state: &ProtoSessionState) -> RuntimeSessionState {
    match state {
        ProtoSessionState::Starting => RuntimeSessionState::Starting,
        ProtoSessionState::Idle => RuntimeSessionState::Idle,
        ProtoSessionState::Running { turn_started_at_ms } => RuntimeSessionState::Running {
            turn_started_at_ms: *turn_started_at_ms,
        },
        ProtoSessionState::Suspended { reason } => RuntimeSessionState::Suspended {
            reason: proto_pause_reason_to_runtime(reason),
        },
        ProtoSessionState::Resuming => RuntimeSessionState::Resuming,
        ProtoSessionState::Closed { reason } => RuntimeSessionState::Closed {
            reason: proto_close_reason_to_runtime(reason),
        },
    }
}

fn proto_pause_reason_to_runtime(reason: &ProtoPauseReason) -> RuntimePauseReason {
    match reason {
        ProtoPauseReason::UserInterrupt => RuntimePauseReason::UserInterrupt,
        ProtoPauseReason::CodexCrashed => RuntimePauseReason::CodexCrashed,
        ProtoPauseReason::DaemonRestart => RuntimePauseReason::DaemonRestart,
        ProtoPauseReason::InstanceReaped => RuntimePauseReason::InstanceReaped,
    }
}

fn proto_close_reason_to_runtime(reason: &ProtoCloseReason) -> RuntimeCloseReason {
    match reason {
        ProtoCloseReason::UserClose => RuntimeCloseReason::UserClose,
        ProtoCloseReason::TerminalError => RuntimeCloseReason::TerminalError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_rpc_requests_have_expected_values() {
        assert_eq!(
            serde_json::to_value(create_project_request("Fire", Path::new("/tmp/fire"))).unwrap(),
            serde_json::json!({
                "name": "Fire",
                "workspace_slug": "fire",
                "workspace_path": "/tmp/fire"
            })
        );
        assert_eq!(
            serde_json::to_value(list_conversations_request("project-1")).unwrap(),
            serde_json::json!({
                "project_id": "project-1",
                "limit": 100
            })
        );
        assert_eq!(
            serde_json::to_value(start_agent_in_conversation_request(
                "conversation-1",
                AgentName::Codex,
                Path::new("/tmp/fire"),
                None,
            ))
            .unwrap(),
            serde_json::json!({
                "conversation_id": "conversation-1",
                "agent": "codex",
                "workspace": "/tmp/fire"
            })
        );
        assert_eq!(
            serde_json::to_value(start_agent_in_conversation_request(
                "conversation-1",
                AgentName::Grok,
                Path::new("/tmp/fire"),
                Some("profile-research".into()),
            ))
            .unwrap(),
            serde_json::json!({
                "conversation_id": "conversation-1",
                "agent": "grok",
                "workspace": "/tmp/fire",
                "profile_id": "profile-research"
            })
        );
    }
}
