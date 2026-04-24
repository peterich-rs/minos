//! Pre-relay mobile-against-daemon e2e. Phase F.1 removed the Tailscale
//! `DaemonHandle::start(DaemonConfig)` entry point, so this test's
//! `DaemonHandle` fixture can't be stood up against the current surface.
//!
//! iOS still uses the legacy jsonrpsee-over-Tailscale stack in-code
//! (`MobileClient`), so the right place to rewrite this is the Phase I
//! mobile-relay migration that ports iOS onto the relay client. Until
//! then, keep the file as an `#[ignore]`d placeholder.

#[tokio::test]
#[ignore = "Phase I rewrite pending: MobileClient still uses Tailscale surface; re-enable when iOS migrates to relay"]
async fn mobile_pairs_with_daemon_and_lists_clis_placeholder() {}
