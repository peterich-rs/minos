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

pub fn run(repo_root: &Path) -> anyhow::Result<()> {
    let mut hits: Vec<String> = Vec::new();
    for t in TARGETS {
        let dir = repo_root.join(t);
        if !dir.exists() {
            continue;
        }
        let out = Command::new("rg")
            .args(["-n", "--no-heading", "-e", PATTERN, dir.to_str().unwrap()])
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
