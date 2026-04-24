//! End-to-end agent-runtime test that drove the old
//! `DaemonHandle::start_with_agent_glue(DaemonConfig, ...)` path. The
//! Phase F.1 rewire removed that entry point along with the bound-socket
//! surface it relied on (`handle.addr()`, `subscribe_events` over the
//! in-process WS server), so the original body cannot compile against
//! the new relay-backed handle.
//!
//! Phase F.6 rebuilds this test on top of the in-process relay fixture
//! used by `relay_client_smoke.rs`. Until then we keep a single
//! `#[ignore]`d placeholder so the test file continues to exist.

#![cfg(feature = "test-support")]

#[tokio::test]
#[ignore = "Phase F rewrite pending: agent-runtime e2e needs relay-backed rewrite in F.6"]
async fn start_send_stream_stop_against_fake_codex_server_placeholder() {}
