//! Naming lint guard: catches identifier regressions and planning-marker
//! leakage in source code.
//!
//! ## Identifier rules
//!
//! Catches Mac→Host / Ios→Mobile rename regressions.
//!
//! ## Planning-marker rules
//!
//! Catches planning artifacts that must not appear in production code
//! comments: `Phase N`, `B6/C5` task IDs, `spec §X.Y`, spec doc aliases
//! (retired working-spec aliases and paths).
//!
//! Run as part of `cargo xtask check-all`.

use std::fs;
use std::path::Path;

use regex::Regex;

const IDENTIFIER_TARGETS: &[&str] = &[
    "crates/minos-protocol/src",
    "crates/minos-domain/src",
    "crates/minos-ffi-frb/src",
    "crates/minos-mobile/src",
    "crates/minos-daemon/src",
    "crates/minos-backend/migrations",
    "crates/minos-backend/src/http",
    "crates/minos-backend/src/store",
];

const MARKER_TARGETS: &[&str] = &[
    "crates",
    "apps/desktop/src",
    "apps/desktop/src-tauri/src",
    "apps/mobile/lib",
    "xtask/src",
];

/// Identifier patterns: mac_/ios_ remnants.
const IDENTIFIER_PATTERN: &str = r"\b(mac|ios)_(device_id|display_name|client|pairings|host|secret)\b|\bMacSummary\b|\bIosClient\b|MeMacsResponse|account_mac_pairings";

/// Planning-marker patterns in comments. ADR references (ADR-NNNN) are allowed.
const MARKER_PATTERN: &str = concat!(
    // Phase markers: "Phase 1", "Phase A", "Phase 2.5"
    r"(?i)\bPhase\s+[0-9A-Z](?:\.\d+)?\b",
    r"|",
    // Task/ticket IDs: "B6/C5", "C5.3", "P0.S2", "(B2.4)"
    r"\b[BCP]\d{1,2}[./]\d{1,2}\b",
    r"|",
    // Anonymous spec/plan section refs: "spec §6", "plan §10", "§5.4"
    r"(?i)(?:spec|plan)\s*§\d",
    r"|§\d",
    r"|",
    // Spec doc aliases (internal working-doc names)
    r"bot-mailbox-ws-im-bus-design",
    r"|hub-collaboration-message-ssot",
    r"|global-bot-identity-design",
    r"|agent-participant-delivery",
    r"|",
    // Retired working-document path leakage.
    r"docs/superpowers/",
    r"|",
    // Plan number refs: "plan 04", "plan 08a"
    r"(?i)\bplan\s+\d+\w?",
);

/// SQL migrations that mention the old `mac_*` vocabulary by design.
const HISTORICAL_MIGRATIONS: &[&str] = &[
    "0011_drop_legacy_pairings.sql",
    "0012_account_mac_pairings.sql",
    "0013_rename_account_mac_to_host.sql",
    "0014_rename_role_ios_client_to_mobile_client.sql",
];

/// File name patterns that are auto-generated and should not be linted
/// (their content is derived from source files; fix at the ownership boundary).
const GENERATED_SUFFIXES: &[&str] = &[
    ".g.dart",
    "frb_generated.dart",
    "frb_generated.io.dart",
    "frb_generated.rs",
];

pub fn run(repo_root: &Path) -> anyhow::Result<()> {
    let id_re = Regex::new(IDENTIFIER_PATTERN).expect("identifier pattern must compile");
    let marker_re = Regex::new(MARKER_PATTERN).expect("marker pattern must compile");
    let mut hits: Vec<String> = Vec::new();

    for t in IDENTIFIER_TARGETS {
        let dir = repo_root.join(t);
        if dir.exists() {
            scan_dir(&dir, &id_re, false, &mut hits)?;
        }
    }
    for t in MARKER_TARGETS {
        let dir = repo_root.join(t);
        if dir.exists() {
            scan_dir(&dir, &marker_re, true, &mut hits)?;
        }
    }

    if hits.is_empty() {
        println!("lint-naming: clean");
        Ok(())
    } else {
        for h in &hits {
            println!("{h}");
        }
        anyhow::bail!("lint-naming: {} hits", hits.len())
    }
}

fn scan_dir(
    dir: &Path,
    re: &Regex,
    check_markers: bool,
    hits: &mut Vec<String>,
) -> anyhow::Result<()> {
    let entries = fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("lint-naming: cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            // Skip known non-source directories.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name,
                "target" | "node_modules" | ".git" | "generated" | "dist" | ".dart_tool"
            ) {
                continue;
            }
            scan_dir(&path, re, check_markers, hits)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if HISTORICAL_MIGRATIONS.contains(&name) {
            continue;
        }
        if GENERATED_SUFFIXES.iter().any(|s| name.ends_with(s)) {
            continue;
        }
        // This file defines the lint patterns — it legitimately mentions them.
        if name == "lint_naming.rs" {
            continue;
        }
        let is_source = Path::new(name).extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("rs")
                || ext.eq_ignore_ascii_case("ts")
                || ext.eq_ignore_ascii_case("tsx")
                || ext.eq_ignore_ascii_case("dart")
                || ext.eq_ignore_ascii_case("sql")
                || ext.eq_ignore_ascii_case("swift")
        });
        if !is_source {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        'line_loop: for (idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                if check_markers {
                    // ADR references like "ADR 0021 §6" are allowed.
                    if line.contains("ADR") && !line.to_lowercase().contains("phase") {
                        continue 'line_loop;
                    }
                }
                hits.push(format!("{}:{}:{line}", path.display(), idx + 1));
            }
        }
    }
    Ok(())
}
