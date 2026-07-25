use anyhow::Result;
use async_trait::async_trait;
use minos_agent_runtime::{
    CloseReason as RuntimeCloseReason, ManagerEvent, PauseReason as RuntimePauseReason,
    SessionState as RuntimeSessionState, StartAgentOutcome,
};
use minos_domain::AgentDescriptor;
use minos_domain::AgentName;
use minos_protocol::LocalIngestFrame;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendConnectionState {
    Connected {
        endpoint: String,
    },
    Disconnected {
        endpoint: String,
        last_error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSessionSnapshot {
    pub session_id: String,
    pub agent: Option<AgentName>,
    pub workspace: PathBuf,
    pub state: minos_agent_runtime::SessionState,
    pub parent_session_id: Option<String>,
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
pub struct SessionSummaryEntry {
    pub session_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub parent_session_id: Option<String>,
    pub state: RuntimeSessionState,
    pub needs_continue: bool,
}

impl SessionSummaryEntry {
    pub fn from_summary(s: &minos_protocol::SessionSummary) -> Self {
        Self {
            session_id: s.session_id.clone(),
            agent: s.agent,
            title: s.title.clone(),
            first_ts_ms: s.first_ts_ms,
            last_ts_ms: s.last_ts_ms,
            message_count: s.message_count,
            ended_at_ms: s.ended_at_ms,
            parent_session_id: s.parent_session_id.clone(),
            state: protocol_session_state_to_runtime(&s.state),
            needs_continue: s.needs_continue,
        }
    }
}

fn protocol_session_state_to_runtime(state: &minos_protocol::SessionState) -> RuntimeSessionState {
    match state {
        minos_protocol::SessionState::Starting => RuntimeSessionState::Starting,
        minos_protocol::SessionState::Idle => RuntimeSessionState::Idle,
        minos_protocol::SessionState::Running { turn_started_at_ms } => {
            RuntimeSessionState::Running {
                turn_started_at_ms: *turn_started_at_ms,
            }
        }
        minos_protocol::SessionState::Suspended { reason } => RuntimeSessionState::Suspended {
            reason: protocol_pause_reason_to_runtime(reason),
        },
        minos_protocol::SessionState::Resuming => RuntimeSessionState::Resuming,
        minos_protocol::SessionState::Closed { reason } => RuntimeSessionState::Closed {
            reason: protocol_close_reason_to_runtime(reason),
        },
    }
}

fn protocol_pause_reason_to_runtime(reason: &minos_protocol::PauseReason) -> RuntimePauseReason {
    match reason {
        minos_protocol::PauseReason::UserInterrupt => RuntimePauseReason::UserInterrupt,
        minos_protocol::PauseReason::CodexCrashed => RuntimePauseReason::CodexCrashed,
        minos_protocol::PauseReason::DaemonRestart => RuntimePauseReason::DaemonRestart,
        minos_protocol::PauseReason::InstanceReaped => RuntimePauseReason::InstanceReaped,
    }
}

fn protocol_close_reason_to_runtime(reason: &minos_protocol::CloseReason) -> RuntimeCloseReason {
    match reason {
        minos_protocol::CloseReason::UserClose => RuntimeCloseReason::UserClose,
        minos_protocol::CloseReason::TerminalError => RuntimeCloseReason::TerminalError,
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
    pub session_id: Option<String>,
    pub created_at_ms: i64,
    pub sender_role: String,
    pub agent: Option<AgentName>,
    pub body: String,
    pub reply_to_message_id: Option<String>,
    pub delegation_id: Option<String>,
    pub mentions: Vec<minos_protocol::ConversationMention>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessageEvent {
    pub conversation_id: String,
    pub message_seq: i64,
}

impl ConversationMessageEntry {
    pub fn from_message(s: &minos_protocol::LocalConversationMessage) -> Self {
        Self {
            message_seq: s.message_seq,
            message_id: s.message_id.clone(),
            conversation_id: s.conversation_id.clone(),
            session_id: s.session_id.clone(),
            created_at_ms: s.created_at_ms,
            sender_role: s.sender_role.clone(),
            agent: s.agent,
            body: s.body.clone(),
            reply_to_message_id: s.reply_to_message_id.clone(),
            delegation_id: s.delegation_id.clone(),
            mentions: s.mentions.clone(),
        }
    }
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>>;

    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome>;

    async fn send_message(&self, session_id: &str, text: &str) -> Result<()>;

    async fn send_approval_decision(
        &self,
        request_id: &str,
        session_id: &str,
        decision: Value,
    ) -> Result<()>;

    async fn respond_opencode_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<()>;

    async fn respond_opencode_question(
        &self,
        session_id: &str,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> Result<()>;

    async fn interrupt_session(&self, session_id: &str) -> Result<()>;

    async fn close_session(&self, session_id: &str) -> Result<()>;

    async fn delete_session(&self, session_id: &str) -> Result<()>;

    async fn list_sessions(&self) -> Result<Vec<BackendSessionSnapshot>>;

    async fn list_projects(&self) -> Result<Vec<ProjectEntry>>;

    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry>;

    async fn list_conversations(&self, project_id: &str) -> Result<Vec<ConversationEntry>>;

    /// Create a conversation. `agents` is the runtime roster (who may be
    /// @mentioned / started). Empty is allowed but then no agent can join until
    /// membership is set at create time by a client that supports it.
    async fn create_conversation(
        &self,
        project_id: &str,
        title: &str,
        agents: &[AgentName],
    ) -> Result<ConversationEntry>;

    async fn list_conversation_messages(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationMessageEntry>>;

    async fn list_conversation_agent_sessions(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<SessionSummaryEntry>>;

    /// Start an agent in a conversation.
    ///
    /// When `profile_id` is set, leave model/effort/instructions unset so
    /// daemon `resolve_launch_options` fills them from the profile.
    async fn start_agent_in_conversation(
        &self,
        conversation_id: &str,
        agent: AgentName,
        workspace: PathBuf,
        profile_id: Option<String>,
    ) -> Result<StartAgentOutcome>;

    /// List host agent profiles (for @-mentions + bare-agent newest-profile default).
    async fn list_agent_profiles(&self) -> Result<Vec<minos_protocol::AgentProfileSummary>> {
        Ok(Vec::new())
    }

    async fn append_conversation_message(
        &self,
        conversation_id: &str,
        message_id: Option<&str>,
        session_id: Option<&str>,
        sender_role: &str,
        agent: Option<AgentName>,
        body: &str,
    ) -> Result<()>;

    /// Reattach a persisted/suspended thread. When `auto_continue` is true and
    /// the store has `needs_continue`, injects a one-shot CONTINUE prompt.
    async fn resume_session(
        &self,
        session_id: &str,
        auto_continue: bool,
    ) -> Result<StartAgentOutcome>;

    async fn read_session_raw_history(
        &self,
        session_id: &str,
        from_seq: Option<u64>,
        limit: u32,
    ) -> Result<minos_protocol::local_rpc::ReadSessionRawHistoryResponse>;

    async fn subscribe_ingest(&self) -> broadcast::Receiver<LocalIngestFrame>;

    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent>;

    async fn subscribe_conversation_message_events(
        &self,
    ) -> broadcast::Receiver<ConversationMessageEvent>;

    fn connection_state(&self) -> BackendConnectionState;
}

pub mod daemon;
pub use daemon::DaemonBackend;
