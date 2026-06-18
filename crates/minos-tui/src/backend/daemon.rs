use super::{AgentBackend, BackendConnectionState, BackendThreadSnapshot, ProjectEntry, ThreadSummaryEntry};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::params::{ArrayParams, ObjectParams};
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use minos_agent_runtime::{
    CloseReason as RuntimeCloseReason, ManagerEvent, PauseReason as RuntimePauseReason,
    StartAgentOutcome, ThreadState as RuntimeThreadState,
};
use minos_domain::{AgentDescriptor, AgentName};
use minos_protocol::{
    ApprovalDecisionRequest, CloseReason as ProtoCloseReason, CloseThreadRequest, GetThreadParams,
    InterruptThreadRequest, ListClisResponse, LocalGroupChatMessage, LocalIngestFrame,
    LocalManagerEvent, LocalThreadSnapshot, PauseReason as ProtoPauseReason, ReadGroupChatParams,
    ReadThreadParams, ReadThreadRawHistoryResponse, RespondOpencodePermissionRequest,
    RespondOpencodeQuestionRequest, SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
    ThreadState as ProtoThreadState,
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

        Ok(Self {
            client,
            endpoint,
            ingest_tx,
            manager_tx,
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
}

fn create_project_params(name: &str, workspace_path: &Path) -> Result<ObjectParams> {
    let workspace_str = workspace_path.to_string_lossy().into_owned();
    let slug = workspace_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_lowercase().replace(' ', "-"));
    let mut params = ObjectParams::new();
    params.insert("name", name)?;
    params.insert("workspace_slug", slug)?;
    params.insert("workspace_path", workspace_str)?;
    Ok(params)
}

fn list_project_threads_params(project_id: &str) -> Result<ObjectParams> {
    let mut params = ObjectParams::new();
    params.insert("project_id", project_id)?;
    params.insert("limit", 100_u32)?;
    params.insert("before_ts_ms", None::<i64>)?;
    Ok(params)
}

fn start_agent_in_project_params(
    project_id: &str,
    agent: AgentName,
    workspace: &Path,
) -> Result<ObjectParams> {
    let mut params = ObjectParams::new();
    params.insert("agent", agent)?;
    params.insert("workspace", workspace.to_string_lossy().into_owned())?;
    params.insert("project_id", project_id)?;
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonrpsee::core::traits::ToRpcParams;

    fn params_json(params: ObjectParams) -> serde_json::Value {
        let raw = params
            .to_rpc_params()
            .expect("params serialize")
            .expect("params are present");
        serde_json::from_str(raw.get()).expect("params are json")
    }

    #[test]
    fn project_rpc_params_are_objects() {
        assert_eq!(
            params_json(create_project_params("Fire", Path::new("/tmp/fire")).unwrap()),
            serde_json::json!({
                "name": "Fire",
                "workspace_slug": "fire",
                "workspace_path": "/tmp/fire"
            })
        );
        assert_eq!(
            params_json(list_project_threads_params("project-1").unwrap()),
            serde_json::json!({
                "project_id": "project-1",
                "limit": 100,
                "before_ts_ms": null
            })
        );
        assert_eq!(
            params_json(
                start_agent_in_project_params("project-1", AgentName::Codex, Path::new("/tmp/fire"))
                    .unwrap()
            ),
            serde_json::json!({
                "agent": "codex",
                "workspace": "/tmp/fire",
                "project_id": "project-1"
            })
        );
    }
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
        };
        let response: StartAgentResponse = self
            .client
            .request("minos_local_start_agent", [request])
            .await
            .context("RPC minos_local_start_agent failed")?;
        Ok(StartAgentOutcome {
            thread_id: response.session_id,
            cwd: PathBuf::from(response.cwd),
            provider_session_id: None,
        })
    }

    async fn send_message(&self, thread_id: &str, text: &str) -> Result<()> {
        let request = SendUserMessageRequest {
            session_id: thread_id.to_owned(),
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
        thread_id: &str,
        decision: Value,
    ) -> Result<()> {
        let request = ApprovalDecisionRequest {
            request_id: request_id.to_owned(),
            thread_id: thread_id.to_owned(),
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
        thread_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<()> {
        let request = RespondOpencodePermissionRequest {
            thread_id: thread_id.to_owned(),
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
        thread_id: &str,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> Result<()> {
        let request = RespondOpencodeQuestionRequest {
            thread_id: thread_id.to_owned(),
            question_id: question_id.to_owned(),
            answers,
        };
        self.client
            .request::<(), _>("minos_local_respond_opencode_question", [request])
            .await
            .context("RPC minos_local_respond_opencode_question failed")?;
        Ok(())
    }

    async fn interrupt_thread(&self, thread_id: &str) -> Result<()> {
        let request = InterruptThreadRequest {
            thread_id: thread_id.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_interrupt_thread", [request])
            .await
            .context("RPC minos_local_interrupt_thread failed")?;
        Ok(())
    }

    async fn close_thread(&self, thread_id: &str) -> Result<()> {
        let request = CloseThreadRequest {
            thread_id: thread_id.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_close_thread", [request])
            .await
            .context("RPC minos_local_close_thread failed")?;
        Ok(())
    }

    async fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let request = CloseThreadRequest {
            thread_id: thread_id.to_owned(),
        };
        self.client
            .request::<(), _>("minos_local_delete_thread", [request])
            .await
            .context("RPC minos_local_delete_thread failed")?;
        Ok(())
    }

    async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>> {
        let snapshots: Vec<LocalThreadSnapshot> = self
            .client
            .request("minos_local_list_local_threads", ArrayParams::new())
            .await
            .context("RPC minos_local_list_local_threads failed")?;
        Ok(snapshots
            .into_iter()
            .map(|s| BackendThreadSnapshot {
                thread_id: s.thread_id,
                agent: Some(s.agent),
                workspace: PathBuf::from(s.workspace),
                state: proto_state_to_runtime(&s.state),
            })
            .collect())
    }

    async fn resume_thread(&self, thread_id: &str) -> Result<StartAgentOutcome> {
        let request = GetThreadParams {
            thread_id: thread_id.to_owned(),
        };
        let response: StartAgentResponse = self
            .client
            .request("minos_local_resume_thread", [request])
            .await
            .context("RPC minos_local_resume_thread failed")?;
        Ok(StartAgentOutcome {
            thread_id: response.session_id,
            cwd: PathBuf::from(response.cwd),
            provider_session_id: None,
        })
    }

    async fn list_projects(&self) -> Result<Vec<ProjectEntry>> {
        let response: minos_protocol::ListProjectsResponse = self
            .client
            .request("minos_list_projects", ArrayParams::new())
            .await
            .context("RPC minos_list_projects failed")?;
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
                "minos_create_project",
                create_project_params(name, workspace_path)?,
            )
            .await
            .context("RPC minos_create_project failed")?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(ProjectEntry::from_summary(&response.project, &cwd))
    }

    async fn list_project_threads(&self, project_id: &str) -> Result<Vec<ThreadSummaryEntry>> {
        let response: minos_protocol::ListProjectThreadsResponse = self
            .client
            .request("minos_list_project_threads", list_project_threads_params(project_id)?)
            .await
            .context("RPC minos_list_project_threads failed")?;
        Ok(response
            .threads
            .iter()
            .map(ThreadSummaryEntry::from_summary)
            .collect())
    }

    async fn start_agent_in_project(
        &self,
        project_id: &str,
        agent: AgentName,
        workspace: PathBuf,
    ) -> Result<StartAgentOutcome> {
        let response: StartAgentResponse = self
            .client
            .request(
                "minos_start_agent_in_project",
                start_agent_in_project_params(project_id, agent, &workspace)?,
            )
            .await
            .context("RPC minos_start_agent_in_project failed")?;
        Ok(StartAgentOutcome {
            thread_id: response.session_id,
            cwd: PathBuf::from(response.cwd),
            provider_session_id: None,
        })
    }

    async fn read_thread_raw_history(
        &self,
        thread_id: &str,
        from_seq: Option<u64>,
        limit: u32,
    ) -> Result<ReadThreadRawHistoryResponse> {
        let request = ReadThreadParams {
            thread_id: thread_id.to_owned(),
            from_seq,
            limit,
        };
        self.client
            .request("minos_local_read_thread_raw_history", [request])
            .await
            .context("RPC minos_local_read_thread_raw_history failed")
    }

    async fn read_group_chat(
        &self,
        room_id: &str,
        after_seq: Option<u64>,
        before_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<LocalGroupChatMessage>> {
        let request = ReadGroupChatParams {
            room_id: Some(room_id.to_owned()),
            after_seq,
            before_seq,
            limit: Some(limit),
        };
        let mut response: minos_protocol::ReadGroupChatResponse = self
            .client
            .request("minos_local_read_group_chat", [request])
            .await
            .context("RPC minos_local_read_group_chat failed")?;
        response.messages.sort_by_key(|message| message.seq);
        Ok(response.messages)
    }

    async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame> {
        self.ingest_tx.subscribe()
    }

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager_tx.subscribe()
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
        LocalManagerEvent::ThreadAdded {
            thread_id,
            workspace,
            agent,
        } => ManagerEvent::ThreadAdded {
            thread_id,
            workspace: PathBuf::from(workspace),
            agent,
        },
        LocalManagerEvent::ThreadStateChanged {
            thread_id,
            old,
            new,
            at_ms,
        } => ManagerEvent::ThreadStateChanged {
            thread_id,
            old: proto_state_to_runtime(&old),
            new: proto_state_to_runtime(&new),
            at_ms,
        },
        LocalManagerEvent::ThreadClosed { thread_id, reason } => ManagerEvent::ThreadClosed {
            thread_id,
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

fn proto_state_to_runtime(state: &ProtoThreadState) -> RuntimeThreadState {
    match state {
        ProtoThreadState::Starting => RuntimeThreadState::Starting,
        ProtoThreadState::Idle => RuntimeThreadState::Idle,
        ProtoThreadState::Running { turn_started_at_ms } => RuntimeThreadState::Running {
            turn_started_at_ms: *turn_started_at_ms,
        },
        ProtoThreadState::Suspended { reason } => RuntimeThreadState::Suspended {
            reason: proto_pause_reason_to_runtime(reason),
        },
        ProtoThreadState::Resuming => RuntimeThreadState::Resuming,
        ProtoThreadState::Closed { reason } => RuntimeThreadState::Closed {
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
