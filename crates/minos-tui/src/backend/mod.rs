use async_trait::async_trait;
use minos_agent_runtime::{ManagerEvent, RawIngest, StartAgentOutcome};
use minos_domain::AgentDescriptor;
use minos_domain::AgentName;
use std::path::PathBuf;
use tokio::sync::broadcast;
use anyhow::Result;

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>>;

    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome>;

    async fn send_message(&self, thread_id: &str, text: &str) -> Result<()>;

    async fn interrupt_thread(&self, thread_id: &str) -> Result<()>;

    async fn close_thread(&self, thread_id: &str) -> Result<()>;

    async fn list_threads(&self) -> Result<Vec<minos_agent_runtime::store_facing::ThreadSnapshot>>;

    async fn subscribe_ingest(&self) -> broadcast::Receiver<RawIngest>;

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent>;
}

pub mod embedded;
pub use embedded::EmbeddedBackend;
