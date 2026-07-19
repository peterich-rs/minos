//! Metrics lint: check metric registry drift gate.
//!
//! Stub implementation — prints OK.

use std::path::Path;

pub fn run(_repo_root: &Path) -> anyhow::Result<()> {
    // Stub: real implementation will verify the metric registry matches
    // the expected set defined in the spec.
    eprintln!("lint-metrics: OK (stub — not yet enforced)");
    Ok(())
}
