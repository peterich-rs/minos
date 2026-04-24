//! Pre-relay mobile pair-with-json symmetry test. Phase F.1 removed
//! `DaemonHandle::start(DaemonConfig)` / `pairing_qr` (sync) so the
//! fixture no longer compiles.
//!
//! See `tests/e2e.rs` for the same story: Phase I mobile-relay
//! migration re-enables iOS-side coverage.

#[tokio::test]
#[ignore = "Phase I rewrite pending: MobileClient still uses Tailscale surface; re-enable when iOS migrates to relay"]
async fn pair_with_json_matches_pair_with_against_daemon_placeholder() {}
