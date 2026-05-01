use minos_domain::AgentName;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEventMessage {
    // ── Thread lifecycle ─────────────
    ThreadOpened {
        thread_id: String,
        agent: AgentName,
        title: Option<String>,
        opened_at_ms: i64,
    },
    ThreadTitleUpdated {
        thread_id: String,
        title: String,
    },
    ThreadClosed {
        thread_id: String,
        reason: ThreadEndReason,
        closed_at_ms: i64,
    },

    // ── Message boundaries ───────────
    MessageStarted {
        message_id: String,
        role: MessageRole,
        started_at_ms: i64,
    },
    MessageCompleted {
        message_id: String,
        finished_at_ms: i64,
    },

    // ── Message content ──────────────
    TextDelta {
        message_id: String,
        text: String,
    },
    ReasoningDelta {
        message_id: String,
        text: String,
    },

    // ── Tool calls ───────────────────
    ToolCallPlaced {
        message_id: String,
        tool_call_id: String,
        name: String,
        args_json: String,
    },
    ToolCallCompleted {
        tool_call_id: String,
        output: String,
        is_error: bool,
    },

    // ── Meta / escape hatch ──────────
    Error {
        code: String,
        message: String,
        message_id: Option<String>,
    },
    Raw {
        // `kind` collides with the outer `tag = "kind"` discriminator;
        // rename only the JSON wire key. Rust identifier stays `kind`.
        #[serde(rename = "raw_kind")]
        kind: String,
        payload_json: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadEndReason {
    UserStopped,
    AgentDone,
    Crashed { message: String },
    Timeout,
    HostDisconnected,
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use pretty_assertions::assert_eq;

    #[test]
    fn text_delta_round_trip() {
        let ev = UiEventMessage::TextDelta {
            message_id: "msg_1".into(),
            text: "Hello".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"text_delta","message_id":"msg_1","text":"Hello"}"#
        );
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn thread_opened_serialises_snake_case_agent() {
        let ev = UiEventMessage::ThreadOpened {
            thread_id: "thr_1".into(),
            agent: AgentName::Codex,
            title: Some("hi".into()),
            opened_at_ms: 1_714_000_000_000,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"thread_opened""#));
        assert!(json.contains(r#""agent":"codex""#));
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn thread_opened_with_null_title_round_trip() {
        let ev = UiEventMessage::ThreadOpened {
            thread_id: "thr_2".into(),
            agent: AgentName::Claude,
            title: None,
            opened_at_ms: 1_714_000_000_001,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn thread_closed_reason_crashed_has_nested_message() {
        let ev = UiEventMessage::ThreadClosed {
            thread_id: "thr_1".into(),
            reason: ThreadEndReason::Crashed {
                message: "oom".into(),
            },
            closed_at_ms: 1_714_000_000_000,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""reason":{"kind":"crashed","message":"oom"}"#));
    }

    #[test]
    fn tool_call_placed_carries_full_args_json() {
        let ev = UiEventMessage::ToolCallPlaced {
            message_id: "msg_1".into(),
            tool_call_id: "tc_1".into(),
            name: "apply_patch".into(),
            args_json: r#"{"diff":"..."}"#.into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn raw_is_forward_compat_escape_hatch() {
        let ev = UiEventMessage::Raw {
            kind: "item/plan/delta".into(),
            payload_json: r#"{"step":"compile"}"#.into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"raw""#));
    }

    #[test]
    fn message_role_assistant_snake_case() {
        let r = MessageRole::Assistant;
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#""assistant""#);
    }
}
