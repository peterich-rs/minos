//! Tests for DaemonHandle::start_autobind port-retry logic.
//!
//! These tests don't exercise `start_autobind` directly on CI because CI
//! runners lack Tailscale; `discover_tailscale_ip()` returns None, so
//! `start_autobind` returns `BindFailed { addr: "tailscale" }`. We test
//! both the explicit-bind happy path and the no-tailscale failure path.

#[tokio::test]
async fn start_succeeds_when_first_port_free() {
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
async fn current_trusted_device_empty_then_populated() {
    use chrono::Utc;
    use minos_domain::DeviceId;
    use minos_pairing::{PairingStore, TrustedDevice};
    use std::sync::Arc;

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
        host: "100.64.0.42".into(),
        port: 7878,
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
    handle.stop().await.unwrap();
}
