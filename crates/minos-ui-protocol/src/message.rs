use minos_domain::AgentName;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRef {
    pub session_id: String,
    pub artifact_id: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub media_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayPayload {
    Inline {
        text: String,
    },
    StreamingWindow {
        head: String,
        received_bytes: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<ArtifactRef>,
    },
    WindowedFinal {
        head: String,
        tail: String,
        omitted_bytes: u64,
        artifact: ArtifactRef,
    },
}

impl DisplayPayload {
    #[must_use]
    pub fn inline(text: impl Into<String>) -> Self {
        Self::Inline { text: text.into() }
    }

    #[must_use]
    pub fn render_preview(&self) -> String {
        match self {
            Self::Inline { text } => text.clone(),
            Self::StreamingWindow { head, .. } => head.clone(),
            Self::WindowedFinal {
                head,
                tail,
                omitted_bytes,
                ..
            } => {
                if *omitted_bytes == 0 {
                    format!("{head}{tail}")
                } else {
                    format!("{head}\n\n[omitted {omitted_bytes} bytes]\n\n{tail}")
                }
            }
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Inline { text } => text.is_empty(),
            Self::StreamingWindow { head, .. } => head.is_empty(),
            Self::WindowedFinal { head, tail, .. } => head.is_empty() && tail.is_empty(),
        }
    }
}

impl From<String> for DisplayPayload {
    fn from(text: String) -> Self {
        Self::inline(text)
    }
}

impl From<&str> for DisplayPayload {
    fn from(text: &str) -> Self {
        Self::inline(text)
    }
}

impl PartialEq<str> for DisplayPayload {
    fn eq(&self, other: &str) -> bool {
        self.render_preview() == other
    }
}

impl PartialEq<&str> for DisplayPayload {
    fn eq(&self, other: &&str) -> bool {
        self.render_preview() == *other
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEventMessage {
    // ── Session lifecycle ────────────
    SessionOpened {
        session_id: String,
        agent: AgentName,
        title: Option<String>,
        opened_at_ms: i64,
    },
    SessionTitleUpdated {
        session_id: String,
        title: String,
    },
    SessionClosed {
        session_id: String,
        reason: SessionEndReason,
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
        text: DisplayPayload,
    },
    TextReplace {
        message_id: String,
        text: DisplayPayload,
    },
    ReasoningDelta {
        message_id: String,
        text: DisplayPayload,
    },
    ReasoningReplace {
        message_id: String,
        text: DisplayPayload,
    },

    // ── Tool calls ───────────────────
    ToolCallPlaced {
        message_id: String,
        tool_call_id: String,
        name: String,
        args_json: DisplayPayload,
    },
    ToolCallCompleted {
        tool_call_id: String,
        output: DisplayPayload,
        is_error: bool,
    },
    SubagentSpawned {
        parent_session_id: String,
        sub_session_id: String,
        tool_call_id: String,
        agent: AgentName,
        model: Option<String>,
        prompt: Option<String>,
        title: Option<String>,
    },
    SubagentStatusUpdated {
        sub_session_id: String,
        status: SubagentStatus,
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEndReason {
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
            text: DisplayPayload::inline("Hello"),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"text_delta","message_id":"msg_1","text":{"kind":"inline","text":"Hello"}}"#
        );
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn session_opened_serialises_snake_case_agent() {
        let ev = UiEventMessage::SessionOpened {
            session_id: "thr_1".into(),
            agent: AgentName::Codex,
            title: Some("hi".into()),
            opened_at_ms: 1_714_000_000_000,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"session_opened""#));
        assert!(json.contains(r#""agent":"codex""#));
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn session_opened_with_null_title_round_trip() {
        let ev = UiEventMessage::SessionOpened {
            session_id: "thr_2".into(),
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
        let ev = UiEventMessage::SessionClosed {
            session_id: "thr_1".into(),
            reason: SessionEndReason::Crashed {
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
            args_json: DisplayPayload::inline(r#"{"diff":"..."}"#),
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

    #[test]
    fn subagent_spawned_round_trip() {
        let ev = UiEventMessage::SubagentSpawned {
            parent_session_id: "parent".into(),
            sub_session_id: "sub".into(),
            tool_call_id: "tool-1".into(),
            agent: AgentName::Codex,
            model: Some("gpt-5".into()),
            prompt: Some("inspect".into()),
            title: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"subagent_spawned""#));
        let back: UiEventMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }
}
