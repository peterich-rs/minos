//! `minos-prompt-runtime` — deep module for compiling host-owned session prompts.
//!
//! Public surface:
//!
//! - [`compile_session_context`] — deterministic `SessionContext` → [`CompiledPromptBundle`]
//! - adapter delivery helpers — map a bundle onto Codex / Claude / Grok launch surfaces
//! - [`package`] assets — canonical `minos.teamwork` fragments (bootstrap / MCP / skill)
//!
//! Package host reconciliation (`reconcile_host_packages`) and Gemini/OpenCode delivery
//! land in later slices. This crate must not invent provider-specific pseudo fields
//! for runtimes that lack a proven injection entry.

mod adapter;
mod compile;
mod digest;
pub mod package;

pub use adapter::{
    claude_append_system_prompt, codex_developer_instructions, grok_rules, PromptAdapterId,
};
pub use compile::{
    compile_session_context, CompiledPromptBundle, PromptProvenance, PromptRuntime, SessionContext,
};
pub use package::{
    teamwork_package_digests, TeamworkPackageDigests, TEAMWORK_BOOTSTRAP,
    TEAMWORK_MCP_SERVER_INSTRUCTIONS, TEAMWORK_PACKAGE_ID, TEAMWORK_PACKAGE_VERSION,
    TEAMWORK_SCHEMA_VERSION, TEAMWORK_SEMANTIC_MARKER, TEAMWORK_SKILL_ID, TEAMWORK_SKILL_MD,
    TEAMWORK_SKILL_REPO_PATH,
};

/// Compile-time identity of the prompt compiler itself (not package content).
pub const COMPILER_VERSION: &str = "1";
