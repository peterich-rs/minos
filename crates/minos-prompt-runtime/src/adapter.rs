use crate::compile::CompiledPromptBundle;

/// Delivery surface identity recorded in [`crate::PromptProvenance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptAdapterId {
    CodexDeveloperInstructions,
    ClaudeAppendSystemPrompt,
    GrokRules,
    /// Runtime has no proven injection entry yet (Gemini / OpenCode until Task C).
    Unsupported,
}

impl PromptAdapterId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CodexDeveloperInstructions => "codex@developer_instructions",
            Self::ClaudeAppendSystemPrompt => "claude@append_system_prompt",
            Self::GrokRules => "grok@rules",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Codex `thread/start.developerInstructions` payload.
///
/// Panics in debug builds if `bundle` was not compiled for Codex (mismatched
/// provenance.adapter_id). Release builds still return the body so a wrong
/// call is observable only via digest/adapter_id in logs.
#[must_use]
pub fn codex_developer_instructions(bundle: &CompiledPromptBundle) -> Option<&str> {
    debug_assert_eq!(
        bundle.provenance.adapter_id,
        PromptAdapterId::CodexDeveloperInstructions.as_str(),
        "codex delivery requires a Codex-compiled bundle"
    );
    bundle.system_instructions.as_deref()
}

/// Claude `--append-system-prompt` value. Omit the flag when `None`.
#[must_use]
pub fn claude_append_system_prompt(bundle: &CompiledPromptBundle) -> Option<&str> {
    debug_assert_eq!(
        bundle.provenance.adapter_id,
        PromptAdapterId::ClaudeAppendSystemPrompt.as_str(),
        "claude delivery requires a Claude-compiled bundle"
    );
    bundle.system_instructions.as_deref()
}

/// Grok top-level `--rules` value. Omit the flag when `None`.
#[must_use]
pub fn grok_rules(bundle: &CompiledPromptBundle) -> Option<&str> {
    debug_assert_eq!(
        bundle.provenance.adapter_id,
        PromptAdapterId::GrokRules.as_str(),
        "grok delivery requires a Grok-compiled bundle"
    );
    bundle.system_instructions.as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{compile_session_context, PromptRuntime, SessionContext};

    #[test]
    fn delivery_helpers_share_system_instructions_body() {
        let codex = compile_session_context(&SessionContext {
            runtime: PromptRuntime::Codex,
            conversation_bound: true,
            profile_instructions: Some("P".into()),
        });
        let claude = compile_session_context(&SessionContext {
            runtime: PromptRuntime::Claude,
            conversation_bound: true,
            profile_instructions: Some("P".into()),
        });
        let grok = compile_session_context(&SessionContext {
            runtime: PromptRuntime::Grok,
            conversation_bound: true,
            profile_instructions: Some("P".into()),
        });
        let expected = codex.system_instructions.as_deref();
        assert_eq!(codex_developer_instructions(&codex), expected);
        assert_eq!(claude_append_system_prompt(&claude), expected);
        assert_eq!(grok_rules(&grok), expected);
    }

    #[test]
    fn empty_bundle_delivers_none() {
        let bundle = compile_session_context(&SessionContext {
            runtime: PromptRuntime::Grok,
            conversation_bound: false,
            profile_instructions: None,
        });
        assert_eq!(grok_rules(&bundle), None);
    }
}
