use super::{AgentBackend, BackendConnectionState, BackendThreadSnapshot};
use anyhow::Result;
use async_trait::async_trait;
use minos_agent_runtime::{AgentManager, InstanceCaps, ManagerEvent, RawIngest, StartAgentOutcome};
use minos_cli_detect::{capture_user_shell_env, detect_all, RealCommandRunner};
use minos_domain::AgentName;
use minos_protocol::local_rpc::ReadThreadRawHistoryResponse;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::broadcast;

pub struct EmbeddedBackend {
    manager: Arc<AgentManager>,
}

impl EmbeddedBackend {
    pub async fn new(
        workspace_root: PathBuf,
        max_instances: usize,
        idle_timeout: std::time::Duration,
    ) -> Result<Self> {
        let shell_env = capture_user_shell_env().await;
        let mut config = minos_agent_runtime::AgentRuntimeConfig::new(workspace_root);
        config.subprocess_env = Arc::new(shell_env);
        let caps = InstanceCaps {
            max_instances,
            idle_timeout,
        };
        let manager = AgentManager::new(config, caps);
        Ok(Self {
            manager: Arc::new(manager),
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

    async fn subscribe_ingest(&self) -> broadcast::Receiver<RawIngest> {
        self.manager.ingest_stream()
    }

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager.manager_event_stream()
    }

    fn connection_state(&self) -> BackendConnectionState {
        BackendConnectionState::Embedded
    }
}
