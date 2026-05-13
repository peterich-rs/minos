//! Per-variant round-trip test for `UiEventMessage`.
//! Spec R9 — parse(serialize(x)) == x for every wire type, including
//! the `Raw { raw_kind, payload_json }` fallback.
//!
//! Plan P7.2.

use minos_domain::AgentName;
use minos_ui_protocol::{MessageRole, ThreadEndReason, UiEventMessage};
use pretty_assertions::assert_eq;

fn assert_round_trip(msg: &UiEventMessage) {
    let json = serde_json::to_string(msg).unwrap();
    let back: UiEventMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(*msg, back, "round-trip failed for: {json}");
}

fn thread_variants() -> Vec<UiEventMessage> {
    vec![
        UiEventMessage::ThreadOpened {
            thread_id: "thr-1".into(),
            agent: AgentName::Codex,
            title: Some("Fix the bug".into()),
            opened_at_ms: 1_714_000_000_000,
        },
        UiEventMessage::ThreadOpened {
            thread_id: "thr-2".into(),
            agent: AgentName::Claude,
            title: None,
            opened_at_ms: 1_714_000_000_001,
        },
        UiEventMessage::ThreadOpened {
            thread_id: "thr-3".into(),
            agent: AgentName::Gemini,
            title: Some(String::new()),
            opened_at_ms: 0,
        },
        UiEventMessage::ThreadTitleUpdated {
            thread_id: "thr-1".into(),
            title: "Updated title".into(),
        },
        UiEventMessage::ThreadClosed {
            thread_id: "thr-1".into(),
            reason: ThreadEndReason::UserStopped,
            closed_at_ms: 1_714_000_001_000,
        },
        UiEventMessage::ThreadClosed {
            thread_id: "thr-1".into(),
            reason: ThreadEndReason::AgentDone,
            closed_at_ms: 1_714_000_001_000,
        },
        UiEventMessage::ThreadClosed {
            thread_id: "thr-1".into(),
            reason: ThreadEndReason::Crashed {
                message: "out of memory".into(),
            },
            closed_at_ms: 1_714_000_001_000,
        },
        UiEventMessage::ThreadClosed {
            thread_id: "thr-1".into(),
            reason: ThreadEndReason::Timeout,
            closed_at_ms: 1_714_000_001_000,
        },
        UiEventMessage::ThreadClosed {
            thread_id: "thr-1".into(),
            reason: ThreadEndReason::HostDisconnected,
            closed_at_ms: 1_714_000_001_000,
        },
    ]
}

fn message_variants() -> Vec<UiEventMessage> {
    vec![
        UiEventMessage::MessageStarted {
            message_id: "msg-1".into(),
            role: MessageRole::User,
            started_at_ms: 1_714_000_000_100,
        },
        UiEventMessage::MessageStarted {
            message_id: "msg-2".into(),
            role: MessageRole::Assistant,
            started_at_ms: 1_714_000_000_200,
        },
        UiEventMessage::MessageStarted {
            message_id: "msg-3".into(),
            role: MessageRole::System,
            started_at_ms: 1_714_000_000_300,
        },
        UiEventMessage::MessageCompleted {
            message_id: "msg-1".into(),
            finished_at_ms: 1_714_000_000_500,
        },
        UiEventMessage::TextDelta {
            message_id: "msg-2".into(),
            text: "Hello, world!".into(),
        },
        UiEventMessage::TextDelta {
            message_id: "msg-2".into(),
            text: String::new(),
        },
        UiEventMessage::ReasoningDelta {
            message_id: "msg-2".into(),
            text: "Let me think about this...".into(),
        },
        UiEventMessage::ToolCallPlaced {
            message_id: "msg-2".into(),
            tool_call_id: "tc-1".into(),
            name: "apply_patch".into(),
            args_json: r#"{"diff":"--- a/file.rs\n+++ b/file.rs"}"#.into(),
        },
        UiEventMessage::ToolCallCompleted {
            tool_call_id: "tc-1".into(),
            output: "Patch applied successfully".into(),
            is_error: false,
        },
        UiEventMessage::ToolCallCompleted {
            tool_call_id: "tc-2".into(),
            output: "File not found".into(),
            is_error: true,
        },
        UiEventMessage::Error {
            code: "rate_limited".into(),
            message: "Too many requests".into(),
            message_id: Some("msg-2".into()),
        },
        UiEventMessage::Error {
            code: "internal".into(),
            message: "Unexpected error".into(),
            message_id: None,
        },
    ]
}

fn raw_variants() -> Vec<UiEventMessage> {
    vec![
        UiEventMessage::Raw {
            kind: "stdout".into(),
            payload_json: r#""hello from claude""#.into(),
        },
        UiEventMessage::Raw {
            kind: "stderr".into(),
            payload_json: r#""warning: deprecated""#.into(),
        },
        UiEventMessage::Raw {
            kind: "custom/event/type".into(),
            payload_json: r#"{"step":"compile","progress":0.5}"#.into(),
        },
        UiEventMessage::Raw {
            kind: String::new(),
            payload_json: "null".into(),
        },
    ]
}

#[test]
fn all_ui_event_message_variants_round_trip() {
    let messages: Vec<UiEventMessage> = [thread_variants(), message_variants(), raw_variants()]
        .into_iter()
        .flatten()
        .collect();

    for msg in &messages {
        assert_round_trip(msg);
    }
}

#[test]
fn raw_variant_wire_shape_uses_raw_kind_key() {
    let msg = UiEventMessage::Raw {
        kind: "item/plan/delta".into(),
        payload_json: r#"{"step":"compile"}"#.into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    // The outer discriminator is "kind":"raw"
    assert!(json.contains(r#""kind":"raw""#));
    // The inner field is "raw_kind" (not "kind" again)
    assert!(json.contains(r#""raw_kind":"item/plan/delta""#));
    assert!(json.contains(r#""payload_json":"{\"step\":\"compile\"}""#));
}
