//! Regression: Swift-app callers invoke `DaemonHandle::subscribe` from a
//! plain thread with no current Tokio runtime. Before the fix, the internal
//! `tokio::spawn` in `spawn_observer` panicked with "there is no reactor
//! running". This test reproduces that scenario by starting the daemon
//! inside a runtime via `block_on`, then calling `subscribe` from the outer
//! thread after `block_on` has returned (no runtime context active).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use minos_daemon::{ConnectionStateObserver, DaemonConfig, DaemonHandle};
use minos_domain::ConnectionState;

struct Counter(AtomicU32);

impl ConnectionStateObserver for Counter {
    fn on_state(&self, _state: ConnectionState) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn subscribe_from_thread_without_current_runtime() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let rt = tokio::runtime::Runtime::new().unwrap();
    let handle = rt.block_on(async {
        DaemonHandle::start(DaemonConfig {
            mac_name: "subscribe-no-rt".into(),
            bind_addr: "127.0.0.1:0".parse().unwrap(),
        })
        .await
        .unwrap()
    });

    // We are now outside any runtime context — `tokio::runtime::Handle::try_current()`
    // would return `Err`. This mirrors the Swift FFI call path.
    assert!(tokio::runtime::Handle::try_current().is_err());

    let counter = Arc::new(Counter(AtomicU32::new(0)));
    let observer: Arc<dyn ConnectionStateObserver> = counter.clone();

    // Before the fix this panicked with "there is no reactor running".
    let sub = handle.subscribe(observer);

    // Initial snapshot must propagate. The spawned task runs on the captured
    // runtime, so let it poll before we read the counter.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        counter.0.load(Ordering::SeqCst) >= 1,
        "observer should have received the initial snapshot"
    );

    sub.cancel();
    rt.block_on(async {
        handle.stop().await.unwrap();
    });
}
