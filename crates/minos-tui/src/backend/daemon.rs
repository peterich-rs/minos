use super::{AgentBackend, BackendConnectionState, BackendThreadSnapshot};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
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
    SendUserMessageRequest, StartAgentRequest, StartAgentResponse, ThreadState as ProtoThreadState,
};
use serde_json::Value;
use std::path::PathBuf;
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

use jsonrpsee::core::params::ArrayParams;

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
