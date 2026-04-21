//! Pair through the real `MobileClient` against a real `DaemonHandle`,
//! all in one process. Verifies the symmetric round trip.

use std::net::SocketAddr;
use std::sync::Arc;

use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_domain::ConnectionState;
use minos_mobile::{InMemoryPairingStore, MobileClient};

#[tokio::test]
async fn mobile_pairs_with_daemon_and_lists_clis() {
    // Use MINOS_DATA_DIR override so the daemon's default file store writes
    // into a per-test tempdir without mutating process-global HOME.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", dir.path());

    let daemon = DaemonHandle::start(DaemonConfig {
        mac_name: "MacForTest".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    })
    .await
    .unwrap();

    let qr = daemon.pairing_qr().unwrap();

    let mobile = MobileClient::new(
        Arc::new(InMemoryPairingStore::new()),
        "iPhoneForTest".into(),
    );

    // Pre-pair: client is Disconnected.
    assert_eq!(mobile.current_state(), ConnectionState::Disconnected);

    let resp = mobile.pair_with(qr).await.unwrap();
    assert_eq!(resp.mac_name, "MacForTest");
    assert!(resp.ok);

    // Post-pair: client is Connected.
    assert_eq!(mobile.current_state(), ConnectionState::Connected);

    let clis = mobile.list_clis().await.unwrap();
    assert_eq!(clis.len(), 3);

    // forget_device clears trust + drops the WS + emits Disconnected.
    mobile.forget_device().await.unwrap();
    assert_eq!(mobile.current_state(), ConnectionState::Disconnected);

    daemon.stop().await.unwrap();
}
