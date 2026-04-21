//! Agent CLI descriptors (names, statuses, full descriptor records).

use serde::{Deserialize, Serialize};

/// The set of CLI agents Minos knows how to manage.
///
/// MVP enumerates the three planned backends; expansion is a breaking change
/// (intentional — every consumer must opt in to a new agent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentName {
    Codex,
    Claude,
    Gemini,
}

impl AgentName {
    /// All known agents, in the order shown to users.
    #[must_use]
    pub const fn all() -> &'static [AgentName] {
        &[AgentName::Codex, AgentName::Claude, AgentName::Gemini]
    }

    /// The CLI binary name to look for on PATH.
    #[must_use]
    pub const fn bin_name(self) -> &'static str {
        match self {
            AgentName::Codex => "codex",
            AgentName::Claude => "claude",
            AgentName::Gemini => "gemini",
        }
    }
}

/// Health state of a single CLI agent on the local machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AgentStatus {
    Ok,
    Missing,
    Error { reason: String },
}

/// The complete description of one agent's local installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub name: AgentName,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: AgentStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_name_serializes_snake_case() {
        let s = serde_json::to_string(&AgentName::Codex).unwrap();
        assert_eq!(s, "\"codex\"");
    }

    #[test]
    fn agent_status_ok_serializes_with_kind_tag() {
        let s = serde_json::to_string(&AgentStatus::Ok).unwrap();
        assert_eq!(s, r#"{"kind":"ok"}"#);
    }

    #[test]
    fn agent_status_error_carries_reason() {
        let s = serde_json::to_string(&AgentStatus::Error { reason: "boom".into() }).unwrap();
        assert_eq!(s, r#"{"kind":"error","reason":"boom"}"#);
    }

    #[test]
    fn agent_descriptor_round_trips() {
        let d = AgentDescriptor {
            name: AgentName::Claude,
            path: Some("/usr/local/bin/claude".into()),
            version: Some("1.2.0".into()),
            status: AgentStatus::Ok,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: AgentDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn agent_name_all_returns_three_in_canonical_order() {
        assert_eq!(AgentName::all().len(), 3);
        assert_eq!(AgentName::all()[0], AgentName::Codex);
    }
}
