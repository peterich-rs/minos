//! Integration-level contract: compiler outputs for Codex / Claude / Grok are
//! stable, share bootstrap body, and map to the documented delivery surfaces.
//!
//! Launch argv builders live as `pub(crate)` unit tests in the crate; this file
//! locks the public `minos-prompt-runtime` seam used by agent-runtime.

use minos_domain::AgentName;
use minos_prompt_runtime::{
    claude_append_system_prompt, codex_developer_instructions, compile_session_context, grok_rules,
    PromptRuntime, SessionContext, TEAMWORK_SEMANTIC_MARKER,
};

fn compile(
    runtime: PromptRuntime,
    conversation_bound: bool,
    profile: Option<&str>,
) -> minos_prompt_runtime::CompiledPromptBundle {
    compile_session_context(&SessionContext {
        runtime,
        conversation_bound,
        profile_instructions: profile.map(str::to_owned),
    })
}

#[test]
fn codex_developer_instructions_carry_marker_profile_and_digest() {
    let bundle = compile(PromptRuntime::Codex, true, Some("Profile: reviewer"));
    let developer = codex_developer_instructions(&bundle).expect("developer instructions");
    assert!(developer.contains(TEAMWORK_SEMANTIC_MARKER));
    assert!(developer.contains("minos_teamwork"));
    assert!(developer.contains("Profile: reviewer"));
    assert_eq!(bundle.provenance.adapter_id, "codex@developer_instructions");
    assert_eq!(bundle.provenance.compiled_digest.len(), 64);
    assert!(bundle.provenance.bootstrap_digest.is_some());
}

#[test]
fn codex_unbound_omits_bootstrap() {
    let unbound = compile(PromptRuntime::Codex, false, None);
    assert!(codex_developer_instructions(&unbound).is_none());

    let profile_bundle = compile(PromptRuntime::Codex, false, Some("Only profile"));
    let profile_only = codex_developer_instructions(&profile_bundle).expect("profile");
    assert_eq!(profile_only, "Only profile");
    assert!(!profile_only.contains(TEAMWORK_SEMANTIC_MARKER));
}

#[test]
fn claude_append_system_matches_codex_body_and_digest_shape() {
    let bundle = compile(PromptRuntime::Claude, true, Some("Be concise."));
    let system = claude_append_system_prompt(&bundle).expect("system");
    assert!(system.contains(TEAMWORK_SEMANTIC_MARKER));
    assert!(system.contains("Be concise."));
    assert_eq!(bundle.provenance.adapter_id, "claude@append_system_prompt");
    assert_eq!(bundle.provenance.compiled_digest.len(), 64);
}

#[test]
fn claude_unbound_without_profile_is_none() {
    let unbound = compile(PromptRuntime::Claude, false, None);
    assert!(claude_append_system_prompt(&unbound).is_none());
}

#[test]
fn grok_rules_match_shared_body() {
    let bundle = compile(PromptRuntime::Grok, true, Some("Ship carefully."));
    let rules = grok_rules(&bundle).expect("rules");
    assert!(rules.contains(TEAMWORK_SEMANTIC_MARKER));
    assert!(rules.contains("Ship carefully."));
    assert_eq!(bundle.provenance.adapter_id, "grok@rules");
    assert_eq!(bundle.provenance.compiled_digest.len(), 64);
}

#[test]
fn three_adapters_share_bootstrap_body() {
    let codex = compile(PromptRuntime::Codex, true, None);
    let claude = compile(PromptRuntime::Claude, true, None);
    let grok = compile(PromptRuntime::Grok, true, None);
    let c = codex_developer_instructions(&codex).unwrap();
    let cl = claude_append_system_prompt(&claude).unwrap();
    let g = grok_rules(&grok).unwrap();
    assert_eq!(c, cl);
    assert_eq!(c, g);
    // Digest includes runtime id, so differs per adapter while bootstrap digest matches.
    assert_ne!(
        codex.provenance.compiled_digest,
        claude.provenance.compiled_digest
    );
    assert_eq!(
        codex.provenance.bootstrap_digest,
        claude.provenance.bootstrap_digest
    );
    assert_eq!(
        codex.provenance.bootstrap_digest,
        grok.provenance.bootstrap_digest
    );
}

#[test]
fn agent_name_matrix_covers_supported_runtimes() {
    // Ensure AgentName variants used by Minos still map in consumer code.
    assert_eq!(AgentName::all().len(), 5);
}
