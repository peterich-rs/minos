use anyhow::Result;
use async_trait::async_trait;
use minos_agent_runtime::{ManagerEvent, StartAgentOutcome};
use minos_domain::AgentDescriptor;
use minos_domain::AgentName;
use minos_protocol::{LocalGroupChatMessage, LocalIngestFrame};
use serde_json::Value;
use std::path::{Path, PathBuf};
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
    pub parent_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub project_id: String,
    pub name: String,
    pub workspace_path: PathBuf,
    pub thread_count: u32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ProjectEntry {
    pub fn from_summary(s: &minos_protocol::ProjectSummary, fallback_cwd: &Path) -> Self {
        let workspace_path = s
            .workspace_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| fallback_cwd.to_path_buf());
        Self {
            project_id: s.project_id.clone(),
            name: s.name.clone(),
            workspace_path,
            thread_count: s.thread_count,
            created_at_ms: s.created_at_ms,
            updated_at_ms: s.updated_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummaryEntry {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub parent_thread_id: Option<String>,
}

impl ThreadSummaryEntry {
    pub fn from_summary(s: &minos_protocol::ThreadSummary) -> Self {
        Self {
            thread_id: s.thread_id.clone(),
            agent: s.agent,
            title: s.title.clone(),
            first_ts_ms: s.first_ts_ms,
            last_ts_ms: s.last_ts_ms,
            message_count: s.message_count,
            ended_at_ms: s.ended_at_ms,
            parent_thread_id: s.parent_thread_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationEntry {
    pub conversation_id: String,
    pub project_id: String,
    pub title: String,
    pub last_message_preview: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message_count: u32,
    pub agent_session_count: u32,
    pub participating_agents: Vec<AgentName>,
}

impl ConversationEntry {
    pub fn from_summary(s: &minos_protocol::LocalConversationSummary) -> Self {
        Self {
            conversation_id: s.conversation_id.clone(),
            project_id: s.project_id.clone(),
            title: s.title.clone(),
            last_message_preview: s.last_message_preview.clone(),
            created_at_ms: s.created_at_ms,
            updated_at_ms: s.updated_at_ms,
            message_count: s.message_count,
            agent_session_count: s.agent_session_count,
            participating_agents: s.participating_agents.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessageEntry {
    pub message_seq: i64,
    pub message_id: String,
    pub conversation_id: String,
    pub thread_id: Option<String>,
    pub created_at_ms: i64,
    pub sender_role: String,
    pub agent: Option<AgentName>,
    pub body: String,
}

impl ConversationMessageEntry {
    pub fn from_message(s: &minos_protocol::LocalConversationMessage) -> Self {
        Self {
            message_seq: s.message_seq,
            message_id: s.message_id.clone(),
            conversation_id: s.conversation_id.clone(),
            thread_id: s.thread_id.clone(),
            created_at_ms: s.created_at_ms,
            sender_role: s.sender_role.clone(),
            agent: s.agent,
            body: s.body.clone(),
        }
    }
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

    async fn respond_opencode_question(
        &self,
        thread_id: &str,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> Result<()>;

    async fn interrupt_thread(&self, thread_id: &str) -> Result<()>;

    async fn close_thread(&self, thread_id: &str) -> Result<()>;

    async fn delete_thread(&self, thread_id: &str) -> Result<()>;

    async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>>;

    async fn list_projects(&self) -> Result<Vec<ProjectEntry>>;

    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry>;

    async fn list_conversations(&self, project_id: &str) -> Result<Vec<ConversationEntry>>;

    async fn create_conversation(&self, project_id: &str, title: &str)
        -> Result<ConversationEntry>;

    async fn list_conversation_messages(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationMessageEntry>>;

    async fn list_conversation_agent_sessions(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ThreadSummaryEntry>>;

    async fn start_agent_in_conversation(
        &self,
        conversation_id: &str,
        agent: AgentName,
        workspace: PathBuf,
    ) -> Result<StartAgentOutcome>;

    async fn append_conversation_message(
        &self,
        conversation_id: &str,
        thread_id: Option<&str>,
        sender_role: &str,
        agent: Option<AgentName>,
        body: &str,
    ) -> Result<()>;

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

    async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame>;

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
