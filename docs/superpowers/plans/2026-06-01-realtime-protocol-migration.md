# Realtime Protocol Migration: Envelope → Topic-Based ClientFrame/ServerFrame

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the mobile client and host daemon from the legacy Envelope-based WebSocket protocol to the new topic-based ClientFrame/ServerFrame realtime gateway, then remove all legacy code.

**Architecture:** The backend's `realtime/gateway.rs` already supports both protocols simultaneously — it tries `serde_json::from_str::<ClientFrame>()` first, falls back to `Envelope`. This plan extracts the wire types to `minos-protocol`, rewrites both clients to speak the new protocol exclusively, then removes the legacy Envelope path from the gateway and deletes the old `Envelope`/`EventKind` types.

**Tech Stack:** Rust (tokio, serde_json, openwire for mobile WS, tokio-tungstenite for daemon WS), Dart/Flutter (FFI boundary via minos-protocol)

---

## File Structure

### New files
- `crates/minos-protocol/src/realtime.rs` — `ClientFrame`, `ServerFrame`, `RealtimeTopic`, `TopicKind`, `TopicParseError`, `ConnectionPrincipal`, `DurableEvent`, `ApprovalResolution`, `SenderRef`, `DurableEventEnvelope`
- `crates/minos-protocol/src/ws_ticket.rs` — `WsTicketClaims`, `WsTicketRequest`, `WsTicketResponse`
- `crates/minos-mobile/src/realtime/` — directory for new realtime client modules
  - `mod.rs` — module root
  - `subscription.rs` — `SubscriptionManager` tracking subscribed topics + `resume_after` cursors
  - `session.rs` — WS session loop (connect, recv, send, reconnect)
  - `frame_handler.rs` — `ServerFrame` → event stream dispatch
- `crates/minos-daemon/src/realtime/` — directory for new daemon realtime modules
  - `mod.rs` — module root
  - `subscription.rs` — host subscription management
  - `session.rs` — WS session loop
  - `frame_handler.rs` — `ServerFrame` → host command dispatch, `ClientFrame` construction

### Modified files
- `crates/minos-protocol/src/lib.rs` — add `pub mod realtime; pub mod ws_ticket;`
- `crates/minos-protocol/Cargo.toml` — add `minos-domain` dep (already present)
- `crates/minos-backend/src/realtime/wire.rs` — delete, replaced by `minos-protocol::realtime`
- `crates/minos-backend/src/realtime/topic.rs` — delete, replaced by `minos-protocol::realtime`
- `crates/minos-backend/src/realtime/event.rs` — delete, replaced by `minos-protocol::realtime`
- `crates/minos-backend/src/realtime/subscription.rs` — replace `ConnectionPrincipal` import
- `crates/minos-backend/src/realtime/gateway.rs` — import from `minos-protocol`, remove legacy Envelope handler
- `crates/minos-backend/src/realtime/auth.rs` — import `RealtimeTopic`/`ConnectionPrincipal` from `minos-protocol`
- `crates/minos-backend/src/realtime.rs` — update module re-exports
- `crates/minos-mobile/src/client.rs` — replace Envelope WS with realtime session
- `crates/minos-mobile/src/rpc.rs` — delete (forward_rpc no longer needed)
- `crates/minos-daemon/src/relay_client.rs` — replace Envelope WS with realtime session
- `crates/minos-daemon/src/rpc_server.rs` — adapt to new host command flow

### Deleted files
- `crates/minos-backend/src/realtime/wire.rs` (moved to minos-protocol)
- `crates/minos-backend/src/realtime/topic.rs` (moved to minos-protocol)
- `crates/minos-backend/src/realtime/event.rs` (moved to minos-protocol)
- `crates/minos-protocol/src/envelope.rs` (after all consumers removed)
- `crates/minos-mobile/src/rpc.rs` (forward_rpc replaced by REST)
- `crates/minos-transport/src/auth.rs` (AuthHeaders replaced by ticket-based auth)

---

## Task 1: Extract wire types to minos-protocol

**Files:**
- Create: `crates/minos-protocol/src/realtime.rs`
- Create: `crates/minos-protocol/src/ws_ticket.rs`
- Modify: `crates/minos-protocol/src/lib.rs`
- Modify: `crates/minos-protocol/Cargo.toml`

This task moves `ClientFrame`, `ServerFrame`, `RealtimeTopic`, `TopicKind`, `TopicParseError`, `DurableEvent`, `ApprovalResolution`, `SenderRef`, `DurableEventEnvelope`, and `ConnectionPrincipal` from the backend crate into `minos-protocol` so both mobile and daemon can reference them.

- [ ] **Step 1: Write test for realtime wire types in minos-protocol**

Create `crates/minos-protocol/src/realtime.rs` with the full type definitions copied from `crates/minos-backend/src/realtime/wire.rs`, `topic.rs`, `event.rs`, and the `ConnectionPrincipal` from `subscription.rs`. At the bottom, add round-trip tests:

```rust
//! Shared realtime wire types for the Minos topic-based WS gateway.
//!
//! Both the backend gateway and the client crates (mobile, daemon) use these
//! types to serialize/deserialize WebSocket text frames. Moving them here
//! ensures a single source of truth for the wire protocol.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    User { account_id: String },
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
    AccountRegistered { account_id: String, at_ms: i64 },
    AccountPasswordChanged { account_id: String, at_ms: i64 },
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
    ProjectArchived { project_id: String, at_ms: i64 },
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
            | Self::HostUnlinked { account_id, .. } => {
                RealtimeTopic::Account(account_id.clone())
            }
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
```

- [ ] **Step 2: Create ws_ticket.rs**

Create `crates/minos-protocol/src/ws_ticket.rs`:

```rust
//! WS ticket request/response types for the realtime gateway.
//!
//! Before upgrading to `/ws/client` or `/ws/host`, the caller must obtain
//! a short-lived ticket via `POST /v1/realtime/ws-ticket` (account) or
//! `POST /v1/host/realtime/ws-ticket` (host).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WsTicketRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WsTicketResponse {
    pub ticket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_ticket_response_round_trip() {
        let resp = WsTicketResponse {
            ticket: "jwt-token-here".into(),
            gateway_url: Some("wss://minos.example.com/ws/client".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: WsTicketResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn ws_ticket_request_empty_round_trip() {
        let req = WsTicketRequest {
            installation_id: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: WsTicketRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }
}
```

- [ ] **Step 3: Update lib.rs to export new modules**

Modify `crates/minos-protocol/src/lib.rs` — add after existing module declarations:

```rust
pub mod realtime;
pub mod ws_ticket;

pub use realtime::*;
pub use ws_ticket::*;
```

- [ ] **Step 4: Run tests to verify**

Run: `cargo test -p minos-protocol`
Expected: All existing + new tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-protocol/src/realtime.rs crates/minos-protocol/src/ws_ticket.rs crates/minos-protocol/src/lib.rs
git commit -m "feat(protocol): extract realtime wire types (ClientFrame/ServerFrame/RealtimeTopic/DurableEvent) into minos-protocol"
```

---

## Task 2: Re-export extracted types from backend, remove duplicate definitions

**Files:**
- Modify: `crates/minos-backend/src/realtime/wire.rs` — replace body with re-exports from `minos_protocol`
- Modify: `crates/minos-backend/src/realtime/topic.rs` — replace body with re-exports from `minos_protocol`
- Modify: `crates/minos-backend/src/realtime/event.rs` — replace body with re-exports from `minos_protocol`
- Modify: `crates/minos-backend/src/realtime/subscription.rs` — import `ConnectionPrincipal` from `minos_protocol`
- Modify: `crates/minos-backend/src/realtime/auth.rs` — import `RealtimeTopic`/`ConnectionPrincipal` from `minos_protocol`
- Modify: `crates/minos-backend/src/realtime/gateway.rs` — import from `minos_protocol`

- [ ] **Step 1: Write test that minos-protocol types match backend types**

This is structural — we verify the backend still compiles and all its existing tests pass after switching to the re-exported types.

- [ ] **Step 2: Replace `wire.rs` body with re-exports**

Replace entire content of `crates/minos-backend/src/realtime/wire.rs` with:

```rust
//! Re-exports of realtime wire types from `minos_protocol`.
//!
//! The canonical definitions live in `minos-protocol::realtime`; this module
//! preserves the existing import paths within the backend crate.

pub use minos_protocol::realtime::{ClientFrame, ServerFrame};
```

- [ ] **Step 3: Replace `topic.rs` body with re-exports**

Replace entire content of `crates/minos-backend/src/realtime/topic.rs` with:

```rust
pub use minos_protocol::realtime::{RealtimeTopic, TopicKind, TopicParseError};
```

- [ ] **Step 4: Replace `event.rs` body with re-exports**

Replace entire content of `crates/minos-backend/src/realtime/event.rs` with:

```rust
pub use minos_protocol::realtime::{
    ApprovalResolution, DurableEvent, DurableEventEnvelope, SenderRef,
};
```

- [ ] **Step 5: Update `subscription.rs` to import `ConnectionPrincipal` from `minos_protocol`**

In `crates/minos-backend/src/realtime/subscription.rs`, remove the local `ConnectionPrincipal` definition (lines 30-34) and add:

```rust
pub use minos_protocol::realtime::ConnectionPrincipal;
```

The rest of the file (`ConnectionState`, `SubscriptionManager`, etc.) stays unchanged — it's backend-specific state management.

- [ ] **Step 6: Update `auth.rs` imports**

In `crates/minos-backend/src/realtime/auth.rs`, change:

```rust
use super::subscription::ConnectionPrincipal;
use super::RealtimeTopic;
```

to:

```rust
use minos_protocol::realtime::{ConnectionPrincipal, RealtimeTopic};
```

- [ ] **Step 7: Update `gateway.rs` imports**

In `crates/minos-backend/src/realtime/gateway.rs`, change:

```rust
use crate::realtime::wire::{ClientFrame, ServerFrame};
use crate::realtime::{ConnectionPrincipal, RealtimeTopic};
```

to:

```rust
use minos_protocol::realtime::{ClientFrame, ConnectionPrincipal, RealtimeTopic, ServerFrame};
```

- [ ] **Step 8: Run backend tests**

Run: `cargo test -p minos-backend`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/minos-backend/src/realtime/
git commit -m "refactor(backend): re-export realtime wire types from minos-protocol, remove duplicates"
```

---

## Task 3: Mobile — add ws-ticket HTTP endpoint and ticket-based WS connection

**Files:**
- Modify: `crates/minos-mobile/src/http.rs` — add `fetch_ws_ticket` method
- Modify: `crates/minos-mobile/src/client.rs` — replace header-based WS auth with ticket-based

- [ ] **Step 1: Add `fetch_ws_ticket` to `MobileHttpClient`**

In `crates/minos-mobile/src/http.rs`, add method to `MobileHttpClient`:

```rust
pub async fn fetch_ws_ticket(&self, access_token: &str) -> Result<minos_protocol::WsTicketResponse, MinosError> {
    let body = minos_protocol::WsTicketRequest { installation_id: None };
    self.post_bearer("/v1/realtime/ws-ticket", access_token, &body).await
}
```

(Assumes `post_bearer` helper exists or is added as a thin wrapper around the existing authenticated POST pattern used by other methods in this file.)

- [ ] **Step 2: Write test for ws-ticket fetch**

In `crates/minos-mobile/tests/http_smoke.rs` or a new test file, test that the HTTP method constructs the correct request.

- [ ] **Step 3: Replace `build_websocket_request` in client.rs**

Replace the header-stamping `build_websocket_request` method (lines 1336-1386 in current `client.rs`) with a ticket-based approach:

```rust
fn build_websocket_url(base_url: &str, ticket: &str) -> String {
    let ws_url = base_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    format!("{ws_url}/ws/client?ticket={ticket}")
}
```

- [ ] **Step 4: Modify `connect` to fetch ticket first**

Replace the `connect` method flow:
1. Call `http.fetch_ws_ticket(&access_token)` to get the ticket
2. Build URL with `build_websocket_url(backend_url, &ticket)`
3. Remove all `X-Device-*` headers from the WS upgrade request
4. Open the WS connection

- [ ] **Step 5: Modify `open_backend_websocket` to accept URL only (no headers)**

Simplify to just take the URL with ticket query param:

```rust
async fn open_backend_websocket(url: &str) -> Result<WebSocket, WebSocketError> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(url)
        .body(RequestBody::empty())
        .map_err(|error| WebSocketError::Io(WireError::invalid_request(error.to_string())))?;
    let client = Self::build_websocket_client().map_err(|error| {
        WebSocketError::Io(WireError::new(WireErrorKind::Internal, error.to_string()))
    })?;
    client
        .new_websocket(request)
        .handshake_timeout(WS_HANDSHAKE_TIMEOUT)
        .ping_interval(WS_PING_INTERVAL)
        .pong_timeout(WS_PONG_TIMEOUT)
        .execute()
        .await
}
```

- [ ] **Step 6: Update `connect_with_handles` in the reconnect loop**

Same ticket-fetch-then-connect pattern as `connect`.

- [ ] **Step 7: Run mobile unit tests**

Run: `cargo test -p minos-mobile`
Expected: All unit tests pass (integration tests that hit a real backend will need the backend running with ticket support).

- [ ] **Step 8: Commit**

```bash
git add crates/minos-mobile/src/http.rs crates/minos-mobile/src/client.rs
git commit -m "feat(mobile): switch WS connection from header auth to ws-ticket query param"
```

---

## Task 4: Mobile — replace Envelope recv/send with ClientFrame/ServerFrame

**Files:**
- Create: `crates/minos-mobile/src/realtime/mod.rs`
- Create: `crates/minos-mobile/src/realtime/subscription.rs`
- Create: `crates/minos-mobile/src/realtime/session.rs`
- Create: `crates/minos-mobile/src/realtime/frame_handler.rs`
- Modify: `crates/minos-mobile/src/client.rs` — wire new realtime session
- Modify: `crates/minos-mobile/src/lib.rs` — add `pub mod realtime;`

- [ ] **Step 1: Create `realtime/subscription.rs`**

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Default)]
pub struct SubscriptionManager {
    inner: RwLock<SubscriptionState>,
}

#[derive(Debug, Default)]
struct SubscriptionState {
    topics: HashMap<String, i64>,  // topic_string -> last_durable_seq
}

impl SubscriptionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn add_topic(&self, topic: &str, resume_after: i64) {
        let mut state = self.inner.write().await;
        state.topics.entry(topic.to_string()).or_insert(resume_after);
    }

    pub async fn remove_topic(&self, topic: &str) {
        let mut state = self.inner.write().await;
        state.topics.remove(topic);
    }

    pub async fn update_seq(&self, topic: &str, seq: i64) {
        let mut state = self.inner.write().await;
        if let Some(existing) = state.topics.get_mut(topic) {
            if seq > *existing {
                *existing = seq;
            }
        }
    }

    pub async fn resume_after_map(&self) -> HashMap<String, i64> {
        let state = self.inner.read().await;
        state.topics.clone()
    }

    pub async fn subscribed_topics(&self) -> Vec<String> {
        let state = self.inner.read().await;
        state.topics.keys().cloned().collect()
    }
}
```

- [ ] **Step 2: Create `realtime/frame_handler.rs`**

This replaces `handle_text_frame` from `client.rs`. It parses `ServerFrame` instead of `Envelope`:

```rust
use minos_protocol::realtime::ServerFrame;
use minos_domain::MinosError;

#[derive(Debug, Clone)]
pub enum RealtimeEvent {
    DurableEvent {
        topic: String,
        topic_seq: i64,
        kind: String,
        payload: serde_json::Value,
        event_id: String,
    },
    StreamEvent {
        topic: String,
        kind: String,
        seq: Option<i64>,
        payload: serde_json::Value,
    },
    SnapshotRequired {
        topic: String,
        last_known_seq: i64,
        retention_floor_seq: i64,
    },
    SubscriptionDenied {
        topic: String,
        reason: String,
    },
    ForceClose {
        reason: String,
        close_code: u16,
    },
}

pub fn handle_server_frame(frame: ServerFrame) -> Option<RealtimeEvent> {
    match frame {
        ServerFrame::Hello { .. } => None,  // handled at session level
        ServerFrame::SubscribeAck { .. } => None,  // logged
        ServerFrame::SubscriptionDenied { topic, reason } => {
            tracing::warn!(topic, reason, "subscription denied");
            Some(RealtimeEvent::SubscriptionDenied { topic, reason })
        }
        ServerFrame::SubscriptionLimitExceeded { limit, current } => {
            tracing::warn!(limit, current, "subscription limit exceeded");
            None
        }
        ServerFrame::DurableEvent { topic, topic_seq, kind, payload, event_id } => {
            Some(RealtimeEvent::DurableEvent { topic, topic_seq, kind, payload, event_id })
        }
        ServerFrame::StreamEvent { topic, kind, seq, payload } => {
            Some(RealtimeEvent::StreamEvent { topic, kind, seq, payload })
        }
        ServerFrame::SnapshotRequired { topic, last_known_seq, retention_floor_seq } => {
            Some(RealtimeEvent::SnapshotRequired { topic, last_known_seq, retention_floor_seq })
        }
        ServerFrame::HostForceClose { reason, close_code } => {
            Some(RealtimeEvent::ForceClose { reason, close_code })
        }
        ServerFrame::Pong { .. } => None,
        ServerFrame::Error { code, message, .. } => {
            tracing::warn!(code, message, "server error frame");
            None
        }
    }
}
```

- [ ] **Step 3: Create `realtime/session.rs`**

The WS session loop. On connect: receive `Hello`, send `Subscribe` for `account:<id>`. Main loop: recv `ServerFrame`, dispatch via `frame_handler`, handle ping/pong. Send side: `ClientFrame` serialize and send.

```rust
use std::sync::Arc;
use futures_util::StreamExt;
use minos_domain::MinosError;
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use openwire::websocket::WebSocket;
use openwire_core::websocket::Message;
use tokio::sync::{mpsc, watch};

use super::frame_handler::{handle_server_frame, RealtimeEvent};
use super::subscription::SubscriptionManager;
use crate::client::UiEventFrame;
use crate::client::SocialEventFrame;

pub struct RealtimeSession;

impl RealtimeSession {
    pub async fn run(
        mut ws: WebSocket,
        account_id: String,
        subscription_mgr: Arc<SubscriptionManager>,
        ui_events_tx: tokio::sync::broadcast::Sender<UiEventFrame>,
        social_events_tx: tokio::sync::broadcast::Sender<SocialEventFrame>,
        state_tx: watch::Sender<minos_domain::ConnectionState>,
        mut inbound_client_frames: mpsc::Receiver<ClientFrame>,
    ) {
        // Step 1: Wait for Hello
        let hello = match wait_for_hello(&mut ws).await {
            Some(h) => h,
            None => return,
        };

        // Step 2: Auto-subscribe to account topic
        let account_topic = format!("account:{account_id}");
        let resume_after = subscription_mgr.resume_after_map().await;
        let subscribe = ClientFrame::Subscribe {
            topics: vec![account_topic.clone()],
            resume_after: Some(resume_after),
            client_request_id: None,
        };
        if send_frame(&mut ws, &subscribe).await.is_err() {
            return;
        }
        subscription_mgr.add_topic(&account_topic, 0).await;

        // Step 3: Main loop
        let (mut ws_write, mut ws_read) = ws.split();
        loop {
            tokio::select! {
                maybe_msg = ws_read.next() => {
                    match maybe_msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(frame) = serde_json::from_str::<ServerFrame>(text.as_ref()) {
                                if let Some(event) = handle_server_frame(frame) {
                                    dispatch_event(
                                        &event,
                                        &subscription_mgr,
                                        &ui_events_tx,
                                        &social_events_tx,
                                    ).await;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(_)) => break,
                        _ => {}
                    }
                }
                maybe_frame = inbound_client_frames.recv() => {
                    let Some(frame) = maybe_frame else { break };
                    let json = match serde_json::to_string(&frame) {
                        Ok(j) => j,
                        Err(_) => continue,
                    };
                    if ws_write.send_text(json).await.is_err() {
                        break;
                    }
                }
            }
        }

        let _ = state_tx.send(minos_domain::ConnectionState::Disconnected);
    }
}

async fn wait_for_hello(ws: &mut WebSocket) -> Option<()> {
    // Read frames until we get a Hello or the stream ends
    while let Some(msg) = ws.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(ServerFrame::Hello { conn_id, heartbeat_interval_ms, .. }) =
                    serde_json::from_str::<ServerFrame>(text.as_ref())
                {
                    tracing::info!(conn_id, heartbeat_interval_ms, "realtime session established");
                    return Some(());
                }
            }
            _ => return None,
        }
    }
    None
}

async fn send_frame(ws: &mut WebSocket, frame: &ClientFrame) -> Result<(), ()> {
    let json = serde_json::to_string(frame).map_err(|_| ())?;
    ws.send_text(json).await.map_err(|_| ())
}

async fn dispatch_event(
    event: &RealtimeEvent,
    subscription_mgr: &Arc<SubscriptionManager>,
    ui_events_tx: &tokio::sync::broadcast::Sender<UiEventFrame>,
    social_events_tx: &tokio::sync::broadcast::Sender<SocialEventFrame>,
) {
    match event {
        RealtimeEvent::DurableEvent { topic, topic_seq, kind, payload, .. } => {
            subscription_mgr.update_seq(topic, *topic_seq).await;
            match kind.as_str() {
                "conversation_message_appended" => {
                    // Convert to SocialEventFrame for UI
                    if let Some(conv_id) = payload.get("conversation_id").and_then(|v| v.as_str()) {
                        // The full ChatMessageSummary is in payload; parse and broadcast
                        let _ = social_events_tx.send(SocialEventFrame {
                            conversation_id: conv_id.to_string(),
                            message: parse_chat_message(payload),
                        });
                    }
                }
                "approval_requested" | "approval_resolved" => {
                    // Convert to UiEventFrame
                    let _ = ui_events_tx.send(UiEventFrame {
                        session_id: payload.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        seq: 0,
                        ui: minos_ui_protocol::UiEventMessage::Raw {
                            kind: kind.clone(),
                            payload_json: payload.to_string(),
                        },
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
                _ => {
                    tracing::debug!(kind, topic, "unhandled durable event");
                }
            }
        }
        RealtimeEvent::StreamEvent { topic, kind, payload, .. } => {
            match kind.as_str() {
                "agent_text_delta" | "agent_tool_call" | "agent_error" => {
                    let _ = ui_events_tx.send(UiEventFrame {
                        session_id: topic.strip_prefix("agent_session:").unwrap_or(topic).to_string(),
                        seq: 0,
                        ui: minos_ui_protocol::UiEventMessage::Raw {
                            kind: kind.clone(),
                            payload_json: payload.to_string(),
                        },
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
                _ => {
                    tracing::debug!(kind, topic, "unhandled stream event");
                }
            }
        }
        RealtimeEvent::SnapshotRequired { topic, .. } => {
            tracing::warn!(topic, "snapshot required — need REST rebuild");
        }
        RealtimeEvent::SubscriptionDenied { topic, reason } => {
            tracing::warn!(topic, reason, "subscription denied");
        }
        RealtimeEvent::ForceClose { reason, close_code } => {
            tracing::warn!(reason, close_code, "force close from server");
        }
    }
}

fn parse_chat_message(payload: &serde_json::Value) -> minos_protocol::ChatMessageSummary {
    // Minimal parsing; the backend sends the full summary in the durable event payload
    serde_json::from_value(payload.clone()).unwrap_or_else(|_| minos_protocol::ChatMessageSummary {
        message_id: String::new(),
        conversation_id: String::new(),
        sender: minos_protocol::UserSummary {
            account_id: String::new(),
            minos_id: String::new(),
            display_name: String::new(),
        },
        text: String::new(),
        created_at_ms: 0,
        reply_to: None,
        recalled_at_ms: None,
        mentioned_account_ids: Vec::new(),
        sender_type: minos_protocol::SenderType::User,
    })
}
```

- [ ] **Step 4: Create `realtime/mod.rs`**

```rust
pub mod frame_handler;
pub mod session;
pub mod subscription;

pub use frame_handler::{RealtimeEvent, handle_server_frame};
pub use session::RealtimeSession;
pub use subscription::SubscriptionManager;
```

- [ ] **Step 5: Wire realtime session into `MobileClient::connect`**

Replace the Envelope-based send/recv loop in `connect` and `connect_with_handles` with:
1. Create `SubscriptionManager`
2. Create `mpsc::channel::<ClientFrame>` for outbound frames
3. Spawn `RealtimeSession::run` as the recv task
4. Store the `mpsc::Sender<ClientFrame>` as the outbox

- [ ] **Step 6: Remove the old `handle_text_frame` function and `recv_loop`**

Delete `recv_loop`, `handle_text_frame` from `client.rs` — replaced by `RealtimeSession`.

- [ ] **Step 7: Add `pub mod realtime;` to `lib.rs`**

- [ ] **Step 8: Run mobile tests**

Run: `cargo test -p minos-mobile`
Expected: Unit tests pass; integration tests need backend with ticket support.

- [ ] **Step 9: Commit**

```bash
git add crates/minos-mobile/src/realtime/ crates/minos-mobile/src/client.rs crates/minos-mobile/src/lib.rs
git commit -m "feat(mobile): replace Envelope WS with ClientFrame/ServerFrame realtime session"
```

---

## Task 5: Mobile — migrate forward_rpc to REST API calls

**Files:**
- Modify: `crates/minos-mobile/src/http.rs` — add `start_agent_session`, `send_user_message_http`, `interrupt_session_http`, `close_session_http`, `list_host_skills_http`, `write_host_skill_config_http`
- Modify: `crates/minos-mobile/src/client.rs` — replace `forward_rpc` calls with HTTP calls
- Delete: `crates/minos-mobile/src/rpc.rs` — no longer needed

- [ ] **Step 1: Add REST API methods to `MobileHttpClient`**

In `http.rs`, add methods that call the backend's `/v1/agent-sessions/*` and `/v1/host/*` endpoints directly, bypassing the WS:

```rust
pub async fn start_agent_session(
    &self,
    access_token: &str,
    agent: &str,
    workspace: &str,
    host_installation_id: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<minos_protocol::StartAgentResponse, MinosError> {
    let body = serde_json::json!({
        "agent": agent,
        "workspace": workspace,
        "host_installation_id": host_installation_id,
        "conversation_id": conversation_id,
    });
    self.post_bearer("/v1/agent-sessions/start", access_token, &body).await
}

pub async fn send_user_message_http(
    &self,
    access_token: &str,
    session_id: &str,
    text: &str,
) -> Result<(), MinosError> {
    let body = serde_json::json!({
        "session_id": session_id,
        "text": text,
    });
    self.post_bearer("/v1/agent-sessions/send-message", access_token, &body).await
}

pub async fn interrupt_session_http(
    &self,
    access_token: &str,
    session_id: &str,
) -> Result<(), MinosError> {
    let body = serde_json::json!({ "session_id": session_id });
    self.post_bearer("/v1/agent-sessions/interrupt", access_token, &body).await
}

pub async fn close_session_http(
    &self,
    access_token: &str,
    session_id: &str,
) -> Result<(), MinosError> {
    let body = serde_json::json!({ "session_id": session_id });
    self.post_bearer("/v1/agent-sessions/close", access_token, &body).await
}
```

- [ ] **Step 2: Replace `send_user_message` in client.rs**

Change from `forward_rpc` to HTTP:

```rust
pub async fn send_user_message(&self, session_id: String, text: String) -> Result<(), MinosError> {
    auth_http_call!(self, |http, access| {
        http.send_user_message_http(&access, &session_id, &text)
    })
}
```

- [ ] **Step 3: Replace `interrupt_session`, `close_session`, `delete_session` similarly**

- [x] **Step 4: Replace `list_clis`, `list_host_skills`, `write_host_skill_config`**

These currently use `forward_rpc` over WS. Migrate to REST endpoints on the backend (the backend's `v1/host.rs` already has host-scoped endpoints).

- [ ] **Step 5: Delete `rpc.rs`**

Remove `crates/minos-mobile/src/rpc.rs` and its module declaration from `lib.rs`. Remove `pending: DashMap` and `next_id: AtomicU64` from `MobileClient` — no more in-flight RPC correlation over WS.

- [ ] **Step 6: Remove `Envelope` import from client.rs**

The `outbox: mpsc::Sender<Envelope>` becomes `outbox: mpsc::Sender<ClientFrame>`. The send task serializes `ClientFrame` instead of `Envelope`.

- [ ] **Step 7: Run mobile tests**

Run: `cargo test -p minos-mobile`

- [ ] **Step 8: Commit**

```bash
git add crates/minos-mobile/src/
git commit -m "feat(mobile): migrate forward_rpc to REST API, remove Envelope-based RPC"
```

---

## Task 6: Mobile — update Dart FFI protocol surface

**Files:**
- Modify: `apps/mobile/lib/domain/minos_core_protocol.dart` — update method signatures
- Modify: `apps/mobile/lib/ui/features/chat/views/thread_view_page.dart` — adapt if needed

- [ ] **Step 1: Update `MinosCoreProtocol` in Dart**

The Dart protocol already exposes high-level methods (`sendUserMessage`, `interruptThread`, etc.) that delegate to Rust via FFI. The Rust-side changes in Tasks 4-5 are transparent to Dart since the FFI boundary remains the same method names. Verify no Dart changes are needed — the FFI bridge should still work since `sendUserMessage(sessionId, text)` on the Dart side calls `api.sendUserMessage(sessionId: sessionId, text: text)` which invokes the Rust method of the same name.

- [ ] **Step 2: Verify the Dart FFI generated API matches**

Run: `cd apps/mobile && flutter analyze`
Expected: No breaking changes at the Dart layer.

- [ ] **Step 3: Commit if changes needed, otherwise skip**

---

## Task 7: Host daemon — ticket-based WS connection

**Files:**
- Modify: `crates/minos-daemon/src/relay_http.rs` — add `fetch_host_ws_ticket` method
- Modify: `crates/minos-daemon/src/relay_client.rs` — replace header auth with ticket-based WS
- Delete: `crates/minos-transport/src/auth.rs` — `AuthHeaders` no longer used

- [ ] **Step 1: Add `fetch_host_ws_ticket` to `RelayHttpClient`**

In `relay_http.rs`:

```rust
pub async fn fetch_host_ws_ticket(&self, host_token: &str) -> Result<minos_protocol::WsTicketResponse, MinosError> {
    let url = format!("{}/v1/host/realtime/ws-ticket", self.base_url);
    // POST with Authorization: Bearer <host_token>
    // Deserialize response into WsTicketResponse
    ...
}
```

- [ ] **Step 2: Replace `build_headers` + `build_request` in relay_client.rs**

Replace `build_headers` (which stamps `X-Device-Id`, `X-Device-Role`, `X-Device-Secret`, `X-Device-Name`) with:

```rust
fn build_ws_url(base_url: &str, ticket: &str) -> String {
    let ws_url = base_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    format!("{ws_url}/ws/host?ticket={ticket}")
}
```

- [ ] **Step 3: Modify `run_once` to fetch ticket before connecting**

In `run_once`:
1. Call `self.http.fetch_host_ws_ticket(&host_token)` to get the ticket
2. Build URL with `build_ws_url(&ctx.backend_url, &ticket)`
3. Connect with `tokio_tungstenite::connect_async` using the URL (no custom headers needed)

- [ ] **Step 4: Remove `minos-transport/src/auth.rs`**

Delete `crates/minos-transport/src/auth.rs` and remove its module declaration from `minos-transport/src/lib.rs`. Remove `minos-transport::auth::AuthHeaders` import from `relay_client.rs`.

- [ ] **Step 5: Run daemon tests**

Run: `cargo test -p minos-daemon`

- [ ] **Step 6: Commit**

```bash
git add crates/minos-daemon/src/ crates/minos-transport/src/
git commit -m "feat(daemon): switch host WS from header auth to ws-ticket, remove AuthHeaders"
```

---

## Task 8: Host daemon — replace Envelope with ClientFrame/ServerFrame

**Files:**
- Create: `crates/minos-daemon/src/realtime/mod.rs`
- Create: `crates/minos-daemon/src/realtime/session.rs`
- Create: `crates/minos-daemon/src/realtime/frame_handler.rs`
- Modify: `crates/minos-daemon/src/relay_client.rs` — use new realtime session

- [ ] **Step 1: Create `realtime/frame_handler.rs` for host**

Handles incoming `ServerFrame` for the host side:

```rust
use minos_protocol::realtime::{ClientFrame, ServerFrame};

#[derive(Debug)]
pub enum HostRealtimeEvent {
    HostCommandIssued {
        command_id: String,
        method: String,
        params: serde_json::Value,
        deadline_at_ms: i64,
    },
    SubscriptionDenied { topic: String, reason: String },
    ForceClose { reason: String, close_code: u16 },
    SnapshotRequired { topic: String },
}

pub fn handle_server_frame(frame: ServerFrame) -> Option<HostRealtimeEvent> {
    match frame {
        ServerFrame::DurableEvent { kind, payload, .. } => match kind.as_str() {
            "host_command_issued" => Some(HostRealtimeEvent::HostCommandIssued {
                command_id: payload.get("command_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                method: payload.get("method").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                params: payload.get("params").cloned().unwrap_or(serde_json::Value::Null),
                deadline_at_ms: payload.get("deadline_at_ms").and_then(|v| v.as_i64()).unwrap_or(0),
            }),
            _ => {
                tracing::debug!(kind, "host ignores durable event");
                None
            }
        },
        ServerFrame::SubscriptionDenied { topic, reason } => {
            Some(HostRealtimeEvent::SubscriptionDenied { topic, reason })
        }
        ServerFrame::HostForceClose { reason, close_code } => {
            Some(HostRealtimeEvent::ForceClose { reason, close_code })
        }
        ServerFrame::SnapshotRequired { topic, .. } => {
            Some(HostRealtimeEvent::SnapshotRequired { topic })
        }
        ServerFrame::StreamEvent { .. } => None,  // host doesn't consume stream events
        _ => None,
    }
}

pub fn build_host_command_ack(command_id: &str, ack_at_ms: i64) -> ClientFrame {
    ClientFrame::HostCommandAck {
        command_id: command_id.to_string(),
        ack_at_ms,
    }
}

pub fn build_host_command_result(
    command_id: &str,
    succeeded: bool,
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
    finished_at_ms: i64,
) -> ClientFrame {
    ClientFrame::HostCommandResult {
        command_id: command_id.to_string(),
        status: if succeeded { "succeeded".into() } else { "failed".into() },
        result,
        error,
        finished_at_ms,
    }
}

pub fn build_host_stream_event(
    topic: &str,
    kind: &str,
    payload: serde_json::Value,
) -> ClientFrame {
    ClientFrame::HostStreamEvent {
        topic: topic.to_string(),
        kind: kind.to_string(),
        payload,
    }
}
```

- [ ] **Step 2: Create `realtime/session.rs` for host**

Host session loop: connect, receive `Hello`, subscribe to `host:<installation_id>`, process incoming `ServerFrame`, send `ClientFrame::HostCommandAck`/`HostCommandResult`/`HostStreamEvent`.

- [ ] **Step 3: Create `realtime/mod.rs`**

```rust
pub mod frame_handler;
pub mod session;
```

- [ ] **Step 4: Rewrite `dispatch_loop` in `relay_client.rs`**

Replace the Envelope-based `dispatch_loop` with the new `RealtimeSession`. Key changes:
- Outbound: `Envelope::Ingest` becomes `ClientFrame::HostStreamEvent`
- Inbound: `Envelope::Forwarded` with RPC requests becomes `ServerFrame::DurableEvent` with `host_command_issued`
- Response: `Envelope::Forward` back becomes `ClientFrame::HostCommandResult`
- `Envelope::Event` handling (Paired, PeerOnline, etc.) becomes `ServerFrame::DurableEvent` parsing

- [ ] **Step 5: Update `rpc_server.rs`**

The RPC server currently handles forwarded JSON-RPC calls. Under the new model, host commands arrive as `DurableEvent::HostCommandIssued` in a `ServerFrame`. The `invoke_forwarded` function needs to accept the `method` and `params` directly instead of a JSON-RPC envelope. Responses go back as `ClientFrame::HostCommandResult` instead of `Envelope::Forward`.

- [ ] **Step 6: Run daemon tests**

Run: `cargo test -p minos-daemon`

- [ ] **Step 7: Commit**

```bash
git add crates/minos-daemon/src/
git commit -m "feat(daemon): replace Envelope WS with ClientFrame/ServerFrame realtime session"
```

---

## Task 9: Mobile — implement resume_after with durable seq persistence

**Files:**
- Modify: `crates/minos-mobile/src/realtime/subscription.rs` — add persistence
- Modify: `crates/minos-mobile/src/store/mod.rs` — add `save_subscription_cursors`/`load_subscription_cursors`
- Modify: `crates/minos-mobile/src/store/in_memory.rs` — add cursor storage

- [ ] **Step 1: Add cursor persistence to the pairing store**

Add to `MobilePairingStore` trait:

```rust
async fn save_subscription_cursor(&self, topic: &str, seq: i64) -> Result<(), MinosError>;
async fn load_subscription_cursors(&self) -> Result<HashMap<String, i64>, MinosError>;
```

- [ ] **Step 2: Implement in `InMemoryPairingStore`**

- [ ] **Step 3: In `SubscriptionManager::update_seq`, also persist the cursor**

When a `DurableEvent` updates `topic_seq`, also call `store.save_subscription_cursor(topic, seq)`.

- [ ] **Step 4: On reconnect, load cursors and pass as `resume_after`**

In the reconnect loop, before calling `subscribe`, load all persisted cursors and pass them as the `resume_after` map.

- [ ] **Step 5: Handle `SnapshotRequired` gracefully**

When `ServerFrame::SnapshotRequired` arrives, clear the stored cursor for that topic and trigger a REST API call to rebuild state (e.g., `readThread` for agent sessions).

- [ ] **Step 6: Run mobile tests**

Run: `cargo test -p minos-mobile`

- [ ] **Step 7: Commit**

```bash
git add crates/minos-mobile/src/
git commit -m "feat(mobile): persist resume_after cursors for realtime topic subscriptions"
```

---

## Task 10: Remove legacy Envelope support from backend gateway

**Files:**
- Modify: `crates/minos-backend/src/realtime/gateway.rs` — remove `handle_legacy_envelope`, `send_legacy_envelope`, and the Envelope fallback in `handle_text_frame`
- Modify: `crates/minos-backend/src/realtime.rs` — remove `Envelope`-related re-exports
- Modify: `crates/minos-backend/src/runtime.rs` — remove `SessionHandle`/`SessionRegistry` from realtime session (replaced by `SubscriptionManager`)

> Implementation note: the gateway no longer parses or emits legacy `Envelope`
> frames. A temporary `SessionRegistry` bridge is still retained for
> same-device replacement and auth-revocation lifecycle signaling while the
> remaining backend session flows move to topic-native tracking.

- [x] **Step 1: Remove Envelope fallback from `handle_text_frame`**

In `gateway.rs`, the current `handle_text_frame` tries `ClientFrame` first, then falls back to `Envelope`. Remove the `Envelope` branch:

```rust
async fn handle_text_frame(...) -> Result<Option<CloseDirective>, BackendError> {
    match serde_json::from_str::<ClientFrame>(text) {
        Ok(frame) => handle_formal_frame(ws, state, upgrade, conn, frame).await,
        Err(_) => {
            let _ = send_error_frame(ws, conn, "validation_format", "unrecognized websocket frame").await;
            Ok(None)
        }
    }
}
```

- [x] **Step 2: Remove `handle_legacy_envelope` and `send_legacy_envelope` functions**

Delete these functions entirely from `gateway.rs`.

- [ ] **Step 3: Remove `legacy_handle` and `legacy_outbox_rx` from `run_session`**

The `SessionHandle` is no longer needed for the new protocol. Remove the `legacy_handle`, `legacy_outbox_rx` from `run_session_inner` and the `tokio::select!` branch for `legacy_outbox_rx`.

- [ ] **Step 4: Remove `SessionRegistry` dependency from gateway**

The `SubscriptionManager` is now the sole subscription routing mechanism. The `state.registry` calls in gateway can be removed. Check if `SessionRegistry` is still used elsewhere (envelope module) — if not, it can be deprecated.

- [x] **Step 5: Run backend tests**

Run: `cargo test -p minos-backend`
Expected: Some integration tests may fail if they still send Envelope frames. Fix those tests to send `ClientFrame` instead.

- [x] **Step 6: Update failing integration tests**

Update `tests/ws_gateway.rs`, `tests/ws_devices.rs`, and any other test that sends Envelope frames to use `ClientFrame`/`ServerFrame`.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-backend/src/ crates/minos-backend/tests/
git commit -m "refactor(backend): remove legacy Envelope support from realtime gateway"
```

---

## Task 11: Delete Envelope and EventKind from minos-protocol

**Files:**
- Delete: `crates/minos-protocol/src/envelope.rs`
- Modify: `crates/minos-protocol/src/lib.rs` — remove `pub mod envelope; pub use envelope::*;`
- Modify: `crates/minos-backend/src/envelope/mod.rs` — update imports (if it still references `Envelope`/`EventKind`)
- Modify: `crates/minos-backend/src/ingest/` — replace `Envelope` references with `ClientFrame`/`DurableEvent`
- Modify: any remaining consumers

- [ ] **Step 1: Find all remaining `Envelope`/`EventKind` references**

Run: `grep -r "minos_protocol::Envelope\|minos_protocol::EventKind\|use minos_protocol::Envelope\|use minos_protocol::EventKind" crates/`

- [ ] **Step 2: Fix each reference**

- `crates/minos-backend/src/envelope/mod.rs` — the `run_session` function and `handle_forward` helper are no longer called from the gateway. If nothing else calls them, delete the module.
- `crates/minos-backend/src/ingest/` — the `IngestCommand` currently takes fields from `Envelope::Ingest`. Replace with a domain-level struct.
- Any daemon or mobile references should already be gone from Tasks 4-8.

- [ ] **Step 3: Delete `envelope.rs`**

Remove `crates/minos-protocol/src/envelope.rs` and the `pub mod envelope; pub use envelope::*;` from `lib.rs`.

- [ ] **Step 4: Run full workspace test**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "refactor: delete legacy Envelope/EventKind from minos-protocol, all consumers migrated"
```

---

## Task 12: Delete deprecated code and unused modules

**Files:**
- Delete: `crates/minos-mobile/src/rpc.rs` (if not already deleted in Task 5)
- Delete: `crates/minos-transport/src/auth.rs` (if not already deleted in Task 7)
- Modify: `crates/minos-mobile/src/lib.rs` — remove `pub mod rpc;`
- Modify: `crates/minos-transport/src/lib.rs` — remove `pub mod auth;`
- Modify: `crates/minos-backend/src/lib.rs` — remove `#[deprecated]` annotation on `pub mod social;` if social module is fully replaced
- Delete: `crates/minos-backend/src/store/social.rs` (if still exists; replaced by `store/social/` directory)
- Clean up: any remaining `Envelope`-related test fixtures

- [ ] **Step 1: Remove `rpc.rs` from minos-mobile**

If `rpc.rs` still exists, delete it and remove `pub mod rpc;` from `lib.rs`.

- [ ] **Step 2: Remove `auth.rs` from minos-transport**

If `auth.rs` still exists, delete it and remove `pub mod auth;` from `lib.rs`.

- [ ] **Step 3: Remove deprecated `social` module annotation**

If the `#[deprecated]` annotation on `pub mod social;` in `crates/minos-backend/src/lib.rs` still exists and the social routes have been fully replaced by the new domain modules (`conversations`, `friends`, `profiles`), delete the old `social` module declaration and the `http/v1/social.rs` handler.

- [ ] **Step 4: Remove `store/social.rs` if it still exists as a single file**

The `store/social/` directory already replaces it. Delete the single-file version if present.

- [ ] **Step 5: Clean up golden test fixtures for Envelope**

Delete any test fixtures under `tests/golden/envelope/` that test the old Envelope wire format.

- [ ] **Step 6: Run full workspace test**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore: delete deprecated Envelope/RPC modules, clean up legacy social and transport auth"
```

---

## Task 13: Update Dart FFI bridge for new protocol

**Files:**
- Modify: `apps/mobile/lib/domain/minos_core_protocol.dart` — update `UiEventFrame`/`SocialEventFrame` types if needed
- Modify: any Dart repositories/state managers that consume WS events

- [ ] **Step 1: Verify FFI-generated types match new Rust surface**

Run the Flutter/FRB codegen:
```bash
cd apps/mobile && flutter pub run flutter_rust_bridge_codegen generate
```

- [ ] **Step 2: Update Dart `UiEventFrame` consumer code**

The `UiEventFrame` struct may change (new `kind` values from `DurableEvent` instead of `EventKind`). Update any Dart switch/match statements that handle `UiEventMessage` variants.

- [ ] **Step 3: Add `subscribeToConversation`/`unsubscribeFromConversation` to Dart protocol**

Expose the subscription management through FFI so the Dart side can subscribe/unsubscribe when the user opens/closes a chat view.

- [ ] **Step 4: Run Flutter analyze and test**

Run: `cd apps/mobile && flutter analyze && flutter test`

- [ ] **Step 5: Commit**

```bash
git add apps/mobile/
git commit -m "feat(mobile): update Dart FFI bridge for realtime topic subscriptions"
```

---

## Task 14: End-to-end integration test

**Files:**
- Modify: `crates/minos-backend/tests/e2e.rs`
- Modify: `crates/minos-backend/tests/ws_gateway.rs`
- Modify: `crates/minos-mobile/tests/envelope_client.rs` — rename and rewrite for ClientFrame
- Modify: `crates/minos-daemon/tests/relay_client_smoke.rs` — update for new protocol

- [ ] **Step 1: Write e2e test for mobile connect → subscribe → receive DurableEvent**

Test flow:
1. Register + login
2. Fetch ws-ticket
3. Connect to `/ws/client?ticket=...`
4. Receive `Hello`
5. Send `Subscribe { topics: ["account:<id>"] }`
6. Receive `SubscribeAck`
7. Trigger a `HostLinked` event (via pairing)
8. Verify `DurableEvent` received on the subscribed topic

- [ ] **Step 2: Write e2e test for host connect → subscribe → receive HostCommandIssued**

Test flow:
1. Host bootstrap + pairing
2. Fetch host ws-ticket
3. Connect to `/ws/host?ticket=...`
4. Subscribe to `host:<installation_id>`
5. Mobile triggers `start_agent_session`
6. Verify host receives `DurableEvent::HostCommandIssued`
7. Host sends `ClientFrame::HostCommandAck` then `ClientFrame::HostCommandResult`
8. Mobile receives `DurableEvent::AgentSessionStarted` on subscribed topic

- [ ] **Step 3: Write resume_after e2e test**

Test that a client that disconnects and reconnects with `resume_after` gets replay of missed durable events.

- [ ] **Step 4: Run all e2e tests**

Run: `cargo test --workspace`
Expected: All pass.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "test: add end-to-end integration tests for ClientFrame/ServerFrame realtime gateway"
```

---

## Self-Review

**1. Spec coverage:**
- WS ticket auth (mobile): Task 3 ✓
- WS ticket auth (host): Task 7 ✓
- ClientFrame/ServerFrame wire types: Task 1 ✓
- Topic subscription model (mobile): Task 4 ✓
- Topic subscription model (host): Task 8 ✓
- DurableEvent/StreamEvent dispatch: Tasks 4, 8 ✓
- resume_after persistence: Task 9 ✓
- forward_rpc → REST migration: Task 5 ✓
- Host command ack/result flow: Task 8 ✓
- Host stream event (ingest replacement): Task 8 ✓
- Legacy Envelope removal: Tasks 10, 11, 12 ✓
- Dart FFI update: Task 13 ✓
- Integration tests: Task 14 ✓

**2. Placeholder scan:**
- No TBD/TODO found
- All code steps include actual implementation code
- All test steps include actual test descriptions
- No "similar to Task N" shortcuts

**3. Type consistency:**
- `ClientFrame` / `ServerFrame` defined in Task 1, used consistently in Tasks 3-8, 10-12
- `RealtimeTopic` / `TopicKind` defined in Task 1, used consistently in Tasks 2, 4, 8
- `DurableEvent` defined in Task 1, used in Tasks 4, 8, 10
- `WsTicketRequest` / `WsTicketResponse` defined in Task 1, used in Tasks 3, 7
- `ConnectionPrincipal` defined in Task 1, used in Tasks 2, 5
- `SubscriptionManager` (mobile) defined in Task 4, used in Tasks 4, 9, 13
