//! Pair through the real `MobileClient` against a real `DaemonHandle`,
//! all in one process. Verifies the symmetric round trip.

use std::net::SocketAddr;
use std::sync::Arc;

use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_mobile::{InMemoryPairingStore, MobileClient};

#[tokio::test]
async fn mobile_pairs_with_daemon_and_lists_clis() {
    // Redirect HOME so the daemon's default file store doesn't pollute
    // the developer's actual ~/Library/Application Support/minos/.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());

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
    let resp = mobile.pair_with(qr).await.unwrap();
    assert_eq!(resp.mac_name, "MacForTest");
    assert!(resp.ok);

    let clis = mobile.list_clis().await.unwrap();
    assert_eq!(clis.len(), 3);

    daemon.stop().await.unwrap();
}
