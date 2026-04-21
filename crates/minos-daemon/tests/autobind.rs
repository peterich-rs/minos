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
