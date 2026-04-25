//! Pre-relay end-to-end tests (`pair` + `list_clis` over in-process WS).
//! The relay-client rewire in Phase F.1 removed `DaemonHandle::
//! start(DaemonConfig)` + the `pairing_qr`/`events_stream`/`host`/`port`/
//! `current_state` sync getters these tests drove, so the bodies cannot
//! compile against the new surface.
//!
//! Phase F.6 rewrites this file to run `pair` → `list_clis` through a
//! real in-process backend (mirroring the relay_client_smoke pattern).
//! Until then we keep a single `#[ignore]`d placeholder so the file
//! continues to exist for `cargo build --tests`.

#[tokio::test]
#[ignore = "Phase F rewrite pending: pre-relay e2e assertions need backend-backed rewrite in F.6"]
async fn pair_and_list_clis_placeholder() {}
