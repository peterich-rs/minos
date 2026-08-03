//! WebSocket envelope for the Minos relay.
//!
//! Every text frame over the relay's `/devices` endpoint is exactly one JSON
//! object matching [`Envelope`]. The envelope is intentionally small and
//! server-terminated: the relay routes [`Envelope::Forward`] payloads
//! opaquely between paired devices and terminates everything else locally.
//!
//! # Wire shape
//!
//! - The outer discriminator is `kind` (e.g. `"kind":"forward"`),
//!   `rename_all = "snake_case"`.
//! - Every envelope carries `"v": 1` (the field is named `version` in Rust
//!   but renamed to `v` on the wire). Future breaking changes bump the
//!   version; clients that see an unrecognised `v` are expected to close
//!   the socket with a typed error (spec §6.3).
//! - `EventKind` flattens into [`Envelope::Event`] with a `type`
//!   discriminator matching spec §6.
//!
//! The Rust types here plus the golden JSON fixtures under
//! `tests/golden/envelope/` are the authoritative wire definition.
//! Any change to these types MUST be accompanied by a fixture update.

use minos_domain::{DeviceId, DeviceSecret};
use serde::{Deserialize, Serialize};

use crate::ChatMessageSummary;

/// One WebSocket frame on the relay's `/devices` endpoint.
///
/// Serialised as a tagged JSON object with `kind` as the discriminator.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Envelope {
    /// Client → Relay. Relay forwards `payload` opaquely to the paired
    /// peer as an [`Envelope::Forwarded`]. The relay does not inspect or
    /// mutate `payload`; correlation of request/response is the clients'
    /// responsibility (see spec §6.2).
    Forward {
        /// Protocol version.
        #[serde(rename = "v")]
        version: u8,
        /// The Mac device this forward should be routed to. Backend
        /// validates against the caller's account_host_pairings rows.
        /// Mismatch → routing error (PeerOffline).
        target_device_id: DeviceId,
        /// Opaque payload (JSON-RPC 2.0 by convention between Minos
        /// clients, but the relay does not read it).
        payload: serde_json::Value,
    },
    /// Relay → Client. Delivery of a peer's [`Envelope::Forward`].
    Forwarded {
        /// Protocol version.
        #[serde(rename = "v")]
        version: u8,
        /// Sender's `DeviceId`. Serialised as a bare UUID string because
        /// `DeviceId` is `#[serde(transparent)]`.
        from: DeviceId,
        /// The payload the peer sent, verbatim.
        payload: serde_json::Value,
    },
    /// Relay → Client. Server-side state push; carries a typed
    /// [`EventKind`] flattened with a `type` discriminator.
    Event {
        /// Protocol version.
        #[serde(rename = "v")]
        version: u8,
        /// The event body; see [`EventKind`] variants.
        #[serde(flatten)]
        event: EventKind,
    },
    /// Agent-host → Backend. Raw native event from a CLI for persistence
    /// and fan-out. No response expected. (seq, session_id) must be unique
    /// server-side; the host treats conflicts as a no-op.
    Ingest {
        #[serde(rename = "v")]
        version: u8,
        agent: minos_domain::AgentName,
        session_id: String,
        seq: u64,
        payload: serde_json::Value,
        ts_ms: i64,
    },
}

/// Server-initiated state change pushed to the client, body of
/// [`Envelope::Event`].
///
/// On the wire, the `type` key carries the variant name in `snake_case`
/// (spec §6). Payload fields sit alongside `type` thanks to the
/// `#[serde(flatten)]` in [`Envelope::Event`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum EventKind {
    /// Emitted only to the Mac side after an iPhone successfully consumes
    /// a pairing token (spec §7.1). Delivers the iPhone's identity plus
    /// the long-lived `DeviceSecret` the Mac will use for future
    /// WebSocket auth (spec §9.4).
    Paired {
        /// The iPhone's `DeviceId`.
        peer_device_id: DeviceId,
        /// Display name the iPhone registered during `pair`.
        peer_name: String,
        /// Long-lived bearer secret for the Mac recipient. `None` when this
        /// event is delivered to an iOS recipient (iOS rail is bearer-only;
        /// see ADR-0020).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        your_device_secret: Option<DeviceSecret>,
    },
    /// The paired peer's WebSocket came online.
    PeerOnline {
        /// Peer's `DeviceId`.
        peer_device_id: DeviceId,
    },
    /// The paired peer's WebSocket dropped (clean close or failure).
    PeerOffline {
        /// Peer's `DeviceId`.
        peer_device_id: DeviceId,
    },
    /// The peer called `forget_peer`, or an admin revoked the pairing
    /// server-side. Clients should clear local pair state.
    Unpaired,
    /// Relay is shutting down; clients should reconnect with backoff.
    ServerShutdown,
    /// Backend → Mobile. One translated UI event from backend's live
    /// fan-out. `seq` matches the underlying `raw_events` row so mobile
    /// can dedupe against its per-session watermark.
    UiEventMessage {
        session_id: String,
        seq: u64,
        ui: minos_ui_protocol::UiEventMessage,
        ts_ms: i64,
    },
    /// Backend → Mobile. Approval request forwarded from the host for an
    /// in-flight turn and awaiting explicit user action.
    ApprovalRequest {
        session_id: String,
        turn_id: String,
        request_id: String,
        method: String,
        params: serde_json::Value,
        timeout_ms: u64,
    },
    /// Backend → Mobile. A pending approval auto-expired or was declined due
    /// to disconnect before the user responded.
    ApprovalTimeout {
        session_id: String,
        request_id: String,
        reason: String,
    },
    /// Backend → Mobile. Structured error associated with sending into an
    /// agent session. `session_id` is omitted when the failure happened before
    /// a session existed.
    AgentError {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        code: String,
        message: String,
    },
    /// Backend → Mobile. One realtime social-chat message fan-out.
    ///
    /// Emitted to every live mobile session whose account is a member of
    /// `conversation_id`. The payload is the hydrated HTTP message summary so
    /// mobile can append it locally without refetching the full page.
    SocialMessage {
        conversation_id: String,
        message: ChatMessageSummary,
    },
}

#[cfg(test)]
mod tests {
    //! Inline round-trip tests. The separate
    //! `tests/envelope_golden.rs` integration test freezes the exact wire
    //! shape via hand-authored JSON fixtures.

    use super::*;
    use pretty_assertions::assert_eq;

    fn round_trip(env: &Envelope) {
        let json = serde_json::to_string(env).unwrap();
        let back: Envelope = serde_json::from_str(&json).unwrap();
        let reserialised = serde_json::to_value(&back).unwrap();
        let expected: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(reserialised, expected);
    }

    #[test]
    fn forward_round_trips() {
        let env = Envelope::Forward {
            version: 1,
            target_device_id: DeviceId::new(),
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "method": "list_clis",
                "id": 1,
            }),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "forward");
    }

    #[test]
    fn forward_with_target_round_trips() {
        let target = DeviceId::new();
        let env = Envelope::Forward {
            version: 1,
            target_device_id: target,
            payload: serde_json::json!({"jsonrpc": "2.0", "method": "ping", "id": 1}),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "forward");
        assert_eq!(
            v["target_device_id"].as_str().unwrap(),
            target.0.to_string()
        );
    }

    #[test]
    fn forwarded_round_trips_with_transparent_device_id() {
        let id = DeviceId::new();
        let env = Envelope::Forwarded {
            version: 1,
            from: id,
            payload: serde_json::json!({"jsonrpc": "2.0", "result": [], "id": 1}),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        // DeviceId must serialise as a bare UUID string (transparent).
        assert!(v["from"].is_string());
        assert_eq!(v["from"].as_str().unwrap(), id.0.to_string());
    }

    #[test]
    fn event_paired_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: DeviceId::new(),
                peer_name: "Mac-mini".into(),
                your_device_secret: Some(DeviceSecret("sek".into())),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "event");
        assert_eq!(v["type"], "paired");
        // Plaintext secret MUST appear on the wire (no redaction via serde).
        assert_eq!(v["your_device_secret"], "sek");
    }

    #[test]
    fn paired_event_with_no_secret_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: DeviceId::new(),
                peer_name: "iPhone".into(),
                your_device_secret: None,
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "paired");
        assert!(v.get("your_device_secret").is_none() || v["your_device_secret"].is_null());
    }

    #[test]
    fn event_peer_online_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::PeerOnline {
                peer_device_id: DeviceId::new(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "peer_online");
    }

    #[test]
    fn social_message_event_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::SocialMessage {
                conversation_id: "conv-123".into(),
                message: ChatMessageSummary {
                    message_id: "msg-123".into(),
                    conversation_id: "conv-123".into(),
                    sender: crate::UserSummary {
                        account_id: "acct-1".into(),
                        minos_id: "alice01".into(),
                        display_name: "Alice".into(),
                    },
                    text: "hello from websocket".into(),
                    created_at_ms: 1_717_171_717,
                    message_seq: 1,
                    reply_to: None,
                    recalled_at_ms: None,
                    mentioned_account_ids: Vec::new(),
                    sender_type: crate::SenderType::User,
                    reactions: vec![],
                },
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "event");
        assert_eq!(v["type"], "social_message");
        assert_eq!(v["conversation_id"], "conv-123");
        assert_eq!(v["message"]["message_id"], "msg-123");
        assert_eq!(v["message"]["sender"]["display_name"], "Alice");
    }

    #[test]
    fn event_peer_offline_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::PeerOffline {
                peer_device_id: DeviceId::new(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "peer_offline");
    }

    #[test]
    fn event_unpaired_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::Unpaired,
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "unpaired");
    }

    #[test]
    fn event_server_shutdown_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::ServerShutdown,
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "server_shutdown");
    }

    #[test]
    fn approval_request_event_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::ApprovalRequest {
                session_id: "thr-approval".into(),
                turn_id: "turn-123".into(),
                request_id: "req-123".into(),
                method: "exec_command".into(),
                params: serde_json::json!({
                    "command": ["cargo", "test"],
                    "cwd": "/Users/fan/dev/minos",
                }),
                timeout_ms: 120_000,
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "approval_request");
        assert_eq!(v["request_id"], "req-123");
        assert_eq!(v["params"]["command"][0], "cargo");
    }

    #[test]
    fn approval_timeout_event_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::ApprovalTimeout {
                session_id: "thr-approval".into(),
                request_id: "req-123".into(),
                reason: "timeout".into(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "approval_timeout");
        assert_eq!(v["reason"], "timeout");
    }

    #[test]
    fn agent_error_event_round_trips_without_session_id() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::AgentError {
                session_id: None,
                code: "peer_offline".into(),
                message: "host is offline".into(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "agent_error");
        assert_eq!(v["code"], "peer_offline");
        assert!(v.get("session_id").is_none());
    }

    #[test]
    fn envelope_ingest_round_trip() {
        let e = Envelope::Ingest {
            version: 1,
            agent: minos_domain::AgentName::Codex,
            session_id: "thr_1".into(),
            seq: 42,
            payload: serde_json::json!({"method":"item/agentMessage/delta","params":{"delta":"Hi"}}),
            ts_ms: 1_714_000_000_000,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""kind":"ingest""#));
        assert!(s.contains(r#""agent":"codex""#));
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn envelope_event_ui_event_message_round_trip() {
        let e = Envelope::Event {
            version: 1,
            event: EventKind::UiEventMessage {
                session_id: "thr_1".into(),
                seq: 42,
                ui: minos_ui_protocol::UiEventMessage::TextDelta {
                    message_id: "msg_1".into(),
                    text: "Hi".into(),
                },
                ts_ms: 1_714_000_000_000,
            },
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""type":"ui_event_message""#));
        assert!(s.contains(r#""kind":"text_delta""#));
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }
}
