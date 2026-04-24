//! Tailscale-era autobind tests — kept as a single ignored placeholder
//! until Phase F.2 deletes the file outright (along with `tailscale.rs`
//! and the old `start_autobind`/`start_on_port_range` entry points on
//! `DaemonHandle`).
//!
//! The original bodies exercised `DaemonHandle::{start(DaemonConfig),
//! start_autobind, start_on_port_range}` plus sync getters `host/port/
//! addr/current_state/current_trusted_device`, all of which the
//! relay-client rewire in F.1 removed. Re-adding equivalent coverage
//! against the new relay-backed handle is tracked in Phase F.6
//! (`tests/e2e.rs` rewrite); that rewrite will also delete this file.

#[tokio::test]
#[ignore = "Phase F rewrite pending: Tailscale-era autobind surface removed in F.1/F.2"]
async fn autobind_surface_removed() {}
