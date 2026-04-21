//! End-to-end: start daemon → connect a fake mobile (jsonrpsee ws-client) →
//! call `pair` → call `list_clis` → tear down. No FFI involved; this test is
//! the pre-FFI MVP confidence anchor.

use std::net::SocketAddr;

use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_domain::{ConnectionState, DeviceId, PairingToken};
use minos_protocol::{MinosRpcClient, PairRequest};

#[tokio::test]
async fn pair_then_list_clis_in_process() {
    // Use MINOS_DATA_DIR override so the daemon's default file store writes
    // into a per-test tempdir without mutating process-global HOME.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", dir.path());

    // Bind to an ephemeral local port to avoid CI port collisions.
    let cfg = DaemonConfig {
        mac_name: "test-mac".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    };
    let handle = DaemonHandle::start(cfg).await.unwrap();

    // Take the QR (puts state into AwaitingPeer).
    let qr = handle.pairing_qr().unwrap();
    let url = format!("ws://{}", handle.addr());

    let client = jsonrpsee::ws_client::WsClientBuilder::default()
        .build(&url)
        .await
        .unwrap();

    // Post-QR, pre-pair: state is Pairing (pairing_qr emits Pairing per spec §6.2).
    assert_eq!(handle.current_state(), ConnectionState::Pairing);
    let device_id = DeviceId::new();

    // pair (with the token from the QR)
    let pair_resp = MinosRpcClient::pair(
        &client,
        PairRequest {
            device_id,
            name: "test-iphone".into(),
            token: qr.token.clone(),
        },
    )
    .await
    .unwrap();
    assert!(pair_resp.ok);
    assert_eq!(pair_resp.mac_name, "test-mac");

    // After pair: events_stream observed Connected (sent by RpcServerImpl::pair).
    assert_eq!(handle.current_state(), ConnectionState::Connected);

    // list_clis — three rows (codex/claude/gemini) regardless of host machine
    let clis = MinosRpcClient::list_clis(&client).await.unwrap();
    assert_eq!(clis.len(), 3);

    // Token still in QR (sanity: serialization works through real WS)
    assert_eq!(qr.port, handle.addr().port());

    // forget_device clears trust + emits Disconnected.
    handle.forget_device(device_id).await.unwrap();
    assert_eq!(handle.current_state(), ConnectionState::Disconnected);

    drop(client);
    handle.stop().await.unwrap();
}

#[tokio::test]
async fn events_stream_observes_pair_transition() {
    // Same setup as pair_then_list_clis_in_process, but the assertion is
    // through the events_stream() Receiver rather than current_state(): we
    // call .changed().await and observe the Connected emission.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", dir.path());

    let handle = DaemonHandle::start(DaemonConfig {
        mac_name: "test-mac".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    })
    .await
    .unwrap();

    let mut events = handle.events_stream();
    // Initial value is Disconnected (set during DaemonHandle::start).
    assert_eq!(*events.borrow_and_update(), ConnectionState::Disconnected);

    let qr = handle.pairing_qr().unwrap();
    let url = format!("ws://{}", handle.addr());
    let client = jsonrpsee::ws_client::WsClientBuilder::default()
        .build(&url)
        .await
        .unwrap();

    let _resp = MinosRpcClient::pair(
        &client,
        PairRequest {
            device_id: DeviceId::new(),
            name: "test-iphone".into(),
            token: qr.token.clone(),
        },
    )
    .await
    .unwrap();

    // pair() emitted Connected; the receiver picks it up.
    tokio::time::timeout(std::time::Duration::from_secs(2), events.changed())
        .await
        .expect("expected a Connected transition within 2s")
        .expect("watch sender alive");
    assert_eq!(*events.borrow(), ConnectionState::Connected);

    drop(client);
    handle.stop().await.unwrap();
}

#[tokio::test]
async fn pair_with_wrong_token_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", dir.path());

    let cfg = DaemonConfig {
        mac_name: "test-mac".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    };
    let handle = DaemonHandle::start(cfg).await.unwrap();
    let _qr = handle.pairing_qr().unwrap(); // generates token; we deliberately ignore it

    let url = format!("ws://{}", handle.addr());
    let client = jsonrpsee::ws_client::WsClientBuilder::default()
        .build(&url)
        .await
        .unwrap();

    // Try to pair with a fresh (wrong) token.
    let result = MinosRpcClient::pair(
        &client,
        PairRequest {
            device_id: DeviceId::new(),
            name: "attacker".into(),
            token: PairingToken::generate(),
        },
    )
    .await;
    assert!(result.is_err(), "wrong token should be rejected");

    // State should still be Pairing — pairing_qr emitted Pairing, pair() rejected
    // before emitting Connected, so no further transition happened.
    assert_eq!(handle.current_state(), ConnectionState::Pairing);

    drop(client);
    handle.stop().await.unwrap();
}
