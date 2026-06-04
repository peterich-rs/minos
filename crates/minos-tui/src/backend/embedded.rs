use super::AgentBackend;
use async_trait::async_trait;
use minos_agent_runtime::{
    AgentManager, InstanceCaps, ManagerEvent, RawIngest, StartAgentOutcome,
    store_facing::ThreadSnapshot,
};
use minos_cli_detect::{detect_all, RealCommandRunner, capture_user_shell_env};
use minos_domain::AgentName;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::broadcast;
use anyhow::Result;

pub struct EmbeddedBackend {
    manager: Arc<AgentManager>,
}

impl EmbeddedBackend {
    pub async fn new(
        workspace_root: PathBuf,
        max_instances: usize,
        idle_timeout: std::time::Duration,
    ) -> Result<Self> {
        let config = minos_agent_runtime::AgentRuntimeConfig::new(workspace_root);
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

    async fn list_threads(&self) -> Result<Vec<ThreadSnapshot>> {
        Ok(self.manager.list_threads().await)
    }

    async fn subscribe_ingest(&self) -> broadcast::Receiver<RawIngest> {
        self.manager.ingest_stream()
    }

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager.manager_event_stream()
    }
}
