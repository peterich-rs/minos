//! Thin bridge from agent-runtime session facts to `minos-prompt-runtime`.
//!
//! All Codex / Claude / Grok system-prompt assembly goes through
//! [`compile_for_session`]. Adapters must not concatenate teamwork/profile
//! themselves.

use minos_domain::AgentName;
use minos_prompt_runtime::{
    compile_session_context, CompiledPromptBundle, PromptRuntime, SessionContext,
};

/// Map a Minos agent kind to the prompt compiler's runtime enum.
///
/// Gemini / OpenCode compile for digest/tests only; Task A does not deliver
/// their system instructions (requires capability probe — Task C).
#[must_use]
pub(crate) fn prompt_runtime_for(agent: AgentName) -> PromptRuntime {
    match agent {
        AgentName::Codex => PromptRuntime::Codex,
        AgentName::Claude => PromptRuntime::Claude,
        AgentName::Grok => PromptRuntime::Grok,
        AgentName::Gemini => PromptRuntime::Gemini,
        AgentName::Opencode => PromptRuntime::Opencode,
    }
}

/// Compile host system instructions for a session.
///
/// `conversation_bound` is the **only** activation signal for teamwork bootstrap.
#[must_use]
pub(crate) fn compile_for_session(
    agent: AgentName,
    conversation_bound: bool,
    profile_instructions: Option<&str>,
) -> CompiledPromptBundle {
    compile_session_context(&SessionContext {
        runtime: prompt_runtime_for(agent),
        conversation_bound,
        profile_instructions: profile_instructions.map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_driver::build_claude_args;
    use crate::grok_driver::build_grok_spawn_args;
    use minos_prompt_runtime::{
        claude_append_system_prompt, codex_developer_instructions, grok_rules,
        TEAMWORK_SEMANTIC_MARKER,
    };

    #[test]
    fn conversation_bound_codex_delivers_bootstrap() {
        let bundle = compile_for_session(AgentName::Codex, true, None);
        let text = codex_developer_instructions(&bundle).expect("instructions");
        assert!(text.contains(TEAMWORK_SEMANTIC_MARKER));
        assert_eq!(bundle.provenance.adapter_id, "codex@developer_instructions");
    }

    #[test]
    fn unbound_claude_profile_only() {
        let bundle = compile_for_session(AgentName::Claude, false, Some("Stay brief."));
        let text = claude_append_system_prompt(&bundle).expect("profile");
        assert_eq!(text, "Stay brief.");
        assert!(!text.contains(TEAMWORK_SEMANTIC_MARKER));
    }

    #[test]
    fn unbound_grok_without_profile_is_empty() {
        let bundle = compile_for_session(AgentName::Grok, false, None);
        assert!(grok_rules(&bundle).is_none());
    }

    /// Contract: Claude argv carries compiler output only (no local re-join).
    #[test]
    fn claude_launch_args_use_compiled_system_prompt() {
        let bundle = compile_for_session(AgentName::Claude, true, Some("Be concise."));
        let system = claude_append_system_prompt(&bundle).expect("system");
        assert_eq!(bundle.provenance.compiled_digest.len(), 64);
        assert!(bundle.provenance.bootstrap_digest.is_some());
        assert_eq!(bundle.provenance.adapter_id, "claude@append_system_prompt");
        let args = build_claude_args(
            "hello",
            Some("session-1"),
            None,
            None,
            None,
            Some(system),
            true,
        );
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--append-system-prompt"
                && pair[1] == system
                && pair[1].contains(TEAMWORK_SEMANTIC_MARKER)
                && pair[1].contains("Be concise.")
        }));
    }

    #[test]
    fn claude_launch_args_omit_system_when_unbound() {
        let bundle = compile_for_session(AgentName::Claude, false, None);
        assert!(claude_append_system_prompt(&bundle).is_none());
        let args = build_claude_args("hello", None, None, None, None, None, true);
        assert!(!args.iter().any(|a| a == "--append-system-prompt"));
    }

    /// Contract: Grok `--rules` is compiler output; flag precedes `agent`.
    #[test]
    fn grok_launch_args_use_compiled_rules() {
        let bundle = compile_for_session(AgentName::Grok, true, Some("Ship carefully."));
        let rules = grok_rules(&bundle).expect("rules");
        assert_eq!(bundle.provenance.compiled_digest.len(), 64);
        assert!(bundle.provenance.bootstrap_digest.is_some());
        assert_eq!(bundle.provenance.adapter_id, "grok@rules");
        let args = build_grok_spawn_args(false, Some(rules), None, None);
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--rules"
                && pair[1] == rules
                && pair[1].contains(TEAMWORK_SEMANTIC_MARKER)
                && pair[1].contains("Ship carefully.")
        }));
        let rules_idx = args.iter().position(|a| a == "--rules").unwrap();
        let agent_idx = args.iter().position(|a| a == "agent").unwrap();
        assert!(rules_idx < agent_idx);
    }

    #[test]
    fn grok_launch_args_omit_rules_when_unbound() {
        let bundle = compile_for_session(AgentName::Grok, false, None);
        assert!(grok_rules(&bundle).is_none());
        let args = build_grok_spawn_args(false, None, None, None);
        assert!(!args.iter().any(|a| a == "--rules"));
    }
}
