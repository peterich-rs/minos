use crate::adapter::PromptAdapterId;
use crate::digest::{normalize_fragment, sha256_hex};
use crate::package::{TEAMWORK_BOOTSTRAP, TEAMWORK_PACKAGE_ID, TEAMWORK_PACKAGE_VERSION};

/// Which host runtime will deliver the compiled bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptRuntime {
    Codex,
    Claude,
    Grok,
    /// Not delivered by Task A adapters; compile still produces digests for tests.
    Gemini,
    Opencode,
}

impl PromptRuntime {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Grok => "grok",
            Self::Gemini => "gemini",
            Self::Opencode => "opencode",
        }
    }

    #[must_use]
    pub const fn adapter_id(self) -> PromptAdapterId {
        match self {
            Self::Codex => PromptAdapterId::CodexDeveloperInstructions,
            Self::Claude => PromptAdapterId::ClaudeAppendSystemPrompt,
            Self::Grok => PromptAdapterId::GrokRules,
            // Placeholders until Task C proves a real injection entry.
            Self::Gemini => PromptAdapterId::Unsupported,
            Self::Opencode => PromptAdapterId::Unsupported,
        }
    }
}

/// Inputs that fully determine a compiled session prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    pub runtime: PromptRuntime,
    /// True when the agent session is bound to a Minos conversation (teamwork MCP).
    /// Bootstrap activation is decided **only** here — adapters must not re-check.
    pub conversation_bound: bool,
    /// Host profile / launch instructions (user-defined role text).
    pub profile_instructions: Option<String>,
}

/// Provenance frozen at compile time for observability and session binding (Task D).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptProvenance {
    pub package_id: String,
    pub package_version: String,
    pub compiler_version: String,
    pub runtime: String,
    pub adapter_id: String,
    pub conversation_bound: bool,
    /// SHA-256 of normalized bootstrap when activated; `None` when bootstrap omitted.
    pub bootstrap_digest: Option<String>,
    /// SHA-256 of the full compiled system instructions material (ordered parts).
    pub compiled_digest: String,
}

/// Output of [`compile_session_context`]. Adapters only consume this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPromptBundle {
    /// Teamwork bootstrap when `conversation_bound`; otherwise `None`.
    pub bootstrap: Option<String>,
    /// Normalized profile instructions when non-empty.
    pub profile: Option<String>,
    /// Deterministic join of bootstrap then profile (double newline). Empty → `None`.
    pub system_instructions: Option<String>,
    pub provenance: PromptProvenance,
}

impl CompiledPromptBundle {
    /// Whether any system-level text should be delivered to the runtime.
    #[must_use]
    pub fn has_system_instructions(&self) -> bool {
        self.system_instructions.is_some()
    }
}

/// Compile a session prompt. Pure and deterministic for equal inputs.
#[must_use]
pub fn compile_session_context(ctx: &SessionContext) -> CompiledPromptBundle {
    let bootstrap = if ctx.conversation_bound {
        Some(normalize_fragment(TEAMWORK_BOOTSTRAP))
    } else {
        None
    };

    let profile = ctx
        .profile_instructions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_fragment);

    let system_instructions = join_system_parts(bootstrap.as_deref(), profile.as_deref());

    let bootstrap_digest = bootstrap.as_ref().map(|b| sha256_hex(b.as_bytes()));
    let compiled_material = system_instructions.as_deref().unwrap_or("");
    // Digest covers activation + ordered parts so "no instructions" is also stable.
    let digest_input = format!(
        "v{compiler}|{package}@{version}|bound={bound}|runtime={runtime}|body={body}",
        compiler = crate::COMPILER_VERSION,
        package = TEAMWORK_PACKAGE_ID,
        version = TEAMWORK_PACKAGE_VERSION,
        bound = ctx.conversation_bound,
        runtime = ctx.runtime.as_str(),
        body = compiled_material,
    );
    let compiled_digest = sha256_hex(digest_input.as_bytes());

    let adapter_id = ctx.runtime.adapter_id();
    CompiledPromptBundle {
        bootstrap,
        profile,
        system_instructions,
        provenance: PromptProvenance {
            package_id: TEAMWORK_PACKAGE_ID.to_string(),
            package_version: TEAMWORK_PACKAGE_VERSION.to_string(),
            compiler_version: crate::COMPILER_VERSION.to_string(),
            runtime: ctx.runtime.as_str().to_string(),
            adapter_id: adapter_id.as_str().to_string(),
            conversation_bound: ctx.conversation_bound,
            bootstrap_digest,
            compiled_digest,
        },
    }
}

fn join_system_parts(bootstrap: Option<&str>, profile: Option<&str>) -> Option<String> {
    match (bootstrap, profile) {
        (Some(b), Some(p)) => Some(format!("{b}\n\n{p}")),
        (Some(b), None) => Some(b.to_string()),
        (None, Some(p)) => Some(p.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::TEAMWORK_SEMANTIC_MARKER;
    use pretty_assertions::assert_eq;

    fn ctx(
        runtime: PromptRuntime,
        conversation_bound: bool,
        profile: Option<&str>,
    ) -> SessionContext {
        SessionContext {
            runtime,
            conversation_bound,
            profile_instructions: profile.map(str::to_owned),
        }
    }

    #[test]
    fn conversation_bound_activates_bootstrap_with_semantic_marker() {
        let bundle = compile_session_context(&ctx(PromptRuntime::Codex, true, None));
        let system = bundle.system_instructions.expect("bootstrap present");
        assert!(system.contains(TEAMWORK_SEMANTIC_MARKER));
        assert!(system.contains("minos_teamwork"));
        assert!(system.contains("react_to_message"));
        assert_eq!(bundle.bootstrap.as_deref(), Some(system.as_str()));
        assert!(bundle.provenance.bootstrap_digest.is_some());
        assert!(bundle.provenance.conversation_bound);
    }

    #[test]
    fn unbound_session_omits_bootstrap() {
        let bundle = compile_session_context(&ctx(PromptRuntime::Grok, false, None));
        assert!(bundle.bootstrap.is_none());
        assert!(bundle.system_instructions.is_none());
        assert!(!bundle.provenance.conversation_bound);
        assert!(bundle.provenance.bootstrap_digest.is_none());
    }

    #[test]
    fn profile_only_when_unbound() {
        let bundle =
            compile_session_context(&ctx(PromptRuntime::Claude, false, Some("  Be terse.  ")));
        assert!(bundle.bootstrap.is_none());
        assert_eq!(bundle.system_instructions.as_deref(), Some("Be terse."));
        assert_eq!(bundle.profile.as_deref(), Some("Be terse."));
    }

    #[test]
    fn bootstrap_then_profile_order_is_fixed() {
        let bundle =
            compile_session_context(&ctx(PromptRuntime::Codex, true, Some("Role: reviewer")));
        let system = bundle.system_instructions.expect("joined");
        let boot = bundle.bootstrap.expect("bootstrap");
        assert!(system.starts_with(&boot));
        assert!(system.ends_with("Role: reviewer"));
        assert!(system.contains("\n\nRole: reviewer"));
    }

    #[test]
    fn digest_is_deterministic_and_changes_with_activation() {
        let a = compile_session_context(&ctx(PromptRuntime::Codex, true, Some("x")));
        let b = compile_session_context(&ctx(PromptRuntime::Codex, true, Some("x")));
        assert_eq!(a.provenance.compiled_digest, b.provenance.compiled_digest);

        let unbound = compile_session_context(&ctx(PromptRuntime::Codex, false, Some("x")));
        assert_ne!(
            a.provenance.compiled_digest,
            unbound.provenance.compiled_digest
        );

        let other_profile = compile_session_context(&ctx(PromptRuntime::Codex, true, Some("y")));
        assert_ne!(
            a.provenance.compiled_digest,
            other_profile.provenance.compiled_digest
        );
    }

    #[test]
    fn blank_profile_is_dropped() {
        let bundle = compile_session_context(&ctx(PromptRuntime::Claude, true, Some("  \n  ")));
        assert!(bundle.profile.is_none());
        assert!(bundle.bootstrap.is_some());
        assert_eq!(
            bundle.system_instructions.as_deref(),
            bundle.bootstrap.as_deref()
        );
    }

    #[test]
    fn adapter_ids_match_runtime_delivery_surface() {
        assert_eq!(
            compile_session_context(&ctx(PromptRuntime::Codex, true, None))
                .provenance
                .adapter_id,
            "codex@developer_instructions"
        );
        assert_eq!(
            compile_session_context(&ctx(PromptRuntime::Claude, true, None))
                .provenance
                .adapter_id,
            "claude@append_system_prompt"
        );
        assert_eq!(
            compile_session_context(&ctx(PromptRuntime::Grok, true, None))
                .provenance
                .adapter_id,
            "grok@rules"
        );
    }
}
