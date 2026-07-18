//! Naming lint guard: zero `mac_*` / `ios_*` identifiers in protocol-facing code.
//!
//! Phase B of plan 12-agent-session-manager-and-minos-home renames Mac → Host
//! and Ios → Mobile across the protocol, FFI, mobile, daemon, and backend
//! HTTP/store/migration surfaces. This lint catches regressions by scanning
//! the listed roots for the offending identifier patterns. Run as part of
//! `cargo xtask check-all`.
//!
//! Implemented in pure Rust (no external `rg`) so Linux/macOS CI runners do not
//! need ripgrep installed.
use std::fs;
use std::path::Path;

use regex::Regex;

const TARGETS: &[&str] = &[
    "crates/minos-protocol/src",
    "crates/minos-domain/src",
    "crates/minos-ffi-uniffi/src",
    "crates/minos-ffi-frb/src",
    "crates/minos-mobile/src",
    "crates/minos-daemon/src",
    "crates/minos-backend/migrations",
    "crates/minos-backend/src/http",
    "crates/minos-backend/src/store",
];

const PATTERN: &str = r"\b(mac|ios)_(device_id|display_name|client|pairings|host|secret)\b|\bMacSummary\b|\bIosClient\b|MeMacsResponse|account_mac_pairings";

/// SQL migrations that mention the old `mac_*` vocabulary by design.
/// 0011 references `account_mac_pairings` in a comment as the
/// replacement-table; 0012 created that table; 0013 renames it to
/// `account_host_pairings`; 0014 rewrites the role CHECK list to drop
/// `ios-client` in favor of `mobile-client`. They are immutable history
/// that the lint is *not* trying to gate — the rename is enforced going
/// forward.
const HISTORICAL_MIGRATIONS: &[&str] = &[
    "0011_drop_legacy_pairings.sql",
    "0012_account_mac_pairings.sql",
    "0013_rename_account_mac_to_host.sql",
    "0014_rename_role_ios_client_to_mobile_client.sql",
];

pub fn run(repo_root: &Path) -> anyhow::Result<()> {
    let re = Regex::new(PATTERN).expect("lint-naming pattern must compile");
    let mut hits: Vec<String> = Vec::new();
    for t in TARGETS {
        let dir = repo_root.join(t);
        if !dir.exists() {
            continue;
        }
        scan_dir(&dir, &re, &mut hits)?;
    }
    if hits.is_empty() {
        println!("lint-naming: clean");
        Ok(())
    } else {
        for h in &hits {
            println!("{h}");
        }
        anyhow::bail!(
            "lint-naming: {} hits in protocol/FFI/HTTP/SQL surfaces",
            hits.len()
        )
    }
}

fn scan_dir(dir: &Path, re: &Regex, hits: &mut Vec<String>) -> anyhow::Result<()> {
    let entries = fs::read_dir(dir).map_err(|e| {
        anyhow::anyhow!("lint-naming: cannot read {}: {e}", dir.display())
    })?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_dir(&path, re, hits)?;
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
        // Only text sources that historically carried the rename surface.
        let is_source = name.ends_with(".rs")
            || name.ends_with(".sql")
            || name.ends_with(".swift")
            || name.ends_with(".dart");
        if !is_source {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                hits.push(format!("{}:{}:{line}", path.display(), idx + 1));
            }
        }
    }
    Ok(())
}
