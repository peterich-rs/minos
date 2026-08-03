//! Shared realtime wire types for the Minos topic-based WS gateway.
//!
//! Both the backend gateway and the client crates (mobile, daemon) use these
//! types to serialize/deserialize WebSocket text frames. Moving them here
//! ensures a single source of truth for the wire protocol.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::messages::ChatMessageSummary;

// ─── Topic model ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicKind {
    Account,
    Conversation,
    Project,
    AgentSession,
    Host,
}

impl TopicKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Conversation => "conversation",
            Self::Project => "project",
            Self::AgentSession => "agent_session",
            Self::Host => "host",
        }
    }

    pub fn parse(value: &str) -> Result<Self, TopicParseError> {
        match value {
            "account" => Ok(Self::Account),
            "conversation" => Ok(Self::Conversation),
            "project" => Ok(Self::Project),
            "agent_session" => Ok(Self::AgentSession),
            "host" => Ok(Self::Host),
            _ => Err(TopicParseError::UnknownKind),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RealtimeTopic {
    Account(String),
    Conversation(String),
    Project(String),
    AgentSession(String),
    Host(String),
}

impl RealtimeTopic {
    #[must_use]
    pub const fn kind(&self) -> TopicKind {
        match self {
            Self::Account(_) => TopicKind::Account,
            Self::Conversation(_) => TopicKind::Conversation,
            Self::Project(_) => TopicKind::Project,
            Self::AgentSession(_) => TopicKind::AgentSession,
            Self::Host(_) => TopicKind::Host,
        }
    }

    #[must_use]
    pub fn topic_string(&self) -> String {
        match self {
            Self::Account(id) => format!("account:{id}"),
            Self::Conversation(id) => format!("conversation:{id}"),
            Self::Project(id) => format!("project:{id}"),
            Self::AgentSession(id) => format!("agent_session:{id}"),
            Self::Host(id) => format!("host:{id}"),
        }
    }

    #[must_use]
    pub fn partition_key(&self) -> &str {
        match self {
            Self::Account(id)
            | Self::Conversation(id)
            | Self::Project(id)
            | Self::AgentSession(id)
            | Self::Host(id) => id,
        }
    }

    pub fn parse(value: &str) -> Result<Self, TopicParseError> {
        let (kind, partition_key) = value
            .split_once(':')
            .ok_or(TopicParseError::InvalidFormat)?;
        if partition_key.is_empty() {
            return Err(TopicParseError::MissingPartitionKey);
        }
        match TopicKind::parse(kind)? {
            TopicKind::Account => Ok(Self::Account(partition_key.to_string())),
            TopicKind::Conversation => Ok(Self::Conversation(partition_key.to_string())),
            TopicKind::Project => Ok(Self::Project(partition_key.to_string())),
            TopicKind::AgentSession => Ok(Self::AgentSession(partition_key.to_string())),
            TopicKind::Host => Ok(Self::Host(partition_key.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TopicParseError {
    #[error("unknown_topic_kind")]
    UnknownKind,
    #[error("invalid_topic_format")]
    InvalidFormat,
    #[error("missing_partition_key")]
    MissingPartitionKey,
}

// ─── Connection principal ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionPrincipal {
    Account { account_id: String },
    Host { host_installation_id: String },
}

impl ConnectionPrincipal {
    #[must_use]
    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::Account { account_id } => Some(account_id.as_str()),
            Self::Host { .. } => None,
        }
    }
}

// ─── WS wire frames ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Subscribe {
        topics: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume_after: Option<HashMap<String, i64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_request_id: Option<String>,
    },
    Unsubscribe {
        topics: Vec<String>,
    },
    Ping {
        ts: i64,
    },
    HostCommandAck {
        command_id: String,
        ack_at_ms: i64,
    },
    HostCommandResult {
        command_id: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<Value>,
        finished_at_ms: i64,
    },
    HostStreamEvent {
        topic: String,
        kind: String,
        payload: Value,
    },
    HostIngestLiveBatch {
        batch: HostIngestLiveBatch,
    },
    HostGapManifest {
        manifest: HostGapManifest,
    },
    HostIngestPullResponse {
        response: HostIngestPullResponse,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello {
        conn_id: String,
        server_time_ms: i64,
        heartbeat_interval_ms: i64,
    },
    SubscribeAck {
        topics: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_request_id: Option<String>,
    },
    SubscriptionDenied {
        topic: String,
        reason: String,
    },
    SubscriptionLimitExceeded {
        limit: usize,
        current: usize,
    },
    DurableEvent {
        topic: String,
        topic_seq: i64,
        kind: String,
        payload: Value,
        event_id: String,
    },
    StreamEvent {
        topic: String,
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<i64>,
        payload: Value,
    },
    SnapshotRequired {
        topic: String,
        last_known_seq: i64,
        retention_floor_seq: i64,
    },
    HostForceClose {
        reason: String,
        close_code: u16,
    },
    HostIngestAck {
        session_id: String,
        accepted_to_seq: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch_id: Option<String>,
    },
    PullIngestRange {
        request_id: String,
        session_id: String,
        from_seq: u64,
        to_seq: u64,
        max_bytes: u64,
        priority: PullPriority,
        reason: PullReason,
    },
    PullAck {
        request_id: String,
        session_id: String,
        accepted_to_seq: u64,
    },
    Pong {
        ts: i64,
        server_time_ms: i64,
    },
    Error {
        code: String,
        message: String,
        request_id: String,
    },
}

// ─── Presence (IM steady-state online / offline / last_seen) ─────────────

/// StreamEvent `kind` for installation presence updates.
pub const PRESENCE_STREAM_KIND: &str = "presence";

/// Recommended client application/WS heartbeat interval advertised in `Hello`.
pub const DEFAULT_HEARTBEAT_INTERVAL_MS: i64 = 25_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresencePrincipalKind {
    Host,
    AccountClient,
}

/// Live presence of one installation. Delivered as
/// `StreamEvent { kind: "presence", payload }` on interested topics
/// (`account:{id}` for host presence, `host:{id}` for account-client presence).
/// Ephemeral: cold path is HTTP list with `online` + `last_seen_at_ms`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PresencePayload {
    pub installation_id: String,
    pub principal_kind: PresencePrincipalKind,
    pub online: bool,
    pub last_seen_at_ms: i64,
    pub at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

// ─── Host ingest sync ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullPriority {
    LiveCritical,
    ClientOpenedHistory,
    IdleBackfill,
    Audit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullReason {
    ClientOpenedHistory,
    IdleBackfill,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeqRange {
    pub from: u64,
    pub to: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostIngestChunk {
    pub event_id: String,
    pub session_id: String,
    pub seq: u64,
    pub agent: minos_domain::AgentName,
    pub kind: String,
    pub payload: Value,
    /// Host-local conversation id (daemon). Used by hub to auto-register a
    /// formal `agent_sessions` row when the session is first projected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Deprecated for cloud SSOT: hub re-translates from `payload`. Kept for
    /// wire compatibility; receivers should ignore.
    #[serde(default)]
    pub projection: Vec<minos_ui_protocol::UiEventMessage>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub byte_len: u64,
    pub checksum_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostIngestLiveBatch {
    pub batch_id: String,
    pub host_id: minos_domain::DeviceId,
    pub chunks: Vec<HostIngestChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionGapManifest {
    pub session_id: String,
    pub backend_acked_seq: u64,
    pub local_from_seq: u64,
    pub local_to_seq: u64,
    #[serde(default)]
    pub missing_ranges: Vec<SeqRange>,
    pub bytes: u64,
    pub event_count: u64,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostGapManifest {
    pub manifest_id: String,
    pub host_id: minos_domain::DeviceId,
    pub sessions: Vec<SessionGapManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostIngestPullResponse {
    pub request_id: String,
    pub session_id: String,
    pub from_seq: u64,
    pub to_seq: u64,
    pub chunks: Vec<HostIngestChunk>,
    pub has_more: bool,
}

// ─── Durable events ───────────────────────────────────────────────────

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
    User {
        account_id: String,
    },
    Agent {
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// Account roster: host paired (T2 digest on `account:{id}`).
    HostLinked {
        account_id: String,
        host_installation_id: String,
        pair_id: String,
        at_ms: i64,
        /// Display name for immediate list upsert without HTTP.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_display_name: Option<String>,
    },
    /// Account roster: host unlinked (T2 digest on `account:{id}`).
    HostUnlinked {
        account_id: String,
        host_installation_id: String,
        at_ms: i64,
    },
    /// Social graph: friend request created / accepted / rejected (T2 on account).
    /// One event per affected account (from and/or to); clients invalidate roster HTTP.
    FriendRequestUpdated {
        account_id: String,
        request_id: String,
        from_account_id: String,
        to_account_id: String,
        /// `"pending"` | `"accepted"` | `"rejected"`
        status: String,
        at_ms: i64,
    },
    AgentSessionStarted {
        session_id: String,
        conversation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// Open-conversation timeline hot path (`conversation:{id}`).
    /// Optional full `message` lets clients apply without an extra HTTP fetch.
    ConversationMessageAppended {
        conversation_id: String,
        message_id: String,
        sender: SenderRef,
        at_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<ChatMessageSummary>,
    },
    ConversationMessageRecalled {
        conversation_id: String,
        message_id: String,
        at_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<ChatMessageSummary>,
    },
    /// Conversation-only reaction aggregate update (no account fanout / rail unread).
    ConversationMessageReactionUpdated {
        conversation_id: String,
        message_id: String,
        emoji: String,
        /// `"add"` | `"remove"` — animation hint only; not authoritative state.
        action: String,
        actor: crate::SenderRef,
        at_ms: i64,
        /// AUTHORITATIVE full aggregate for `message_id` (viewer-neutral wire shape;
        /// clients resolve `reacted_by_me` from actors when needed).
        reactions: Vec<crate::ReactionGroup>,
    },
    AccountConversationMessageAppended {
        account_id: String,
        conversation_id: String,
        message_id: String,
        sender: SenderRef,
        at_ms: i64,
        message: ChatMessageSummary,
    },
    AccountConversationMessageRecalled {
        account_id: String,
        conversation_id: String,
        message_id: String,
        at_ms: i64,
        message: ChatMessageSummary,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_session_id: Option<String>,
        method: String,
        params: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_by_account_id: Option<String>,
        deadline_at_ms: i64,
        at_ms: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableEventEnvelope {
    pub topic: String,
    pub topic_seq: i64,
    pub event_id: String,
    pub payload: DurableEvent,
}

impl DurableEvent {
    #[must_use]
    pub fn topic_kind(&self) -> TopicKind {
        self.topic().kind()
    }

    #[must_use]
    pub fn event_kind_str(&self) -> &'static str {
        match self {
            Self::AccountRegistered { .. } => "account_registered",
            Self::AccountPasswordChanged { .. } => "account_password_changed",
            Self::HostLinked { .. } => "host_linked",
            Self::HostUnlinked { .. } => "host_unlinked",
            Self::FriendRequestUpdated { .. } => "friend_request_updated",
            Self::AgentSessionStarted { .. } => "agent_session_started",
            Self::AgentSessionEnded { .. } => "agent_session_ended",
            Self::AgentTurnAppended { .. } => "agent_turn_appended",
            Self::ApprovalRequested { .. } => "approval_requested",
            Self::ApprovalResolved { .. } => "approval_resolved",
            Self::ConversationMessageAppended { .. } => "conversation_message_appended",
            Self::ConversationMessageRecalled { .. } => "conversation_message_recalled",
            Self::ConversationMessageReactionUpdated { .. } => {
                "conversation_message_reaction_updated"
            }
            Self::AccountConversationMessageAppended { .. } => {
                "account_conversation_message_appended"
            }
            Self::AccountConversationMessageRecalled { .. } => {
                "account_conversation_message_recalled"
            }
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
            | Self::HostUnlinked { account_id, .. }
            | Self::FriendRequestUpdated { account_id, .. } => {
                RealtimeTopic::Account(account_id.clone())
            }
            Self::AgentSessionStarted { session_id, .. }
            | Self::AgentSessionEnded { session_id, .. }
            | Self::AgentTurnAppended { session_id, .. }
            | Self::ApprovalRequested { session_id, .. }
            | Self::ApprovalResolved { session_id, .. } => {
                RealtimeTopic::AgentSession(session_id.clone())
            }
            Self::ConversationMessageAppended {
                conversation_id, ..
            }
            | Self::ConversationMessageRecalled {
                conversation_id, ..
            }
            | Self::ConversationMessageReactionUpdated {
                conversation_id, ..
            } => RealtimeTopic::Conversation(conversation_id.clone()),
            Self::AccountConversationMessageAppended { account_id, .. }
            | Self::AccountConversationMessageRecalled { account_id, .. } => {
                RealtimeTopic::Account(account_id.clone())
            }
            Self::ProjectConversationLinked { project_id, .. }
            | Self::ProjectArchived { project_id, .. } => {
                RealtimeTopic::Project(project_id.clone())
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_frame_subscribe_round_trip() {
        let frame = ClientFrame::Subscribe {
            topics: vec!["account:abc".into(), "conversation:def".into()],
            resume_after: Some(HashMap::from([("account:abc".into(), 42i64)])),
            client_request_id: Some("req-1".into()),
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ClientFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn server_frame_hello_round_trip() {
        let frame = ServerFrame::Hello {
            conn_id: "conn-1".into(),
            server_time_ms: 1_760_000_000_000,
            heartbeat_interval_ms: 25_000,
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn server_frame_durable_event_round_trip() {
        let frame = ServerFrame::DurableEvent {
            topic: "account:abc".into(),
            topic_seq: 7,
            kind: "host_linked".into(),
            payload: serde_json::json!({"host_installation_id": "host-1"}),
            event_id: "evt-1".into(),
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn server_frame_stream_event_round_trip() {
        let frame = ServerFrame::StreamEvent {
            topic: "agent_session:sess-1".into(),
            kind: "agent_text_delta".into(),
            seq: Some(3),
            payload: serde_json::json!({"delta": "hello"}),
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn realtime_topic_parse_round_trip() {
        for topic in [
            RealtimeTopic::Account("acct-1".into()),
            RealtimeTopic::Conversation("conv-1".into()),
            RealtimeTopic::Project("proj-1".into()),
            RealtimeTopic::AgentSession("sess-1".into()),
            RealtimeTopic::Host("host-1".into()),
        ] {
            let s = topic.topic_string();
            let back = RealtimeTopic::parse(&s).unwrap();
            assert_eq!(topic, back);
        }
    }

    #[test]
    fn client_frame_host_command_result_round_trip() {
        let frame = ClientFrame::HostCommandResult {
            command_id: "cmd-1".into(),
            status: "succeeded".into(),
            result: Some(serde_json::json!({"session_id": "sess-1"})),
            error: None,
            finished_at_ms: 1_760_000_000_000,
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ClientFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn client_frame_host_stream_event_round_trip() {
        let frame = ClientFrame::HostStreamEvent {
            topic: "agent_session:sess-1".into(),
            kind: "agent_text_delta".into(),
            payload: serde_json::json!({"delta": "hi", "turn_id": "turn-1", "seq": 1}),
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ClientFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    fn sample_host_ingest_chunk(seq: u64) -> HostIngestChunk {
        HostIngestChunk {
            event_id: format!("evt-{seq}"),
            session_id: "thr-sync".into(),
            seq,
            agent: minos_domain::AgentName::Codex,
            kind: "agent_event".into(),
            payload: serde_json::json!({
                "method": "item/agentMessage/delta",
                "params": {"delta": "hello"}
            }),
            conversation_id: Some("conv-local-1".into()),
            projection: vec![minos_ui_protocol::UiEventMessage::Raw {
                kind: "agent_event".into(),
                payload_json: "{}".into(),
            }],
            first_ts_ms: 1_760_000_000_000,
            last_ts_ms: 1_760_000_000_010,
            byte_len: 42,
            checksum_sha256: format!("checksum-{seq}"),
        }
    }

    #[test]
    fn client_frame_host_ingest_live_batch_round_trip() {
        let frame = ClientFrame::HostIngestLiveBatch {
            batch: HostIngestLiveBatch {
                batch_id: "batch-1".into(),
                host_id: minos_domain::DeviceId::new(),
                chunks: vec![sample_host_ingest_chunk(1)],
            },
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ClientFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn client_frame_host_gap_manifest_round_trip() {
        let frame = ClientFrame::HostGapManifest {
            manifest: HostGapManifest {
                manifest_id: "manifest-1".into(),
                host_id: minos_domain::DeviceId::new(),
                sessions: vec![SessionGapManifest {
                    session_id: "thr-A".into(),
                    backend_acked_seq: 100,
                    local_from_seq: 101,
                    local_to_seq: 500,
                    missing_ranges: vec![SeqRange { from: 101, to: 500 }],
                    bytes: 1_000_000,
                    event_count: 400,
                    first_ts_ms: 1_760_000_000_000,
                    last_ts_ms: 1_760_000_050_000,
                    running: true,
                }],
            },
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ClientFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn client_frame_host_ingest_pull_response_round_trip() {
        let frame = ClientFrame::HostIngestPullResponse {
            response: HostIngestPullResponse {
                request_id: "pull-1".into(),
                session_id: "thr-sync".into(),
                from_seq: 1,
                to_seq: 2,
                chunks: vec![sample_host_ingest_chunk(1), sample_host_ingest_chunk(2)],
                has_more: false,
            },
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ClientFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn server_frame_pull_and_ack_round_trip() {
        let pull = ServerFrame::PullIngestRange {
            request_id: "pull-1".into(),
            session_id: "thr-sync".into(),
            from_seq: 1,
            to_seq: 50,
            max_bytes: 2_000_000,
            priority: PullPriority::ClientOpenedHistory,
            reason: PullReason::ClientOpenedHistory,
        };
        let json = serde_json::to_string(&pull).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(pull, back);

        let ack = ServerFrame::HostIngestAck {
            session_id: "thr-sync".into(),
            accepted_to_seq: 50,
            batch_id: Some("batch-1".into()),
        };
        let json = serde_json::to_string(&ack).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(ack, back);

        let pull_ack = ServerFrame::PullAck {
            request_id: "pull-1".into(),
            session_id: "thr-sync".into(),
            accepted_to_seq: 50,
        };
        let json = serde_json::to_string(&pull_ack).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(pull_ack, back);
    }

    #[test]
    fn durable_event_round_trip() {
        let event = DurableEvent::HostCommandIssued {
            command_id: "cmd-1".into(),
            host_installation_id: "host-1".into(),
            agent_session_id: Some("sess-1".into()),
            method: "start_agent".into(),
            params: serde_json::json!({"agent": "codex"}),
            requested_by_account_id: Some("acct-1".into()),
            deadline_at_ms: 1_760_000_060_000,
            at_ms: 1_760_000_000_000,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: DurableEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn server_frame_snapshot_required_round_trip() {
        let frame = ServerFrame::SnapshotRequired {
            topic: "account:abc".into(),
            last_known_seq: 0,
            retention_floor_seq: 50,
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn server_frame_host_force_close_round_trip() {
        let frame = ServerFrame::HostForceClose {
            reason: "auth_revoked".into(),
            close_code: 4401,
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: ServerFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }
}
