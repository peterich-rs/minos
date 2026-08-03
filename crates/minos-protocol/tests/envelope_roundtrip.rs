//! Extended round-trip coverage for all `Envelope` and `EventKind` variants.
//! Spec R9 — parse(serialize(x)) == x for every wire type.
//!
//! Plan P7.1: property-style test (hand-rolled loop over representative
//! variants) that encodes each variant and decodes back.

use minos_domain::{AgentName, DeviceId, DeviceSecret};
use minos_protocol::{ChatMessageSummary, Envelope, EventKind, SenderType, UserSummary};
use minos_ui_protocol::UiEventMessage;
use pretty_assertions::assert_eq;

fn assert_round_trip(env: &Envelope) {
    let json = serde_json::to_string(env).unwrap();
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert_eq!(*env, back, "round-trip failed for: {json}");
    // Re-serialize to confirm stability
    let json2 = serde_json::to_string(&back).unwrap();
    let v1: serde_json::Value = serde_json::from_str(&json).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&json2).unwrap();
    assert_eq!(v1, v2, "re-serialization not stable");
}

#[test]
fn all_envelope_variants_round_trip() {
    let device = DeviceId::new();

    let envelopes: Vec<Envelope> = vec![
        // Forward with nested JSON-RPC params
        Envelope::Forward {
            version: 1,
            target_device_id: device,
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "minos_start_agent",
                "params": {
                    "agent": "codex",
                    "workspace": "/Users/test/project",
                    "mode": null,
                    "nested": { "deep": [1, 2, 3] }
                }
            }),
        },
        // Forwarded
        Envelope::Forwarded {
            version: 1,
            from: device,
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 42,
                "result": { "session_id": "sess-1", "cwd": "/tmp" }
            }),
        },
        // Ingest
        Envelope::Ingest {
            version: 1,
            agent: AgentName::Codex,
            session_id: "thr-abc".into(),
            seq: 99,
            payload: serde_json::json!({"method": "item/agentMessage/delta", "params": {"delta": "hello"}}),
            ts_ms: 1_714_000_000_000,
        },
        Envelope::Ingest {
            version: 1,
            agent: AgentName::Claude,
            session_id: "thr-claude-1".into(),
            seq: 1,
            payload: serde_json::json!({"kind": "raw", "raw_kind": "stdout", "payload_json": "\"line1\""}),
            ts_ms: 1_714_000_000_001,
        },
        Envelope::Ingest {
            version: 1,
            agent: AgentName::Gemini,
            session_id: "thr-gemini-1".into(),
            seq: 0,
            payload: serde_json::json!({}),
            ts_ms: 0,
        },
    ];

    for env in &envelopes {
        assert_round_trip(env);
    }
}

#[test]
fn all_event_kind_variants_round_trip() {
    let device = DeviceId::new();

    let events: Vec<EventKind> = vec![
        EventKind::Paired {
            peer_device_id: device,
            peer_name: "Test Mac".into(),
            your_device_secret: Some(DeviceSecret("secret-value".into())),
        },
        EventKind::Paired {
            peer_device_id: device,
            peer_name: "iPhone".into(),
            your_device_secret: None,
        },
        EventKind::PeerOnline {
            peer_device_id: device,
        },
        EventKind::PeerOffline {
            peer_device_id: device,
        },
        EventKind::Unpaired,
        EventKind::ServerShutdown,
        EventKind::UiEventMessage {
            session_id: "thr-1".into(),
            seq: 5,
            ui: UiEventMessage::TextDelta {
                message_id: "msg-1".into(),
                text: "Hello world".into(),
            },
            ts_ms: 1_714_000_000_000,
        },
        EventKind::UiEventMessage {
            session_id: "thr-2".into(),
            seq: 1,
            ui: UiEventMessage::Raw {
                kind: "custom_event".into(),
                payload_json: r#"{"data":"test"}"#.into(),
            },
            ts_ms: 1_714_000_000_001,
        },
        EventKind::ApprovalRequest {
            session_id: "thr-approval".into(),
            turn_id: "turn-123".into(),
            request_id: "req-123".into(),
            method: "exec_command".into(),
            params: serde_json::json!({
                "command": ["cargo", "test", "-p", "minos-protocol"],
                "cwd": "/Users/test/project",
            }),
            timeout_ms: 120_000,
        },
        EventKind::ApprovalTimeout {
            session_id: "thr-approval".into(),
            request_id: "req-123".into(),
            reason: "timeout".into(),
        },
        EventKind::AgentError {
            session_id: None,
            code: "peer_offline".into(),
            message: "host is offline".into(),
        },
        EventKind::SocialMessage {
            conversation_id: "conv-1".into(),
            message: ChatMessageSummary {
                message_id: "msg-social-1".into(),
                conversation_id: "conv-1".into(),
                sender: UserSummary {
                    account_id: "acct-1".into(),
                    minos_id: "alice".into(),
                    display_name: "Alice".into(),
                },
                text: "Hey there".into(),
                created_at_ms: 1_717_000_000,
                message_seq: 1,
                reply_to: None,
                recalled_at_ms: None,
                mentioned_account_ids: vec!["acct-2".into()],
                sender_type: SenderType::User,
            reactions: vec![],
            },
        },
    ];

    for event in &events {
        let env = Envelope::Event {
            version: 1,
            event: event.clone(),
        };
        assert_round_trip(&env);
    }
}

#[test]
fn forward_with_arbitrary_nested_json_params() {
    let env = Envelope::Forward {
        version: 1,
        target_device_id: DeviceId::new(),
        payload: serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "complex_method",
            "params": {
                "array": [1, "two", null, true, {"nested": "object"}],
                "unicode": "日本語テスト 🎉",
                "empty_obj": {},
                "empty_arr": [],
                "number": 3.14160
            }
        }),
    };
    assert_round_trip(&env);
}
