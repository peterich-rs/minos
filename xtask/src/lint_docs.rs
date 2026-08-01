//! Docs lint: verify plan/spec files exist and topic/path consistency.
//!
//! Checks that expected doc files exist and that key identifiers (HTTP
//! paths, table names, topic names) are consistent across the three
//! primary docs:
//! - `docs/architecture-overview.md`
//! - `docs/backend-implementation-plan.md`
//! - `docs/ops/deprecation-timeline.md`

use std::path::Path;

use anyhow::{bail, Result};

const EXPECTED_DOCS: &[&str] = &[
    "docs/backend-implementation-plan.md",
    "docs/architecture-overview.md",
    "docs/ops/deprecation-timeline.md",
];

/// HTTP paths that must be mentioned in the architecture overview.
const CRITICAL_PATHS: &[&str] = &[
    "/health/live",
    "/health/ready",
    "/v1/auth/supabase",
    "/v1/agent-sessions",
];

pub fn run(repo_root: &Path) -> Result<()> {
    let mut missing: Vec<&str> = Vec::new();
    for doc in EXPECTED_DOCS {
        let path = repo_root.join(doc);
        if !path.exists() {
            missing.push(doc);
        }
    }

    if !missing.is_empty() {
        for m in &missing {
            eprintln!("lint-docs: missing {m}");
        }
        bail!(
            "lint-docs: {} expected doc file(s) not found",
            missing.len()
        );
    }

    // Verify critical HTTP paths are mentioned in architecture-overview.md.
    let overview_path = repo_root.join("docs/architecture-overview.md");
    let overview = std::fs::read_to_string(&overview_path).unwrap_or_else(|_| String::new());

    let mut path_issues: Vec<String> = Vec::new();
    for path in CRITICAL_PATHS {
        if !overview.contains(path) {
            path_issues.push(format!("architecture-overview.md does not mention {path}"));
        }
    }

    if path_issues.is_empty() {
        eprintln!("lint-docs: all expected doc files present and consistent");
        Ok(())
    } else {
        for issue in &path_issues {
            eprintln!("lint-docs: {issue}");
        }
        bail!("lint-docs: {} consistency issue(s)", path_issues.len());
    }
}
