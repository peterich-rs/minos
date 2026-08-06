//! Agent CLI descriptors (names, statuses, full descriptor records).
//!
//! **Capability SSOT**: harness/runtime capability facts live here (and in the
//! daemon model catalog for per-model efforts). UI layers only project these
//! values — they must not invent runtimes or default effort ladders.

use serde::{Deserialize, Serialize};

/// How models are discovered for a runtime.
///
/// Pure domain metadata; not currently on the wire `AgentDescriptor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDiscovery {
    /// Curated static list only (no live probe).
    Static,
    /// Probe via CLI (`models` subcommand, etc.).
    Cli,
    /// Probe via app-server / ACP-style process (e.g. Codex).
    AppServer,
}

/// The set of CLI agents Minos knows how to manage.
///
/// Expansion is intentional API surface growth: every consumer must opt in
/// to a new agent (match arms, DB seeds, UI labels, codegen bindings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentName {
    Codex,
    Claude,
    Gemini,
    Opencode,
    Grok,
}

impl AgentName {
    /// All known agents, in the order shown to users.
    #[must_use]
    pub const fn all() -> &'static [AgentName] {
        &[
            AgentName::Codex,
            AgentName::Claude,
            AgentName::Gemini,
            AgentName::Opencode,
            AgentName::Grok,
        ]
    }

    /// The CLI binary name to look for on PATH.
    #[must_use]
    pub const fn bin_name(self) -> &'static str {
        match self {
            AgentName::Codex => "codex",
            AgentName::Claude => "claude",
            AgentName::Gemini => "gemini",
            AgentName::Opencode => "opencode",
            AgentName::Grok => "grok",
        }
    }

    /// Human-facing label (title case product name).
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            AgentName::Codex => "Codex",
            AgentName::Claude => "Claude",
            AgentName::Gemini => "Gemini",
            AgentName::Opencode => "OpenCode",
            AgentName::Grok => "Grok",
        }
    }

    /// Whether the runtime accepts a model id at session start.
    #[must_use]
    pub const fn supports_model_selection(self) -> bool {
        // All current harnesses accept a model id (static list and/or probe).
        true
    }

    /// Whether the runtime supports reasoning-effort as a launch parameter.
    ///
    /// Per-model `supported_reasoning_efforts` may still be empty when the
    /// selected model does not expose effort levels.
    #[must_use]
    pub const fn supports_reasoning_effort(self) -> bool {
        match self {
            AgentName::Codex | AgentName::Grok => true,
            AgentName::Claude | AgentName::Gemini | AgentName::Opencode => false,
        }
    }

    /// Preferred model-discovery strategy for this runtime.
    #[must_use]
    pub const fn model_discovery(self) -> ModelDiscovery {
        match self {
            AgentName::Codex => ModelDiscovery::AppServer,
            AgentName::Claude | AgentName::Gemini => ModelDiscovery::Static,
            AgentName::Opencode | AgentName::Grok => ModelDiscovery::Cli,
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

/// The complete description of one agent's local installation + capabilities.
///
/// Capability fields are filled from [`AgentName`] domain metadata (SSOT).
/// Install probe fields (`path`, `version`, `status`) come from detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub name: AgentName,
    /// Human-facing label projected from domain meta.
    pub display_name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: AgentStatus,
    /// Runtime accepts a model id at session start.
    pub supports_model_selection: bool,
    /// Runtime supports reasoning-effort as a launch parameter.
    pub supports_reasoning_effort: bool,
}

impl AgentDescriptor {
    /// Build a descriptor with capability fields filled from domain SSOT.
    #[must_use]
    pub fn new(
        name: AgentName,
        path: Option<String>,
        version: Option<String>,
        status: AgentStatus,
    ) -> Self {
        Self {
            name,
            display_name: name.display_name().to_owned(),
            path,
            version,
            status,
            supports_model_selection: name.supports_model_selection(),
            supports_reasoning_effort: name.supports_reasoning_effort(),
        }
    }
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
        let s = serde_json::to_string(&AgentStatus::Error {
            reason: "boom".into(),
        })
        .unwrap();
        assert_eq!(s, r#"{"kind":"error","reason":"boom"}"#);
    }

    #[test]
    fn agent_descriptor_round_trips() {
        let d = AgentDescriptor::new(
            AgentName::Claude,
            Some("/usr/local/bin/claude".into()),
            Some("1.2.0".into()),
            AgentStatus::Ok,
        );
        let json = serde_json::to_string(&d).unwrap();
        let back: AgentDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn agent_name_all_returns_five_in_canonical_order() {
        assert_eq!(AgentName::all().len(), 5);
        assert_eq!(AgentName::all()[0], AgentName::Codex);
        assert_eq!(AgentName::all()[4], AgentName::Grok);
    }

    #[test]
    fn every_agent_has_capability_metadata() {
        for &agent in AgentName::all() {
            assert!(!agent.bin_name().is_empty(), "{agent:?} bin_name");
            assert!(!agent.display_name().is_empty(), "{agent:?} display_name");
            // Exhaustive capability reads — panics if a match arm is missing.
            let _ = agent.supports_model_selection();
            let _ = agent.supports_reasoning_effort();
            let _ = agent.model_discovery();
        }
    }

    #[test]
    fn reasoning_effort_supported_only_by_codex_and_grok() {
        assert!(AgentName::Codex.supports_reasoning_effort());
        assert!(AgentName::Grok.supports_reasoning_effort());
        assert!(!AgentName::Claude.supports_reasoning_effort());
        assert!(!AgentName::Gemini.supports_reasoning_effort());
        assert!(!AgentName::Opencode.supports_reasoning_effort());
    }

    #[test]
    fn model_discovery_matches_runtime_strategy() {
        assert_eq!(
            AgentName::Codex.model_discovery(),
            ModelDiscovery::AppServer
        );
        assert_eq!(AgentName::Claude.model_discovery(), ModelDiscovery::Static);
        assert_eq!(AgentName::Gemini.model_discovery(), ModelDiscovery::Static);
        assert_eq!(AgentName::Opencode.model_discovery(), ModelDiscovery::Cli);
        assert_eq!(AgentName::Grok.model_discovery(), ModelDiscovery::Cli);
    }

    #[test]
    fn descriptor_new_fills_capabilities_from_name() {
        let d = AgentDescriptor::new(AgentName::Grok, None, None, AgentStatus::Missing);
        assert_eq!(d.display_name, "Grok");
        assert!(d.supports_model_selection);
        assert!(d.supports_reasoning_effort);

        let claude = AgentDescriptor::new(AgentName::Claude, None, None, AgentStatus::Missing);
        assert_eq!(claude.display_name, "Claude");
        assert!(!claude.supports_reasoning_effort);
    }

    #[test]
    fn display_names_are_stable() {
        assert_eq!(AgentName::Codex.display_name(), "Codex");
        assert_eq!(AgentName::Opencode.display_name(), "OpenCode");
    }
}
