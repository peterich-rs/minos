//! Tests for DaemonHandle::start_autobind port-retry logic.
//!
//! These tests don't exercise `start_autobind` directly on CI because CI
//! runners lack Tailscale; `discover_tailscale_ip()` returns None, so
//! `start_autobind` returns `BindFailed { addr: "tailscale" }`. We test
//! both the explicit-bind happy path and the no-tailscale failure path.
//!
//! Every test here mutates the process-global `MINOS_DATA_DIR` /
//! `MINOS_LOG_DIR` env vars, so they hold `ENV_LOCK` for the duration of the
//! env-sensitive work. This lock is the reason the file does not rely on
//! cargo test parallelism for isolation.

use tokio::sync::Mutex;

// `tokio::sync::Mutex` guard is `Send`, so it can be held across `.await`.
// We need that because every test below does env setup then awaits daemon
// startup; the lock must span both.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn start_succeeds_when_first_port_free() {
    let _env = ENV_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let cfg = minos_daemon::DaemonConfig {
        mac_name: "Autobind Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let handle = minos_daemon::DaemonHandle::start(cfg).await.unwrap();
    assert!(handle.port() > 0);
    handle.stop().await.unwrap();
}

#[tokio::test]
async fn autobind_returns_bind_failed_without_tailscale() {
    let _env = ENV_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    // Skip if the dev machine has tailscale with a real 100.x IP.
    let ip = minos_daemon::discover_tailscale_ip().await;
    if ip.is_some() {
        eprintln!("skipping — machine has a 100.x IP: {ip:?}");
        return;
    }

    let r = minos_daemon::DaemonHandle::start_autobind("Test Mac".into()).await;
    match r {
        Err(minos_domain::MinosError::BindFailed { addr, .. }) => {
            assert_eq!(addr, "tailscale");
        }
        Ok(_) => panic!("start_autobind should fail without tailscale"),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn start_on_port_range_picks_first_free_port() {
    use std::net::TcpListener;

    let _env = ENV_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    // Occupy 17878..=17880, leave 17881 free. Use 17878..=17881 so the
    // first three bind attempts fail with AddrInUse and the fourth succeeds.
    let _d0 = TcpListener::bind("127.0.0.1:17878").unwrap();
    let _d1 = TcpListener::bind("127.0.0.1:17879").unwrap();
    let _d2 = TcpListener::bind("127.0.0.1:17880").unwrap();

    let handle = minos_daemon::DaemonHandle::start_on_port_range(
        "127.0.0.1".into(),
        "PortRange Test".into(),
        17878..=17881,
    )
    .await
    .expect("start_on_port_range should find the free port");
    assert_eq!(handle.port(), 17881);
    handle.stop().await.unwrap();
}

#[tokio::test]
async fn start_on_port_range_returns_all_occupied_when_range_full() {
    use std::net::TcpListener;

    let _env = ENV_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let _d0 = TcpListener::bind("127.0.0.1:17882").unwrap();
    let _d1 = TcpListener::bind("127.0.0.1:17883").unwrap();
    let _d2 = TcpListener::bind("127.0.0.1:17884").unwrap();

    let r = minos_daemon::DaemonHandle::start_on_port_range(
        "127.0.0.1".into(),
        "PortRange Test".into(),
        17882..=17884,
    )
    .await;
    match r {
        Err(minos_domain::MinosError::BindFailed { addr, message }) => {
            assert_eq!(addr, "127.0.0.1:17882-17884");
            assert!(
                !message.is_empty(),
                "expected non-empty propagated bind-failure message"
            );
        }
        Ok(_) => panic!("start_on_port_range should fail when every port is occupied"),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn stop_is_idempotent() {
    let _env = ENV_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let cfg = minos_daemon::DaemonConfig {
        mac_name: "Stop Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let handle = minos_daemon::DaemonHandle::start(cfg).await.unwrap();

    handle.stop().await.expect("first stop must succeed");
    handle
        .stop()
        .await
        .expect("second stop must be a no-op, not a panic or error");

    assert_eq!(
        handle.current_state(),
        minos_domain::ConnectionState::Disconnected
    );
}

#[tokio::test]
async fn current_trusted_device_empty_then_populated() {
    use chrono::Utc;
    use minos_domain::{DeviceId, DeviceSecret};
    use minos_pairing::{PairingStore, TrustedDevice};
    use std::sync::Arc;

    let _env = ENV_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let cfg = minos_daemon::DaemonConfig {
        mac_name: "TD Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    };
    let handle = minos_daemon::DaemonHandle::start(cfg).await.unwrap();

    // Empty on first start
    assert!(handle.current_trusted_device().unwrap().is_none());

    // Populate via the file store directly (simulates a plan-03 pair flow)
    let store: Arc<dyn PairingStore> = Arc::new(minos_daemon::FilePairingStore::new(
        minos_daemon::FilePairingStore::default_path(),
    ));
    let dev = TrustedDevice {
        device_id: DeviceId::new(),
        name: "iPhone".into(),
        host_device_id: Some(DeviceId::new()),
        host: "100.64.0.42".into(),
        port: 7878,
        assigned_device_secret: Some(DeviceSecret::generate()),
        paired_at: Utc::now(),
    };
    store.save(&[dev.clone()]).unwrap();

    // Re-start with the now-populated store (freeing the old handle first
    // because FilePairingStore default_path resolves from MINOS_DATA_DIR)
    handle.stop().await.unwrap();
    drop(handle);

    let handle = minos_daemon::DaemonHandle::start(minos_daemon::DaemonConfig {
        mac_name: "TD Test".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
    })
    .await
    .unwrap();

    let td = handle.current_trusted_device().unwrap().unwrap();
    assert_eq!(td.device_id, dev.device_id);
    assert_eq!(td.name, dev.name);
    assert_eq!(td.host_device_id, dev.host_device_id);
    assert_eq!(td.assigned_device_secret, dev.assigned_device_secret);
    handle.stop().await.unwrap();
}
