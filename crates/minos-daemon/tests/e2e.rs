//! End-to-end: start daemon → connect a fake mobile (jsonrpsee ws-client) →
//! call `pair` → call `list_clis` → tear down. No FFI involved; this test is
//! the pre-FFI MVP confidence anchor.

use std::net::SocketAddr;

use minos_daemon::{DaemonConfig, DaemonHandle};
use minos_domain::DeviceId;
use minos_protocol::{MinosRpcClient, PairRequest};

#[tokio::test]
async fn pair_then_list_clis_in_process() {
    // Redirect HOME so the daemon's default file store doesn't pollute
    // the developer's actual ~/Library/Application Support/minos/.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());

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

    // pair
    let pair_resp = MinosRpcClient::pair(
        &client,
        PairRequest {
            device_id: DeviceId::new(),
            name: "test-iphone".into(),
        },
    )
    .await
    .unwrap();
    assert!(pair_resp.ok);
    assert_eq!(pair_resp.mac_name, "test-mac");

    // list_clis — three rows (codex/claude/gemini) regardless of host machine
    let clis = MinosRpcClient::list_clis(&client).await.unwrap();
    assert_eq!(clis.len(), 3);

    // Token still in QR (sanity: serialization works through real WS)
    assert_eq!(qr.port, handle.addr().port());

    drop(client);
    handle.stop().await.unwrap();
}
