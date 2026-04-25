//! Regression: `DaemonHandle::subscribe_relay_link` and `subscribe_peer`
//! must work when called from an OS thread that has no current Tokio
//! runtime — e.g. Swift's main thread after the UniFFI constructor returned.
//!
//! The guard is the `self.inner.rt_handle.enter()` line in each subscribe
//! method (`crates/minos-daemon/src/handle.rs`). Without it, the
//! `tokio::spawn` inside `spawn_relay_link_observer` /
//! `spawn_peer_observer` panics with "there is no reactor running".
//!
//! The compile-time `BACKEND_URL` defaults to `ws://127.0.0.1:8787/devices`
//! and almost certainly is not actually served during test runs. That's
//! fine: this test only cares about the initial watch snapshot that
//! `spawn_*_observer` emits synchronously and about the `tokio::spawn`
//! inside that helper succeeding. Neither needs a reachable relay.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use minos_daemon::{DaemonHandle, PeerStateObserver, RelayConfig, RelayLinkStateObserver};
use minos_domain::{DeviceId, PeerState, RelayLinkState};
use tokio::runtime::Handle;

/// Captures every `on_state` call onto a shared `Vec`. Constructed with
/// an `Arc<Mutex<Vec<…>>>` so the test can read the captures back after
/// the OS thread has joined.
struct Capture<S> {
    seen: Arc<Mutex<Vec<S>>>,
}

impl RelayLinkStateObserver for Capture<RelayLinkState> {
    fn on_state(&self, state: RelayLinkState) {
        self.seen.lock().unwrap().push(state);
    }
}

impl PeerStateObserver for Capture<PeerState> {
    fn on_state(&self, state: PeerState) {
        self.seen.lock().unwrap().push(state);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_relay_link_and_peer_work_without_current_runtime() {
    let handle = DaemonHandle::start(
        RelayConfig::new(String::new(), String::new()),
        DeviceId::new(),
        None,
        None,
        "test-mac".into(),
    )
    .await
    .expect("start must succeed even when the backend is unreachable");

    let link_seen: Arc<Mutex<Vec<RelayLinkState>>> = Arc::new(Mutex::new(Vec::new()));
    let peer_seen: Arc<Mutex<Vec<PeerState>>> = Arc::new(Mutex::new(Vec::new()));

    let link_obs: Arc<dyn RelayLinkStateObserver> = Arc::new(Capture {
        seen: link_seen.clone(),
    });
    let peer_obs: Arc<dyn PeerStateObserver> = Arc::new(Capture {
        seen: peer_seen.clone(),
    });

    let handle_for_thread = handle.clone();
    let join = std::thread::spawn(move || {
        assert!(
            Handle::try_current().is_err(),
            "test precondition: this OS thread must have no current Tokio runtime"
        );
        // Each call must not panic. The returned Subscription is dropped
        // here — that's fine; both helpers emit the initial snapshot
        // synchronously before spawning the changed()-loop task, and
        // the spawn is what the `rt_handle.enter()` guard protects.
        let _link_sub = handle_for_thread.subscribe_relay_link(link_obs);
        let _peer_sub = handle_for_thread.subscribe_peer(peer_obs);
    });

    join.join().expect("subscribe calls must not panic");

    // Give the runtime a tick to drain any synchronously-queued callbacks
    // (the initial on_state is synchronous so the vec is already non-empty
    // by the time `join()` returns; this only covers the case where a
    // future refactor moved it onto the spawned task).
    tokio::time::sleep(Duration::from_millis(50)).await;

    let link = link_seen.lock().unwrap();
    assert!(
        !link.is_empty(),
        "relay-link observer must receive at least the initial snapshot"
    );
    // The subscribe call races the dispatcher task's first
    // `link_tx.send(Connecting { attempt: 0 })`. Either is acceptable
    // as an "initial snapshot"; what matters for *this* regression is
    // that the call from a non-runtime thread didn't panic and the
    // observer fired at least once.
    assert!(
        matches!(
            link[0],
            RelayLinkState::Disconnected | RelayLinkState::Connecting { attempt: 0 }
        ),
        "initial snapshot should be Disconnected or Connecting{{0}}, got {:?}",
        link[0]
    );

    let peer = peer_seen.lock().unwrap();
    assert!(
        !peer.is_empty(),
        "peer observer must receive at least the initial snapshot"
    );
    assert_eq!(
        peer[0],
        PeerState::Unpaired,
        "initial snapshot should be Unpaired with no persisted peer, got {:?}",
        peer[0]
    );
}
