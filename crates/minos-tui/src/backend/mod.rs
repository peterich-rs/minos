use anyhow::Result;
use async_trait::async_trait;
use minos_agent_runtime::{ManagerEvent, RawIngest, StartAgentOutcome};
use minos_domain::AgentDescriptor;
use minos_domain::AgentName;
use minos_protocol::LocalGroupChatMessage;
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::broadcast;

use crate::event::AppEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendConnectionState {
    Embedded,
    Connected {
        endpoint: String,
    },
    Disconnected {
        endpoint: String,
        last_error: Option<String>,
    },
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackendKind {
    #[default]
    Embedded,
    Daemon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendThreadSnapshot {
    pub thread_id: String,
    pub agent: Option<AgentName>,
    pub workspace: PathBuf,
    pub state: minos_agent_runtime::ThreadState,
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>>;

    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome>;

    async fn send_message(&self, thread_id: &str, text: &str) -> Result<()>;

    async fn send_approval_decision(
        &self,
        request_id: &str,
        thread_id: &str,
        decision: Value,
    ) -> Result<()>;

    async fn respond_opencode_permission(
        &self,
        thread_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<()>;

    async fn interrupt_thread(&self, thread_id: &str) -> Result<()>;

    async fn close_thread(&self, thread_id: &str) -> Result<()>;

    async fn delete_thread(&self, thread_id: &str) -> Result<()>;

    async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>>;

    async fn resume_thread(&self, thread_id: &str) -> Result<StartAgentOutcome>;

    async fn read_thread_raw_history(
        &self,
        thread_id: &str,
        from_seq: Option<u64>,
        limit: u32,
    ) -> Result<minos_protocol::local_rpc::ReadThreadRawHistoryResponse>;

    async fn read_group_chat(
        &self,
        room_id: &str,
        after_seq: Option<u64>,
        before_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<LocalGroupChatMessage>>;

    async fn subscribe_ingest(&self) -> broadcast::Receiver<RawIngest>;

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent>;

    fn start_mcp_socket_handler(
        &self,
        _event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    ) -> Result<()> {
        Ok(())
    }

    fn connection_state(&self) -> BackendConnectionState;
}

pub mod daemon;
pub mod embedded;
pub use daemon::DaemonBackend;
pub use embedded::EmbeddedBackend;
