use super::{AgentBackend, BackendConnectionState, BackendThreadSnapshot};
use anyhow::Result;
use async_trait::async_trait;
use minos_agent_runtime::{AgentManager, InstanceCaps, ManagerEvent, StartAgentOutcome};
use minos_cli_detect::{capture_user_shell_env, detect_all, RealCommandRunner};
use minos_domain::AgentName;
use minos_protocol::local_rpc::ReadThreadRawHistoryResponse;
use minos_protocol::LocalGroupChatMessage;
use minos_protocol::LocalIngestFrame;
use serde_json::Value;
use std::collections::HashMap;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::broadcast;

use crate::event::{AppEvent, McpToolEvent};
use crate::translation::AgentTranslationState;

pub struct EmbeddedBackend {
    manager: Arc<AgentManager>,
    mcp_socket_path: Option<PathBuf>,
}

impl EmbeddedBackend {
    pub async fn new(
        workspace_root: PathBuf,
        max_instances: usize,
        idle_timeout: std::time::Duration,
        mcp_permissions: minos_chat_store::mcp_server::McpToolPermissions,
    ) -> Result<Self> {
        let shell_env = capture_user_shell_env().await;
        let mut config = minos_agent_runtime::AgentRuntimeConfig::new(workspace_root);
        let db_path = minos_chat_store::default_db_path()?;
        let minos_home = db_path.parent().expect("db_path parent").to_path_buf();
        let socket_path = {
            let id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            minos_home.join("run").join(format!("mcp-{id}.sock"))
        };
        let mcp_result = config.enable_default_mcp_with_socket_path(socket_path);
        if let Some(mcp) = config.mcp.as_mut() {
            mcp.permissions = mcp_permissions;
        }
        if let Err(error) = mcp_result {
            tracing::warn!(
                target: "minos_tui::backend::embedded",
                error = %error,
                "failed to enable default MCP"
            );
        }
        let mcp_socket_path = config.mcp.as_ref().map(|mcp| mcp.socket_path.clone());
        config.subprocess_env = Arc::new(shell_env);
        let caps = InstanceCaps {
            max_instances,
            idle_timeout,
        };
        let manager = AgentManager::new(config, caps);
        Ok(Self {
            manager: Arc::new(manager),
            mcp_socket_path,
        })
    }
}

#[async_trait]
impl AgentBackend for EmbeddedBackend {
    async fn detect_clis(&self) -> Result<Vec<minos_domain::AgentDescriptor>> {
        let env = capture_user_shell_env().await;
        let runner = Arc::new(RealCommandRunner::new(Arc::new(env)));
        Ok(detect_all(runner).await)
    }

    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome> {
        self.manager
            .start_agent(agent, workspace)
            .await
            .map_err(Into::into)
    }

    async fn send_message(&self, thread_id: &str, text: &str) -> Result<()> {
        self.manager
            .send_user_message(thread_id, text.to_owned())
            .await
            .map_err(Into::into)
    }

    async fn send_approval_decision(
        &self,
        request_id: &str,
        thread_id: &str,
        decision: Value,
    ) -> Result<()> {
        self.manager
            .resolve_approval(request_id, thread_id, decision)
            .await
            .map_err(Into::into)
    }

    async fn respond_opencode_permission(
        &self,
        thread_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<()> {
        self.manager
            .respond_opencode_permission(thread_id, permission_id, response)
            .await
            .map_err(Into::into)
    }

    async fn interrupt_thread(&self, thread_id: &str) -> Result<()> {
        self.manager
            .interrupt_thread(thread_id)
            .await
            .map_err(Into::into)
    }

    async fn close_thread(&self, thread_id: &str) -> Result<()> {
        self.manager
            .close_thread(thread_id)
            .await
            .map_err(Into::into)
    }

    async fn delete_thread(&self, thread_id: &str) -> Result<()> {
        self.close_thread(thread_id).await
    }

    async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>> {
        Ok(self
            .manager
            .list_threads()
            .await
            .into_iter()
            .map(|thread| BackendThreadSnapshot {
                thread_id: thread.thread_id,
                agent: None,
                workspace: thread.workspace,
                state: thread.state,
            })
            .collect())
    }

    async fn resume_thread(&self, _thread_id: &str) -> Result<StartAgentOutcome> {
        Err(anyhow::anyhow!(
            "embedded mode does not support thread resumption"
        ))
    }

    async fn read_thread_raw_history(
        &self,
        _thread_id: &str,
        _from_seq: Option<u64>,
        _limit: u32,
    ) -> Result<ReadThreadRawHistoryResponse> {
        Ok(ReadThreadRawHistoryResponse {
            events: Vec::new(),
            next_seq: None,
        })
    }

    async fn read_group_chat(
        &self,
        _room_id: &str,
        _after_seq: Option<u64>,
        _before_seq: Option<u64>,
        _limit: u32,
    ) -> Result<Vec<LocalGroupChatMessage>> {
        Err(anyhow::anyhow!(
            "embedded mode does not expose group chat RPC"
        ))
    }

    async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame> {
        let mut raw_rx = self.manager.ingest_stream();
        let (tx, rx) = broadcast::channel(256);
        tokio::spawn(async move {
            let mut translators: HashMap<String, AgentTranslationState> = HashMap::new();
            loop {
                match raw_rx.recv().await {
                    Ok(ingest) => {
                        let Some(payload) = ingest.json_value() else {
                            continue;
                        };
                        let translator = translators
                            .entry(ingest.thread_id.clone())
                            .or_insert_with(|| {
                                AgentTranslationState::new(ingest.agent, ingest.thread_id.clone())
                            });
                        let ui_events = translator.translate(&payload);
                        let _ = tx.send(LocalIngestFrame {
                            thread_id: ingest.thread_id,
                            seq: 0,
                            agent: ingest.agent,
                            ui_events,
                            ts_ms: ingest.ts_ms,
                        });
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        rx
    }

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager.manager_event_stream()
    }

    fn start_mcp_socket_handler(
        &self,
        event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    ) -> Result<()> {
        let Some(socket_path) = self.mcp_socket_path.clone() else {
            return Ok(());
        };
        let callback: minos_chat_store::mcp_handler::ToolCallback = Arc::new(move |request| {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                event_tx
                    .send(AppEvent::McpToolCall(McpToolEvent {
                        request,
                        response_tx,
                    }))
                    .map_err(|_| anyhow::anyhow!("TUI event loop is closed"))?;
                response_rx
                    .await
                    .map_err(|_| anyhow::anyhow!("TUI dropped MCP socket response"))?
            })
        });
        tokio::spawn(async move {
            let handler =
                minos_chat_store::mcp_handler::McpSocketHandler::new(socket_path, callback);
            if let Err(error) = handler.run().await {
                tracing::warn!(
                    target: "minos_tui::backend::embedded",
                    error = %error,
                    "MCP socket handler stopped"
                );
            }
        });
        Ok(())
    }

    fn connection_state(&self) -> BackendConnectionState {
        BackendConnectionState::Embedded
    }
}
