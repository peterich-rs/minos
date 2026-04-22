//! Prove [`MobileClient::pair_with_json`] is symmetric with
//! [`MobileClient::pair_with`] against a real `DaemonHandle`.
//!
//! Because the pairing token is single-use (consumed by the first successful
//! `pair`), each path pairs with its own fresh in-process daemon. Both paths
//! must produce equivalent `PairResponse`s.

use std::net::SocketAddr;
use std::sync::Arc;

use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_domain::ConnectionState;
use minos_mobile::MobileClient;

async fn start_daemon() -> Arc<DaemonHandle> {
    DaemonHandle::start(DaemonConfig {
        mac_name: "MacForTest".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn pair_with_json_matches_pair_with_against_daemon() {
    // Use MINOS_DATA_DIR override so the daemon's default file store writes
    // into a per-test tempdir without mutating process-global HOME.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", dir.path());

    // Path A: scanned QR JSON through the FFI-friendly entry point.
    let daemon_a = start_daemon().await;
    let qr_a = daemon_a.pairing_qr().unwrap();
    let qr_json = serde_json::to_string(&qr_a).unwrap();

    let via_json = MobileClient::new_with_in_memory_store("iPhoneForTestJson".into());
    assert_eq!(via_json.current_state(), ConnectionState::Disconnected);
    let resp_json = via_json.pair_with_json(qr_json).await.unwrap();
    assert_eq!(via_json.current_state(), ConnectionState::Connected);
    daemon_a.stop().await.unwrap();

    // Path B: parsed QrPayload through the Rust-internal entry point against
    // a fresh daemon (the pairing token is single-use).
    let daemon_b = start_daemon().await;
    let qr_b = daemon_b.pairing_qr().unwrap();

    let via_struct = MobileClient::new_with_in_memory_store("iPhoneForTestStruct".into());
    assert_eq!(via_struct.current_state(), ConnectionState::Disconnected);
    let resp_struct = via_struct.pair_with(qr_b).await.unwrap();
    assert_eq!(via_struct.current_state(), ConnectionState::Connected);
    daemon_b.stop().await.unwrap();

    // Symmetric: both paths produce an equivalent successful response.
    assert_eq!(resp_json.mac_name, "MacForTest");
    assert_eq!(resp_struct.mac_name, "MacForTest");
    assert!(resp_json.ok);
    assert!(resp_struct.ok);
    assert_eq!(resp_json.mac_name, resp_struct.mac_name);
    assert_eq!(resp_json.ok, resp_struct.ok);
}
