//! Conventions lint: check for the "transaction triple" pattern
//! (domain write + durable_event_log + outbox_events) in service code.
//!
//! Stub implementation — prints OK.

use std::path::Path;

pub fn run(_repo_root: &Path) -> anyhow::Result<()> {
    // Stub: real implementation will scan service code for the transaction
    // triple pattern and flag violations.
    eprintln!("lint-conventions: OK (stub — not yet enforced)");
    Ok(())
}
