//! Canonical `minos.teamwork` package assets (Task B SSOT).
//!
//! All teamwork bootstrap / MCP initialize instructions / skill body text
//! lives under `packages/minos.teamwork/`. Consumers `include_str!` only these
//! artifacts — never hand-copy overlapping prose into manager, MCP server, or TUI.

use crate::digest::{normalize_fragment, sha256_hex};

/// Embedded package.yaml (manifest).
pub const TEAMWORK_PACKAGE_MANIFEST: &str = include_str!("../packages/minos.teamwork/package.yaml");

/// Package identity (must match `package.yaml` `id:` — enforced by unit tests).
pub const TEAMWORK_PACKAGE_ID: &str = "minos.teamwork";

/// Package semver (must match `package.yaml` `version:`).
pub const TEAMWORK_PACKAGE_VERSION: &str = "1.0.0";

/// Manifest schema version (must match `package.yaml` `schema_version:`).
pub const TEAMWORK_SCHEMA_VERSION: u32 = 1;

/// Session-start bootstrap fragment (conversation-bound system injection).
pub const TEAMWORK_BOOTSTRAP: &str =
    include_str!("../packages/minos.teamwork/fragments/bootstrap.md");

/// MCP `initialize.instructions` body for `minos_teamwork`.
pub const TEAMWORK_MCP_SERVER_INSTRUCTIONS: &str =
    include_str!("../packages/minos.teamwork/fragments/mcp_server_instructions.md");

/// Full skill body installed into CLI skill directories (`SKILL.md`).
pub const TEAMWORK_SKILL_MD: &str =
    include_str!("../packages/minos.teamwork/fragments/skill/SKILL.md");

/// Skill install id / directory name under `~/.agents/skills/` etc.
pub const TEAMWORK_SKILL_ID: &str = "minos-teamwork";

/// Stable phrase present in bootstrap for adapter contract assertions.
pub const TEAMWORK_SEMANTIC_MARKER: &str = "Minos teamwork mode";

/// Source path of the skill relative to the monorepo root (docs + SkillRef).
pub const TEAMWORK_SKILL_REPO_PATH: &str =
    "crates/minos-prompt-runtime/packages/minos.teamwork/fragments/skill/SKILL.md";

// Budgets from package.yaml (duplicated as consts; tests assert YAML match).
pub const BOOTSTRAP_MAX_CHARS: usize = 3200;
pub const MCP_SERVER_INSTRUCTIONS_MAX_CHARS: usize = 2400;
pub const SKILL_MAX_CHARS: usize = 12_000;

/// Content digests of normalized package fragments (stable across platforms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamworkPackageDigests {
    pub bootstrap: String,
    pub mcp_server_instructions: String,
    pub skill: String,
    /// Digest over `id@version|schema|bootstrap|mcp|skill` normalized bodies.
    pub package: String,
}

/// Compute digests for the embedded package (for tests and future reconcile).
#[must_use]
pub fn teamwork_package_digests() -> TeamworkPackageDigests {
    let bootstrap = normalize_fragment(TEAMWORK_BOOTSTRAP);
    let mcp = normalize_fragment(TEAMWORK_MCP_SERVER_INSTRUCTIONS);
    let skill = normalize_fragment(TEAMWORK_SKILL_MD);
    let package_input = format!(
        "{id}@{version}|schema={schema}|bootstrap={bootstrap}|mcp={mcp}|skill={skill}",
        id = TEAMWORK_PACKAGE_ID,
        version = TEAMWORK_PACKAGE_VERSION,
        schema = TEAMWORK_SCHEMA_VERSION,
        bootstrap = bootstrap,
        mcp = mcp,
        skill = skill,
    );
    TeamworkPackageDigests {
        bootstrap: sha256_hex(bootstrap.as_bytes()),
        mcp_server_instructions: sha256_hex(mcp.as_bytes()),
        skill: sha256_hex(skill.as_bytes()),
        package: sha256_hex(package_input.as_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml_field(key: &str) -> String {
        for line in TEAMWORK_PACKAGE_MANIFEST.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
                return rest.trim().trim_matches('"').to_string();
            }
        }
        panic!("package.yaml missing field {key}");
    }

    fn yaml_nested_u32(section: &str, key: &str) -> usize {
        let mut in_section = false;
        for line in TEAMWORK_PACKAGE_MANIFEST.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            if trimmed == format!("{section}:") {
                in_section = true;
                continue;
            }
            if in_section
                && !line.starts_with(' ')
                && !line.starts_with('\t')
                && trimmed.ends_with(':')
            {
                // Next top-level section.
                break;
            }
            if in_section {
                if let Some(rest) = trimmed.strip_prefix(&format!("{key}:")) {
                    return rest
                        .trim()
                        .parse()
                        .unwrap_or_else(|_| panic!("bad int for {section}.{key}"));
                }
            }
        }
        panic!("package.yaml missing {section}.{key}");
    }

    #[test]
    fn constants_match_package_yaml() {
        assert_eq!(yaml_field("id"), TEAMWORK_PACKAGE_ID);
        assert_eq!(yaml_field("version"), TEAMWORK_PACKAGE_VERSION);
        assert_eq!(
            yaml_field("schema_version").parse::<u32>().unwrap(),
            TEAMWORK_SCHEMA_VERSION
        );
        assert_eq!(
            yaml_nested_u32("token_budgets", "bootstrap_max_chars"),
            BOOTSTRAP_MAX_CHARS
        );
        assert_eq!(
            yaml_nested_u32("token_budgets", "mcp_server_instructions_max_chars"),
            MCP_SERVER_INSTRUCTIONS_MAX_CHARS
        );
        assert_eq!(
            yaml_nested_u32("token_budgets", "skill_max_chars"),
            SKILL_MAX_CHARS
        );
    }

    #[test]
    fn fragments_respect_token_budgets() {
        let bootstrap = normalize_fragment(TEAMWORK_BOOTSTRAP);
        let mcp = normalize_fragment(TEAMWORK_MCP_SERVER_INSTRUCTIONS);
        let skill = normalize_fragment(TEAMWORK_SKILL_MD);
        assert!(
            bootstrap.chars().count() <= BOOTSTRAP_MAX_CHARS,
            "bootstrap {} chars > budget {BOOTSTRAP_MAX_CHARS}",
            bootstrap.chars().count()
        );
        assert!(
            mcp.chars().count() <= MCP_SERVER_INSTRUCTIONS_MAX_CHARS,
            "mcp instructions {} chars > budget {MCP_SERVER_INSTRUCTIONS_MAX_CHARS}",
            mcp.chars().count()
        );
        assert!(
            skill.chars().count() <= SKILL_MAX_CHARS,
            "skill {} chars > budget {SKILL_MAX_CHARS}",
            skill.chars().count()
        );
    }

    #[test]
    fn bootstrap_carries_semantic_marker_and_core_tools() {
        assert!(TEAMWORK_BOOTSTRAP.contains(TEAMWORK_SEMANTIC_MARKER));
        assert!(TEAMWORK_BOOTSTRAP.contains("minos_teamwork"));
        assert!(TEAMWORK_BOOTSTRAP.contains("react_to_message"));
        assert!(TEAMWORK_BOOTSTRAP.contains("post_git_update"));
    }

    #[test]
    fn mcp_instructions_cover_tools_without_skill_frontmatter() {
        assert!(TEAMWORK_MCP_SERVER_INSTRUCTIONS.contains("list_conversation_messages"));
        assert!(TEAMWORK_MCP_SERVER_INSTRUCTIONS.contains("delegate_to_agent"));
        assert!(TEAMWORK_MCP_SERVER_INSTRUCTIONS.contains("react_to_message"));
        assert!(!TEAMWORK_MCP_SERVER_INSTRUCTIONS.contains("---"));
        assert!(!TEAMWORK_MCP_SERVER_INSTRUCTIONS.contains("name: minos-teamwork"));
    }

    #[test]
    fn skill_frontmatter_uses_use_when_trigger() {
        let skill = TEAMWORK_SKILL_MD;
        assert!(skill.starts_with("---\n"));
        assert!(skill.contains("name: minos-teamwork"));
        assert!(skill.contains("description: Use when "));
        assert!(skill.contains("minos_teamwork"));
    }

    #[test]
    fn package_digest_is_deterministic() {
        let a = teamwork_package_digests();
        let b = teamwork_package_digests();
        assert_eq!(a, b);
        assert_eq!(a.bootstrap.len(), 64);
        assert_eq!(a.package.len(), 64);
    }

    #[test]
    fn no_hand_copied_overlap_marker_requirement() {
        // Bootstrap is the short inject; skill is the long handbook. Both must
        // mention the MCP server name so contracts stay honest.
        assert!(TEAMWORK_BOOTSTRAP.contains("minos_teamwork"));
        assert!(TEAMWORK_SKILL_MD.contains("minos_teamwork"));
        assert!(TEAMWORK_MCP_SERVER_INSTRUCTIONS.contains("list_conversation_messages"));
    }
}
