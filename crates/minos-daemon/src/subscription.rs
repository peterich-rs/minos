//! UniFFI bridge for connection-state streaming.
//!
//! Rust consumers use `DaemonHandle::events_stream()` to get a raw
//! `watch::Receiver`. UniFFI consumers (Swift) use the push-model
//! `DaemonHandle::subscribe(observer)` + `Subscription::cancel()` because
//! Tokio types cannot cross the FFI boundary.

use std::sync::{Arc, Mutex};

use minos_agent_runtime::AgentState;
use minos_domain::ConnectionState;
use tokio::sync::{oneshot, watch};

/// Opaque subscription handle. Swift holds this and calls `cancel` to
/// tear down the observer task at app shutdown or menu teardown.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct Subscription {
    cancel_tx: Mutex<Option<oneshot::Sender<()>>>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl Subscription {
    /// Cancel the observer task. Idempotent.
    pub fn cancel(&self) {
        if let Some(tx) = self.cancel_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

impl Subscription {
    #[must_use]
    pub(crate) fn new(cancel_tx: oneshot::Sender<()>) -> Self {
        Self {
            cancel_tx: Mutex::new(Some(cancel_tx)),
        }
    }
}

/// Foreign-implementable callback. Swift conforms to the generated
/// `ConnectionStateObserver` protocol; Rust calls `on_state` each time
/// `watch::Receiver::changed` fires.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait ConnectionStateObserver: Send + Sync {
    fn on_state(&self, state: ConnectionState);
}

#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait AgentStateObserver: Send + Sync {
    fn on_state(&self, state: AgentState);
}

/// Bridge a Tokio `watch::Receiver<ConnectionState>` to a foreign callback.
/// Returns a `Subscription` whose `cancel` stops the spawned task.
///
/// Called from `DaemonHandle::subscribe`; kept in its own module for
/// testability.
pub(crate) fn spawn_observer(
    mut rx: watch::Receiver<ConnectionState>,
    observer: Arc<dyn ConnectionStateObserver>,
) -> Arc<Subscription> {
    // Emit the current snapshot so Swift has a starting value.
    observer.on_state(*rx.borrow());

    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => break,
                r = rx.changed() => {
                    if r.is_err() {
                        break; // sender dropped
                    }
                    let state = *rx.borrow();
                    observer.on_state(state);
                }
            }
        }
    });
    Arc::new(Subscription::new(cancel_tx))
}

pub(crate) fn spawn_agent_observer(
    mut rx: watch::Receiver<AgentState>,
    observer: Arc<dyn AgentStateObserver>,
) -> Arc<Subscription> {
    observer.on_state(rx.borrow().clone());

    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => break,
                r = rx.changed() => {
                    if r.is_err() {
                        break;
                    }
                    observer.on_state(rx.borrow().clone());
                }
            }
        }
    });
    Arc::new(Subscription::new(cancel_tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    struct CountingObserver {
        hits: Arc<AtomicU32>,
    }

    impl ConnectionStateObserver for CountingObserver {
        fn on_state(&self, _: ConnectionState) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct CountingAgentObserver {
        hits: Arc<AtomicU32>,
    }

    impl AgentStateObserver for CountingAgentObserver {
        fn on_state(&self, _: AgentState) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn observer_receives_initial_and_subsequent_states() {
        let (tx, rx) = watch::channel(ConnectionState::Disconnected);
        let hits = Arc::new(AtomicU32::new(0));
        let obs = Arc::new(CountingObserver { hits: hits.clone() });

        let sub = spawn_observer(rx, obs);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(hits.load(Ordering::SeqCst) >= 1, "initial snapshot missed");

        tx.send(ConnectionState::Pairing).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(hits.load(Ordering::SeqCst) >= 2, "change not delivered");

        sub.cancel();
        let hits_before_cancel_send = hits.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;

        // After cancel, further sends must not increment hits. The watch
        // sender returns `Err` when its last receiver drops (the cancelled
        // task dropped `rx`); that's expected and not a failure of the
        // property under test.
        let _ = tx.send(ConnectionState::Connected);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            hits_before_cancel_send,
            "observer should have stopped after cancel"
        );
    }

    #[tokio::test]
    async fn cancel_is_idempotent() {
        let (_tx, rx) = watch::channel(ConnectionState::Disconnected);
        let hits = Arc::new(AtomicU32::new(0));
        let obs = Arc::new(CountingObserver { hits });
        let sub = spawn_observer(rx, obs);
        sub.cancel();
        sub.cancel(); // must not panic
    }

    #[tokio::test]
    async fn agent_observer_receives_initial_and_subsequent_states() {
        let (tx, rx) = watch::channel(AgentState::Idle);
        let hits = Arc::new(AtomicU32::new(0));
        let obs = Arc::new(CountingAgentObserver { hits: hits.clone() });

        let sub = spawn_agent_observer(rx, obs);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(hits.load(Ordering::SeqCst) >= 1, "initial snapshot missed");

        tx.send(AgentState::Starting {
            agent: AgentName::Codex,
        })
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(hits.load(Ordering::SeqCst) >= 2, "change not delivered");

        sub.cancel();
        let hits_before_cancel_send = hits.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;

        let _ = tx.send(AgentState::Stopping);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            hits_before_cancel_send,
            "observer should have stopped after cancel"
        );
    }

    #[tokio::test]
    async fn agent_cancel_is_idempotent() {
        let (_tx, rx) = watch::channel(AgentState::Idle);
        let hits = Arc::new(AtomicU32::new(0));
        let obs = Arc::new(CountingAgentObserver { hits });
        let sub = spawn_agent_observer(rx, obs);
        sub.cancel();
        sub.cancel();
    }
}
