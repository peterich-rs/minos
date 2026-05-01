//! Naming lint guard: zero `mac_*` / `ios_*` identifiers in protocol-facing code.
//!
//! Phase B of plan 12-agent-session-manager-and-minos-home renames Mac → Host
//! and Ios → Mobile across the protocol, FFI, mobile, daemon, and backend
//! HTTP/store/migration surfaces. This lint catches regressions by scanning
//! the listed roots for the offending identifier patterns. Run as part of
//! `cargo xtask check-all`.
use std::path::Path;
use std::process::Command;

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
    let mut hits: Vec<String> = Vec::new();
    for t in TARGETS {
        let dir = repo_root.join(t);
        if !dir.exists() {
            continue;
        }
        let mut args: Vec<String> = vec![
            "-n".into(),
            "--no-heading".into(),
            "-e".into(),
            PATTERN.into(),
        ];
        for excluded in HISTORICAL_MIGRATIONS {
            args.push("-g".into());
            args.push(format!("!{excluded}"));
        }
        args.push(dir.to_str().unwrap().to_string());
        let out = Command::new("rg")
            .args(args.iter().map(String::as_str))
            .output()?;
        if !out.stdout.is_empty() {
            hits.push(String::from_utf8_lossy(&out.stdout).into_owned());
        }
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
            hits.iter().map(|s| s.lines().count()).sum::<usize>()
        )
    }
}
