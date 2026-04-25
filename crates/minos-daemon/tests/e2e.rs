//! End-to-end: start daemon → connect a fake mobile (jsonrpsee ws-client) →
//! call `pair` → call `list_clis` → tear down. No FFI involved; this test is
//! the pre-FFI MVP confidence anchor.

// `ENV_GUARD` serializes parallel tests on the process-global
// `MINOS_DATA_DIR`; the std mutex is intentional, with the lock held across
// awaits for the lifetime of one daemon under test.
#![allow(clippy::await_holding_lock)]

use std::net::SocketAddr;
use std::sync::Mutex;

use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_domain::{ConnectionState, DeviceId, PairingToken};
use minos_protocol::{MinosRpcClient, PairRequest, PairResponse};

/// `MINOS_DATA_DIR` is process-global; cargo test runs siblings in parallel
/// by default. Hold this mutex for the lifetime of any test that mutates
/// the env var so the daemon under test always observes its own tempdir.
static ENV_GUARD: Mutex<()> = Mutex::new(());

async fn pair_client(
    handle: &DaemonHandle,
    device_id: DeviceId,
    name: &str,
) -> (WsClient, PairResponse) {
    let qr = handle.pairing_qr().unwrap();
    assert_eq!(handle.current_state(), ConnectionState::Pairing);
    let url = format!("ws://{}", handle.addr());
    let client = WsClientBuilder::default().build(&url).await.unwrap();
    let resp = MinosRpcClient::pair(
        &client,
        PairRequest {
            device_id,
            name: name.into(),
            token: qr.token,
        },
    )
    .await
    .unwrap();

    (client, resp)
}

#[tokio::test]
async fn pair_then_list_clis_in_process() {
    let _env = ENV_GUARD.lock().unwrap();
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

    let device_id = DeviceId::new();

    // pair (with the token from the QR)
    let (client, pair_resp) = pair_client(&handle, device_id, "test-iphone").await;
    assert_eq!(handle.current_state(), ConnectionState::Connected);
    assert_ne!(pair_resp.peer_device_id, device_id);
    assert_eq!(pair_resp.peer_name, "test-mac");
    assert!(
        !pair_resp.your_device_secret.as_str().is_empty(),
        "typed pair surface must return a device secret"
    );
    let trusted = handle.current_trusted_device().unwrap().unwrap();
    assert_eq!(trusted.host_device_id, Some(pair_resp.peer_device_id));
    assert_eq!(
        trusted.assigned_device_secret,
        Some(pair_resp.your_device_secret.clone())
    );
    assert_eq!(trusted.host, handle.addr().ip().to_string());
    assert_eq!(trusted.port, handle.addr().port());

    // list_clis — three rows (codex/claude/gemini) regardless of host machine
    let clis = MinosRpcClient::list_clis(&client).await.unwrap();
    assert_eq!(clis.len(), 3);

    // Token still in QR (sanity: serialization works through real WS)
    assert_eq!(handle.port(), handle.addr().port());

    // forget_device clears trust + emits Disconnected.
    handle.forget_device(device_id).await.unwrap();
    assert_eq!(handle.current_state(), ConnectionState::Disconnected);

    drop(client);
    handle.stop().await.unwrap();
}

#[tokio::test]
async fn pair_response_reuses_persisted_host_id_and_secret_after_restart() {
    let _env = ENV_GUARD.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", dir.path());

    let device_id = DeviceId::new();
    let first = DaemonHandle::start(DaemonConfig {
        mac_name: "test-mac".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    })
    .await
    .unwrap();
    let first_port = first.addr().port();

    let (first_client, first_resp) = pair_client(&first, device_id, "test-iphone").await;
    let first_trusted_port = first.current_trusted_device().unwrap().unwrap().port;
    assert_eq!(first_trusted_port, first_port);
    drop(first_client);
    first.stop().await.unwrap();
    drop(first);

    let second = DaemonHandle::start(DaemonConfig {
        mac_name: "test-mac".into(),
        bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    })
    .await
    .unwrap();
    let second_port = second.addr().port();

    let (second_client, second_resp) = pair_client(&second, device_id, "test-iphone").await;
    drop(second_client);

    assert_eq!(second_resp.peer_device_id, first_resp.peer_device_id);
    assert_eq!(second_resp.peer_name, first_resp.peer_name);
    assert_eq!(
        second_resp.your_device_secret,
        first_resp.your_device_secret
    );

    // After the restart the OS hands out a fresh ephemeral port; the
    // persisted trusted-device row must be rewritten to that port so the
    // mobile peer can reconnect on the next `start_autobind` cycle. If
    // we only persisted the original port, a stale value would survive
    // the restart and break the QR rebuild path.
    let second_trusted = second.current_trusted_device().unwrap().unwrap();
    assert_eq!(
        second_trusted.port, second_port,
        "second pair must persist the post-restart ephemeral port",
    );

    second.stop().await.unwrap();
}

#[tokio::test]
async fn events_stream_observes_pair_transition() {
    let _env = ENV_GUARD.lock().unwrap();
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
    let _env = ENV_GUARD.lock().unwrap();
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

#[tokio::test]
async fn host_and_port_round_trip_through_config() {
    let _env = ENV_GUARD.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let cfg = minos_daemon::DaemonConfig {
        mac_name: "Host Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let handle = minos_daemon::DaemonHandle::start(cfg).await.unwrap();

    assert_eq!(handle.host(), "127.0.0.1");
    assert!(handle.port() > 0, "OS must pick a real port");
    assert_eq!(handle.addr().ip().to_string(), handle.host());
    assert_eq!(handle.addr().port(), handle.port());

    handle.stop().await.unwrap();
}
