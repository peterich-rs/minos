use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use jsonrpsee::core::async_trait;
use jsonrpsee::server::Server;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::MinosError;
use minos_protocol::{
    AppendConversationMessageParams, AppendConversationMessageResponse, ApprovalDecisionRequest,
    CloseSessionRequest, CreateConversationParams, CreateConversationResponse,
    CreateProjectRequest, CreateProjectResponse, HealthResponse, HostApplyLinkTokenParams,
    HostApplyLinkTokenResponse, HostPrepareLinkResponse, HostSignLinkProofParams,
    HostSignLinkProofResponse, InterruptSessionRequest, ListClisResponse,
    ListConversationAgentSessionsParams, ListConversationAgentSessionsResponse,
    ListConversationMessagesParams, ListConversationMessagesResponse, ListConversationsParams,
    ListConversationsResponse, ListProjectsResponse, LocalConversationEvent, LocalDaemonRpcServer,
    LocalIngestFrame, LocalManagerEvent, LocalSessionSnapshot, ReadArtifactRangeRequest,
    ReadArtifactRangeResponse, ReadSessionParams, ReadSessionRawHistoryResponse,
    RemoveConversationAgentParams, RemoveConversationAgentResponse,
    RespondOpencodePermissionRequest, RespondOpencodeQuestionRequest, SendUserMessageRequest,
    StartAgentInConversationRequest, StartAgentRequest, StartAgentResponse,
    ToggleConversationMessageReactionParams, ToggleConversationMessageReactionResponse,
};
use serde_json::json;
use tokio::sync::broadcast;
use tracing;

use crate::agent::{map_store_error, parse_agent_label, row_state_to_proto, AgentGlue};
use crate::relay_client::RelayClient;
use crate::rpc_server::rpc_err;

pub struct LocalRpcConfig {
    pub addr: SocketAddr,
    pub discovery_path: PathBuf,
}

/// Result of starting the local JSON-RPC WebSocket server.
pub struct LocalRpcServer {
    pub handle: jsonrpsee::server::ServerHandle,
    pub addr: SocketAddr,
    /// Canonical client URL (`ws://127.0.0.1:PORT`). Prefer this over re-reading
    /// the discovery file so in-process managed clients never race on stale paths.
    pub url: String,
}

pub struct LocalRpcImpl {
    pub started_at: Instant,
    pub runner: Arc<dyn CommandRunner>,
    pub agent: Arc<AgentGlue>,
    /// Present when the local RPC server is owned by a full daemon with relay.
    pub relay: Option<Arc<RelayClient>>,
    pub ingest_broadcaster: broadcast::Sender<LocalIngestFrame>,
    pub manager_event_broadcaster: broadcast::Sender<LocalManagerEvent>,
    pub conversation_event_broadcaster: broadcast::Sender<LocalConversationEvent>,
}

fn host_link_unavailable() -> jsonrpsee::types::ErrorObjectOwned {
    rpc_err(MinosError::BackendInternal {
        message: "host link RPC requires a running relay client".into(),
    })
}

#[async_trait]
impl LocalDaemonRpcServer for LocalRpcImpl {
    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse> {
        Ok(HealthResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_secs: self.started_at.elapsed().as_secs(),
        })
    }

    async fn host_prepare_link(&self) -> jsonrpsee::core::RpcResult<HostPrepareLinkResponse> {
        let Some(relay) = self.relay.as_ref() else {
            return Err(host_link_unavailable());
        };
        relay.prepare_link().await.map_err(rpc_err)
    }

    async fn host_sign_link_proof(
        &self,
        req: HostSignLinkProofParams,
    ) -> jsonrpsee::core::RpcResult<HostSignLinkProofResponse> {
        let Some(relay) = self.relay.as_ref() else {
            return Err(host_link_unavailable());
        };
        relay
            .sign_link_proof(&req.installation_id, &req.nonce)
            .map_err(rpc_err)
    }

    async fn host_apply_link_token(
        &self,
        req: HostApplyLinkTokenParams,
    ) -> jsonrpsee::core::RpcResult<HostApplyLinkTokenResponse> {
        let Some(relay) = self.relay.as_ref() else {
            return Err(host_link_unavailable());
        };
        relay
            .apply_link_token(&req.host_installation_token)
            .map_err(rpc_err)
    }

    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<ListClisResponse> {
        Ok(detect_all(self.runner.clone()).await)
    }

    async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        self.agent.start_agent(req).await.map_err(rpc_err)
    }

    async fn send_user_message(
        &self,
        req: SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.send_user_message(req).await.map_err(rpc_err)
    }

    async fn approval_decision(
        &self,
        req: ApprovalDecisionRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.resolve_approval(req).await.map_err(rpc_err)
    }

    async fn respond_opencode_permission(
        &self,
        req: RespondOpencodePermissionRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent
            .respond_opencode_permission(req)
            .await
            .map_err(rpc_err)
    }

    async fn respond_opencode_question(
        &self,
        req: RespondOpencodeQuestionRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent
            .respond_opencode_question(req)
            .await
            .map_err(rpc_err)
    }

    async fn interrupt_session(
        &self,
        req: InterruptSessionRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.interrupt_session(req).await.map_err(rpc_err)
    }

    async fn close_session(&self, req: CloseSessionRequest) -> jsonrpsee::core::RpcResult<()> {
        self.agent.close_session(req).await.map_err(rpc_err)
    }

    async fn delete_session(&self, req: CloseSessionRequest) -> jsonrpsee::core::RpcResult<()> {
        self.agent.delete_session(req).await.map_err(rpc_err)
    }

    async fn resume_session(
        &self,
        req: minos_protocol::ResumeSessionRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        self.agent
            .resume_session(&req.session_id, req.auto_continue)
            .await
            .map_err(rpc_err)
    }

    async fn list_local_sessions(&self) -> jsonrpsee::core::RpcResult<Vec<LocalSessionSnapshot>> {
        let rows = self
            .agent
            .store()
            .list_sessions(None, Some(500), None)
            .await
            .map_err(|e| rpc_err(map_store_error("list_local_sessions", e)))?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let session_id = row.session_id.clone();
            let workspace = row.workspace_root.clone();
            let state = row_state_to_proto(&row).map_err(rpc_err)?;
            result.push(LocalSessionSnapshot {
                session_id,
                agent: parse_agent_label(&row.agent).map_err(rpc_err)?,
                workspace,
                state,
                parent_session_id: row.parent_session_id.clone(),
            });
        }
        Ok(result)
    }

    async fn list_projects(&self) -> jsonrpsee::core::RpcResult<ListProjectsResponse> {
        self.agent.list_projects().await.map_err(rpc_err)
    }

    async fn create_project(
        &self,
        req: CreateProjectRequest,
    ) -> jsonrpsee::core::RpcResult<CreateProjectResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            project_name = %req.name,
            workspace_slug = %req.workspace_slug,
            workspace_path = ?req.workspace_path,
            "local RPC create_project",
        );
        self.agent.create_project(req).await.map_err(rpc_err)
    }

    async fn list_conversations(
        &self,
        req: ListConversationsParams,
    ) -> jsonrpsee::core::RpcResult<ListConversationsResponse> {
        self.agent.list_conversations(req).await.map_err(rpc_err)
    }

    async fn create_conversation(
        &self,
        req: CreateConversationParams,
    ) -> jsonrpsee::core::RpcResult<CreateConversationResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            project_id = %req.project_id,
            title = %req.title,
            "local RPC create_conversation",
        );
        self.agent.create_conversation(req).await.map_err(rpc_err)
    }

    async fn update_conversation(
        &self,
        req: minos_protocol::UpdateConversationParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::UpdateConversationResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            conversation_id = %req.conversation_id,
            has_title = req.title.is_some(),
            has_priority = req.priority.is_some(),
            has_progress = req.progress.is_some(),
            "local RPC update_conversation",
        );
        self.agent.update_conversation(req).await.map_err(rpc_err)
    }

    async fn add_conversation_agent(
        &self,
        req: minos_protocol::AddConversationAgentParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::AddConversationAgentResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            conversation_id = %req.conversation_id,
            agent = %req.agent,
            "local RPC add_conversation_agent",
        );
        self.agent
            .add_conversation_agent(req)
            .await
            .map_err(rpc_err)
    }

    async fn remove_conversation_agent(
        &self,
        req: RemoveConversationAgentParams,
    ) -> jsonrpsee::core::RpcResult<RemoveConversationAgentResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            conversation_id = %req.conversation_id,
            agent = %req.agent,
            "local RPC remove_conversation_agent",
        );
        self.agent
            .remove_conversation_agent(req)
            .await
            .map_err(rpc_err)
    }

    async fn list_conversation_messages(
        &self,
        req: ListConversationMessagesParams,
    ) -> jsonrpsee::core::RpcResult<ListConversationMessagesResponse> {
        self.agent
            .list_conversation_messages(req)
            .await
            .map_err(rpc_err)
    }

    async fn list_conversation_roster(
        &self,
        req: minos_protocol::ListConversationRosterParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::ListConversationRosterResponse> {
        self.agent
            .list_conversation_roster(req)
            .await
            .map_err(rpc_err)
    }

    async fn toggle_conversation_message_reaction(
        &self,
        req: ToggleConversationMessageReactionParams,
    ) -> jsonrpsee::core::RpcResult<ToggleConversationMessageReactionResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            message_id = %req.message_id,
            emoji = %req.emoji,
            "local RPC toggle_conversation_message_reaction",
        );
        self.agent
            .toggle_conversation_message_reaction(req)
            .await
            .map_err(rpc_err)
    }

    async fn list_conversation_agent_sessions(
        &self,
        req: ListConversationAgentSessionsParams,
    ) -> jsonrpsee::core::RpcResult<ListConversationAgentSessionsResponse> {
        self.agent
            .list_conversation_agent_sessions(req)
            .await
            .map_err(rpc_err)
    }

    async fn start_agent_in_conversation(
        &self,
        req: StartAgentInConversationRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            conversation_id = %req.conversation_id,
            agent = ?req.agent,
            workspace = %req.workspace,
            model = ?req.model,
            "local RPC start_agent_in_conversation",
        );
        self.agent
            .start_agent_in_conversation(req)
            .await
            .map_err(rpc_err)
    }

    async fn list_models(
        &self,
        req: minos_protocol::ListModelsRequest,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::ListModelsResponse> {
        Ok(crate::model_catalog::list_models_for_runtime(req.runtime).await)
    }

    async fn list_agent_profiles(
        &self,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::ListAgentProfilesResponse> {
        self.agent.list_agent_profiles().await.map_err(rpc_err)
    }

    async fn create_agent_profile(
        &self,
        req: minos_protocol::CreateAgentProfileRequest,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::AgentProfileSummary> {
        self.agent.create_agent_profile(req).await.map_err(rpc_err)
    }

    async fn update_agent_profile(
        &self,
        req: minos_protocol::UpdateAgentProfileRequest,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::AgentProfileSummary> {
        self.agent.update_agent_profile(req).await.map_err(rpc_err)
    }

    async fn delete_agent_profile(
        &self,
        req: minos_protocol::DeleteAgentProfileRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.delete_agent_profile(req).await.map_err(rpc_err)
    }

    async fn append_conversation_message(
        &self,
        req: AppendConversationMessageParams,
    ) -> jsonrpsee::core::RpcResult<AppendConversationMessageResponse> {
        self.agent
            .append_conversation_message(req)
            .await
            .map_err(rpc_err)
    }

    async fn git_get_status(
        &self,
        req: minos_protocol::GitStatusParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::GitStatusResponse> {
        self.agent.git_get_status(req).await.map_err(rpc_err)
    }

    async fn git_get_diff(
        &self,
        req: minos_protocol::GitDiffParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::GitDiffResponse> {
        self.agent.git_get_diff(req).await.map_err(rpc_err)
    }

    async fn git_create_worktree(
        &self,
        req: minos_protocol::GitCreateWorktreeParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::GitCreateWorktreeResponse> {
        self.agent.git_create_worktree(req).await.map_err(rpc_err)
    }

    async fn git_remove_worktree(
        &self,
        req: minos_protocol::GitRemoveWorktreeParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::GitRemoveWorktreeResponse> {
        self.agent.git_remove_worktree(req).await.map_err(rpc_err)
    }

    async fn git_ensure_identity(
        &self,
        req: minos_protocol::GitEnsureIdentityParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::GitEnsureIdentityResponse> {
        self.agent.git_ensure_identity(req).await.map_err(rpc_err)
    }

    async fn git_push_branch(
        &self,
        req: minos_protocol::GitPushBranchParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::GitPushBranchResponse> {
        self.agent.git_push_branch(req).await.map_err(rpc_err)
    }

    async fn git_open_pull_request(
        &self,
        req: minos_protocol::GitOpenPullRequestParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::GitOpenPullRequestResponse> {
        self.agent.git_open_pull_request(req).await.map_err(rpc_err)
    }

    async fn post_git_update(
        &self,
        req: minos_protocol::PostGitUpdateParams,
    ) -> jsonrpsee::core::RpcResult<minos_protocol::PostGitUpdateResponse> {
        self.agent.post_git_update(req).await.map_err(rpc_err)
    }

    async fn read_session_raw_history(
        &self,
        req: ReadSessionParams,
    ) -> jsonrpsee::core::RpcResult<ReadSessionRawHistoryResponse> {
        let (events, next_seq) = self
            .agent
            .read_session_raw_history(&req.session_id, req.from_seq, req.limit)
            .await
            .map_err(rpc_err)?;
        Ok(ReadSessionRawHistoryResponse { events, next_seq })
    }

    async fn read_artifact_range(
        &self,
        req: ReadArtifactRangeRequest,
    ) -> jsonrpsee::core::RpcResult<ReadArtifactRangeResponse> {
        let range = self
            .agent
            .store()
            .read_artifact_range(&req.session_id, &req.artifact_id, req.offset, req.limit)
            .await
            .map_err(|e| rpc_err(map_store_error("read_artifact_range", e)))?;
        Ok(ReadArtifactRangeResponse {
            bytes: range.bytes,
            offset: range.offset,
            total_size: range.total_size,
            eof: range.eof,
        })
    }

    async fn subscribe_ingest(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
    ) -> jsonrpsee::core::SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.ingest_broadcaster.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(frame) => {
                        let msg = match jsonrpsee::server::SubscriptionMessage::from_json(&frame) {
                            Ok(m) => m,
                            Err(_) => break,
                        };
                        if sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                    // Dropping frames silently leaves clients inconsistent; close so
                    // they resubscribe and re-fetch state.
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            target: "minos_daemon::local_rpc",
                            n,
                            "ingest subscription lagged; closing sink for resync"
                        );
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(())
    }

    async fn subscribe_manager_events(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
    ) -> jsonrpsee::core::SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.manager_event_broadcaster.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let msg = match jsonrpsee::server::SubscriptionMessage::from_json(&event) {
                            Ok(m) => m,
                            Err(_) => break,
                        };
                        if sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            target: "minos_daemon::local_rpc",
                            n,
                            "manager subscription lagged; closing sink for resync"
                        );
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(())
    }

    async fn subscribe_conversation_events(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
    ) -> jsonrpsee::core::SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.conversation_event_broadcaster.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let msg = match jsonrpsee::server::SubscriptionMessage::from_json(&event) {
                            Ok(m) => m,
                            Err(_) => break,
                        };
                        if sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            target: "minos_daemon::local_rpc",
                            n,
                            "conversation subscription lagged; closing sink for resync"
                        );
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(())
    }
}

pub async fn start_local_rpc_server(
    config: LocalRpcConfig,
    runner: Arc<dyn CommandRunner>,
    agent: Arc<AgentGlue>,
) -> Result<LocalRpcServer, MinosError> {
    start_local_rpc_server_with_relay(config, runner, agent, None).await
}

/// Like [`start_local_rpc_server`] but wires Host Link RPC to a live relay.
pub async fn start_local_rpc_server_with_relay(
    config: LocalRpcConfig,
    runner: Arc<dyn CommandRunner>,
    agent: Arc<AgentGlue>,
    relay: Option<Arc<RelayClient>>,
) -> Result<LocalRpcServer, MinosError> {
    let (ingest_tx, _) = broadcast::channel(256);
    let (mgr_evt_tx, _) = broadcast::channel(256);
    let (conversation_evt_tx, _) = broadcast::channel(256);

    let impl_ = LocalRpcImpl {
        started_at: Instant::now(),
        runner,
        agent: agent.clone(),
        relay,
        ingest_broadcaster: ingest_tx.clone(),
        manager_event_broadcaster: mgr_evt_tx.clone(),
        conversation_event_broadcaster: conversation_evt_tx.clone(),
    };

    // Local desktop/TUI only — keep connection/subscription caps so a misbehaving
    // client cannot open unbounded WS fan-out.
    let server = Server::builder()
        .max_connections(32)
        .max_subscriptions_per_connection(16)
        .build(config.addr)
        .await
        .map_err(|e| MinosError::CodexProtocolError {
            method: "local_rpc_server_bind".into(),
            message: e.to_string(),
        })?;

    let local_addr = server
        .local_addr()
        .map_err(|e| MinosError::CodexProtocolError {
            method: "local_rpc_local_addr".into(),
            message: e.to_string(),
        })?;

    let url = format!("ws://{local_addr}");
    let handle = server.start(impl_.into_rpc());

    write_discovery_file(&config.discovery_path, &url);

    spawn_ingest_bridge(agent.clone(), ingest_tx);

    spawn_manager_event_bridge(agent.clone(), mgr_evt_tx);
    spawn_conversation_event_bridge(agent.clone(), conversation_evt_tx);

    tracing::info!(
        target: "minos_daemon::local_rpc",
        addr = %local_addr,
        url = %url,
        "local RPC server started",
    );

    Ok(LocalRpcServer {
        handle,
        addr: local_addr,
        url,
    })
}

fn write_discovery_file(path: &PathBuf, url: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = json!({ "url": url });
    match serde_json::to_string_pretty(&payload) {
        Ok(content) => {
            // Atomic replace so readers never see a partial JSON blob.
            let tmp = path.with_extension("json.tmp");
            if let Err(e) = std::fs::write(&tmp, &content) {
                tracing::warn!(
                    target: "minos_daemon::local_rpc",
                    error = %e,
                    path = %tmp.display(),
                    "failed to write discovery temp file",
                );
                return;
            }
            if let Err(e) = std::fs::rename(&tmp, path) {
                // Fall back to direct write if rename fails (cross-device).
                if let Err(e2) = std::fs::write(path, content) {
                    tracing::warn!(
                        target: "minos_daemon::local_rpc",
                        error = %e,
                        fallback_error = %e2,
                        path = %path.display(),
                        "failed to publish discovery file",
                    );
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_daemon::local_rpc",
                error = %e,
                "failed to serialize discovery JSON",
            );
        }
    }
}

fn spawn_ingest_bridge(agent: Arc<AgentGlue>, tx: broadcast::Sender<LocalIngestFrame>) {
    let mut rx = agent.persisted_ingest_stream();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(frame) => {
                    let _ = tx.send(frame);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "minos_daemon::local_rpc",
                        n,
                        "ingest bridge lagged, dropping frames",
                    );
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn spawn_manager_event_bridge(agent: Arc<AgentGlue>, tx: broadcast::Sender<LocalManagerEvent>) {
    let mut rx = agent.local_manager_event_stream();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let _ = tx.send(event);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "minos_daemon::local_rpc",
                        n,
                        "manager event bridge lagged, dropping events",
                    );
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn spawn_conversation_event_bridge(
    agent: Arc<AgentGlue>,
    tx: broadcast::Sender<LocalConversationEvent>,
) {
    let mut rx = agent.local_conversation_event_stream();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let _ = tx.send(event);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "minos_daemon::local_rpc",
                        n,
                        "conversation event bridge lagged, dropping events",
                    );
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
