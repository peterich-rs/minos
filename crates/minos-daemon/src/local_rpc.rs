use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use jsonrpsee::core::async_trait;
use jsonrpsee::server::Server;
use minos_agent_runtime::ManagerEvent;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::MinosError;
use minos_protocol::{
    AppendConversationMessageParams, AppendConversationMessageResponse, ApprovalDecisionRequest,
    CloseReason, CloseThreadRequest, CreateConversationParams, CreateConversationResponse,
    CreateProjectRequest, CreateProjectResponse, GetThreadParams, HealthResponse,
    InterruptThreadRequest, ListClisResponse, ListConversationAgentSessionsParams,
    ListConversationAgentSessionsResponse, ListConversationMessagesParams,
    ListConversationMessagesResponse, ListConversationsParams, ListConversationsResponse,
    ListProjectThreadsParams, ListProjectThreadsResponse, ListProjectsResponse,
    LocalDaemonRpcServer, LocalIngestFrame, LocalManagerEvent, LocalThreadSnapshot,
    ReadArtifactRangeRequest, ReadArtifactRangeResponse, ReadGroupChatParams,
    ReadGroupChatResponse, ReadThreadParams, ReadThreadRawHistoryResponse,
    RespondOpencodePermissionRequest, RespondOpencodeQuestionRequest, SendUserMessageRequest,
    StartAgentInConversationRequest, StartAgentInProjectRequest, StartAgentRequest,
    StartAgentResponse, ThreadState,
};
use serde_json::json;
use tokio::sync::broadcast;
use tracing;

use crate::agent::{map_store_error, parse_agent_label, row_state_to_proto, AgentGlue};
use crate::rpc_server::rpc_err;

pub struct LocalRpcConfig {
    pub addr: SocketAddr,
    pub discovery_path: PathBuf,
    pub group_chat_db_path: PathBuf,
}

pub struct LocalRpcImpl {
    pub started_at: Instant,
    pub runner: Arc<dyn CommandRunner>,
    pub agent: Arc<AgentGlue>,
    pub ingest_broadcaster: broadcast::Sender<LocalIngestFrame>,
    pub manager_event_broadcaster: broadcast::Sender<LocalManagerEvent>,
    pub group_chat_db_path: PathBuf,
}

#[async_trait]
impl LocalDaemonRpcServer for LocalRpcImpl {
    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse> {
        Ok(HealthResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_secs: self.started_at.elapsed().as_secs(),
        })
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

    async fn interrupt_thread(
        &self,
        req: InterruptThreadRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.interrupt_thread(req).await.map_err(rpc_err)
    }

    async fn close_thread(&self, req: CloseThreadRequest) -> jsonrpsee::core::RpcResult<()> {
        self.agent.close_thread(req).await.map_err(rpc_err)
    }

    async fn delete_thread(&self, req: CloseThreadRequest) -> jsonrpsee::core::RpcResult<()> {
        self.agent.delete_thread(req).await.map_err(rpc_err)
    }

    async fn resume_thread(
        &self,
        req: GetThreadParams,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        self.agent
            .resume_thread(&req.thread_id)
            .await
            .map_err(rpc_err)
    }

    async fn list_local_threads(&self) -> jsonrpsee::core::RpcResult<Vec<LocalThreadSnapshot>> {
        let rows = self
            .agent
            .store()
            .list_threads(None, Some(500), None)
            .await
            .map_err(|e| rpc_err(map_store_error("list_local_threads", e)))?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let thread_id = row.thread_id.clone();
            let workspace = row.workspace_root.clone();
            let state = row_state_to_proto(&row).map_err(rpc_err)?;
            result.push(LocalThreadSnapshot {
                thread_id,
                agent: parse_agent_label(&row.agent).map_err(rpc_err)?,
                workspace,
                state,
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

    async fn list_conversation_messages(
        &self,
        req: ListConversationMessagesParams,
    ) -> jsonrpsee::core::RpcResult<ListConversationMessagesResponse> {
        self.agent
            .list_conversation_messages(req)
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
            "local RPC start_agent_in_conversation",
        );
        self.agent
            .start_agent_in_conversation(req)
            .await
            .map_err(rpc_err)
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

    async fn list_project_threads(
        &self,
        req: ListProjectThreadsParams,
    ) -> jsonrpsee::core::RpcResult<ListProjectThreadsResponse> {
        self.agent.list_project_threads(req).await.map_err(rpc_err)
    }

    async fn start_agent_in_project(
        &self,
        req: StartAgentInProjectRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        tracing::info!(
            target: "minos_daemon::local_rpc",
            project_id = %req.project_id,
            agent = ?req.agent,
            workspace = %req.workspace,
            "local RPC start_agent_in_project",
        );
        self.agent
            .start_agent_in_project(
                StartAgentRequest {
                    agent: req.agent,
                    workspace: req.workspace,
                    mode: None,
                },
                &req.project_id,
                req.workspace_slug.as_deref(),
            )
            .await
            .map_err(rpc_err)
    }

    async fn read_thread_raw_history(
        &self,
        req: ReadThreadParams,
    ) -> jsonrpsee::core::RpcResult<ReadThreadRawHistoryResponse> {
        let (events, next_seq) = self
            .agent
            .read_thread_raw_history(&req.thread_id, req.from_seq, req.limit)
            .await
            .map_err(rpc_err)?;
        Ok(ReadThreadRawHistoryResponse { events, next_seq })
    }

    async fn read_group_chat(
        &self,
        req: ReadGroupChatParams,
    ) -> jsonrpsee::core::RpcResult<ReadGroupChatResponse> {
        let page = read_group_chat_messages(&self.group_chat_db_path, &req)
            .await
            .map_err(rpc_err)?;
        Ok(ReadGroupChatResponse {
            log_path: self.group_chat_db_path.display().to_string(),
            messages: page.messages.into_iter().map(Into::into).collect(),
            next_before_seq: page.next_before_seq,
            has_more: page.has_more,
        })
    }

    async fn read_artifact_range(
        &self,
        req: ReadArtifactRangeRequest,
    ) -> jsonrpsee::core::RpcResult<ReadArtifactRangeResponse> {
        let range = self
            .agent
            .store()
            .read_artifact_range(&req.thread_id, &req.artifact_id, req.offset, req.limit)
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
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
) -> Result<jsonrpsee::server::ServerHandle, MinosError> {
    let (ingest_tx, _) = broadcast::channel(256);
    let (mgr_evt_tx, _) = broadcast::channel(256);

    let impl_ = LocalRpcImpl {
        started_at: Instant::now(),
        runner,
        agent: agent.clone(),
        ingest_broadcaster: ingest_tx.clone(),
        manager_event_broadcaster: mgr_evt_tx.clone(),
        group_chat_db_path: config.group_chat_db_path.clone(),
    };

    let server =
        Server::builder()
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

    let handle = server.start(impl_.into_rpc());

    write_discovery_file(&config.discovery_path, local_addr);

    spawn_ingest_bridge(agent.clone(), ingest_tx);

    spawn_manager_event_bridge(agent.clone(), mgr_evt_tx);

    tracing::info!(
        target: "minos_daemon::local_rpc",
        addr = %local_addr,
        "local RPC server started",
    );

    Ok(handle)
}

async fn read_group_chat_messages(
    db_path: &std::path::Path,
    req: &ReadGroupChatParams,
) -> Result<minos_chat_store::ChatMessagePage, MinosError> {
    let store = minos_chat_store::ChatStore::open(db_path)
        .await
        .map_err(|error| MinosError::StoreIo {
            path: db_path.display().to_string(),
            message: error.to_string(),
        })?;
    let room_id = req
        .room_id
        .clone()
        .unwrap_or_else(|| "room-main".to_owned());
    if let Some(after_seq) = req.after_seq {
        let messages = store
            .list_messages_after_asc(&room_id, after_seq, req.limit)
            .await
            .map_err(|error| MinosError::StoreIo {
                path: db_path.display().to_string(),
                message: error.to_string(),
            })?;
        return Ok(minos_chat_store::ChatMessagePage {
            room_id,
            messages,
            next_before_seq: None,
            has_more: false,
        });
    }

    store
        .list_messages_desc(&room_id, req.before_seq, req.limit)
        .await
        .map_err(|error| MinosError::StoreIo {
            path: db_path.display().to_string(),
            message: error.to_string(),
        })
}

fn write_discovery_file(path: &PathBuf, addr: SocketAddr) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = json!({
        "url": format!("ws://{addr}")
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(content) => {
            if let Err(e) = std::fs::write(path, content) {
                tracing::warn!(
                    target: "minos_daemon::local_rpc",
                    error = %e,
                    path = %path.display(),
                    "failed to write discovery file",
                );
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
    let mut rx = agent.manager.manager_event_stream();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let local_event = match convert_manager_event(event) {
                        Some(e) => e,
                        None => continue,
                    };
                    let _ = tx.send(local_event);
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

fn convert_manager_event(event: ManagerEvent) -> Option<LocalManagerEvent> {
    match event {
        ManagerEvent::ThreadAdded {
            thread_id,
            workspace,
            agent,
        } => Some(LocalManagerEvent::ThreadAdded {
            thread_id,
            workspace: workspace.display().to_string(),
            agent,
        }),
        ManagerEvent::ThreadStateChanged {
            thread_id,
            old,
            new,
            at_ms,
        } => Some(LocalManagerEvent::ThreadStateChanged {
            thread_id,
            old: runtime_state_to_proto(&old),
            new: runtime_state_to_proto(&new),
            at_ms,
        }),
        ManagerEvent::ThreadClosed { thread_id, reason } => Some(LocalManagerEvent::ThreadClosed {
            thread_id,
            reason: runtime_close_reason_to_proto(&reason),
        }),
        ManagerEvent::InstanceCrashed {
            workspace,
            affected_threads,
            reason,
        } => Some(LocalManagerEvent::InstanceCrashed {
            workspace: workspace.display().to_string(),
            affected_threads,
            reason: runtime_pause_reason_to_proto(&reason),
        }),
    }
}

fn runtime_state_to_proto(state: &minos_agent_runtime::ThreadState) -> ThreadState {
    use minos_agent_runtime::ThreadState as RtState;
    match state {
        RtState::Starting => ThreadState::Starting,
        RtState::Idle => ThreadState::Idle,
        RtState::Running { turn_started_at_ms } => ThreadState::Running {
            turn_started_at_ms: *turn_started_at_ms,
        },
        RtState::Suspended { reason } => ThreadState::Suspended {
            reason: runtime_pause_reason_to_proto(reason),
        },
        RtState::Resuming => ThreadState::Resuming,
        RtState::Closed { reason } => ThreadState::Closed {
            reason: runtime_close_reason_to_proto(reason),
        },
    }
}

fn runtime_pause_reason_to_proto(
    reason: &minos_agent_runtime::PauseReason,
) -> minos_protocol::PauseReason {
    use minos_agent_runtime::PauseReason as Rt;
    match reason {
        Rt::UserInterrupt => minos_protocol::PauseReason::UserInterrupt,
        Rt::CodexCrashed => minos_protocol::PauseReason::CodexCrashed,
        Rt::DaemonRestart => minos_protocol::PauseReason::DaemonRestart,
        Rt::InstanceReaped => minos_protocol::PauseReason::InstanceReaped,
    }
}

fn runtime_close_reason_to_proto(reason: &minos_agent_runtime::CloseReason) -> CloseReason {
    use minos_agent_runtime::CloseReason as Rt;
    match reason {
        Rt::UserClose => CloseReason::UserClose,
        Rt::TerminalError => CloseReason::TerminalError,
    }
}
