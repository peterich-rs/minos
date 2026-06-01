use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::topic::RealtimeTopic;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ApprovalResolution {
    Decided { decision: Value },
    Timeout,
    Disconnect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SenderRef {
    User { account_id: String },
    Agent {
        agent_id: String,
        session_id: Option<String>,
    },
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DurableEvent {
    AccountRegistered {
        account_id: String,
        at_ms: i64,
    },
    AccountPasswordChanged {
        account_id: String,
        at_ms: i64,
    },
    HostLinked {
        account_id: String,
        host_installation_id: String,
        pair_id: String,
        at_ms: i64,
    },
    HostUnlinked {
        account_id: String,
        host_installation_id: String,
        at_ms: i64,
    },
    AgentSessionStarted {
        session_id: String,
        conversation_id: String,
        project_id: Option<String>,
        host_installation_id: String,
        agent_id: String,
        at_ms: i64,
    },
    AgentSessionEnded {
        session_id: String,
        status: String,
        at_ms: i64,
    },
    AgentTurnAppended {
        session_id: String,
        turn_id: String,
        turn_seq: i64,
        role: String,
        status: String,
        at_ms: i64,
    },
    ApprovalRequested {
        request_id: String,
        session_id: String,
        method: String,
        deadline_at_ms: i64,
        at_ms: i64,
    },
    ApprovalResolved {
        request_id: String,
        session_id: String,
        resolution: ApprovalResolution,
        at_ms: i64,
    },
    ConversationMessageAppended {
        conversation_id: String,
        message_id: String,
        sender: SenderRef,
        at_ms: i64,
    },
    ConversationMessageRecalled {
        conversation_id: String,
        message_id: String,
        at_ms: i64,
    },
    ProjectConversationLinked {
        project_id: String,
        conversation_id: String,
        at_ms: i64,
    },
    ProjectArchived {
        project_id: String,
        at_ms: i64,
    },
    HostForceClose {
        host_installation_id: String,
        reason: String,
        at_ms: i64,
    },
    HostCommandIssued {
        command_id: String,
        host_installation_id: String,
        agent_session_id: Option<String>,
        method: String,
        params: Value,
        requested_by_account_id: Option<String>,
        deadline_at_ms: i64,
        at_ms: i64,
    },
}

// ---------------------------------------------------------------------------
// DurableEventEnvelope — wraps a persisted event with its topic cursor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableEventEnvelope {
    pub topic: String,
    pub topic_seq: i64,
    pub event_id: String,
    pub payload: DurableEvent,
}

impl DurableEvent {
    /// The `TopicKind` this event belongs to.
    #[must_use]
    pub fn topic_kind(&self) -> super::topic::TopicKind {
        self.topic().kind()
    }

    /// The serde tag value for this event variant (e.g. `"account_registered"`).
    #[must_use]
    pub fn event_kind_str(&self) -> &'static str {
        match self {
            Self::AccountRegistered { .. } => "account_registered",
            Self::AccountPasswordChanged { .. } => "account_password_changed",
            Self::HostLinked { .. } => "host_linked",
            Self::HostUnlinked { .. } => "host_unlinked",
            Self::AgentSessionStarted { .. } => "agent_session_started",
            Self::AgentSessionEnded { .. } => "agent_session_ended",
            Self::AgentTurnAppended { .. } => "agent_turn_appended",
            Self::ApprovalRequested { .. } => "approval_requested",
            Self::ApprovalResolved { .. } => "approval_resolved",
            Self::ConversationMessageAppended { .. } => "conversation_message_appended",
            Self::ConversationMessageRecalled { .. } => "conversation_message_recalled",
            Self::ProjectConversationLinked { .. } => "project_conversation_linked",
            Self::ProjectArchived { .. } => "project_archived",
            Self::HostForceClose { .. } => "host_force_close",
            Self::HostCommandIssued { .. } => "host_command_issued",
        }
    }

    #[must_use]
    pub fn topic(&self) -> RealtimeTopic {
        match self {
            Self::AccountRegistered { account_id, .. }
            | Self::AccountPasswordChanged { account_id, .. }
            | Self::HostLinked { account_id, .. }
            | Self::HostUnlinked { account_id, .. } => RealtimeTopic::Account(account_id.clone()),
            Self::AgentSessionStarted { session_id, .. }
            | Self::AgentSessionEnded { session_id, .. }
            | Self::AgentTurnAppended { session_id, .. }
            | Self::ApprovalRequested { session_id, .. }
            | Self::ApprovalResolved { session_id, .. } => {
                RealtimeTopic::AgentSession(session_id.clone())
            }
            Self::ConversationMessageAppended { conversation_id, .. }
            | Self::ConversationMessageRecalled { conversation_id, .. } => {
                RealtimeTopic::Conversation(conversation_id.clone())
            }
            Self::ProjectConversationLinked { project_id, .. }
            | Self::ProjectArchived { project_id, .. } => RealtimeTopic::Project(project_id.clone()),
            Self::HostForceClose {
                host_installation_id,
                ..
            }
            | Self::HostCommandIssued {
                host_installation_id,
                ..
            } => RealtimeTopic::Host(host_installation_id.clone()),
        }
    }
}
