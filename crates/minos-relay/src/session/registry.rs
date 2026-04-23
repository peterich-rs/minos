//! In-memory registry of live device sessions with bounded per-peer outboxes.
//!
//! # Model
//!
//! For every authenticated WebSocket, the relay constructs a
//! [`SessionHandle`] and inserts it into a shared [`SessionRegistry`].
//! The handle carries:
//!
//! - the session's [`DeviceId`] (also the registry key);
//! - a `paired_with` slot (`Arc<RwLock<Option<DeviceId>>>`) that mirrors the
//!   current pairing state for this session. The relay updates this when
//!   `consume_token` or `forget_peer` changes the DB — step 8 wires the
//!   update path;
//! - an **outbox**: `tokio::sync::mpsc::Sender<ServerFrame>` handed to the
//!   per-socket writer task. Anything that wants to push a frame to this
//!   device calls `registry.route(..)` or picks up the handle via `get`
//!   and uses the sender directly.
//!
//! Values are cheap to clone (one `Arc` + one `mpsc::Sender` bump), so the
//! API returns owned clones rather than `DashMap` guards. This keeps
//! callers off the DashMap shard lock while they do I/O work.
//!
//! # Backpressure (MVP)
//!
//! The outbox has a fixed capacity of [`OUTBOX_CAPACITY`] = 256 frames. On
//! a slow consumer the channel fills up and `route()` has three options:
//!
//! 1. **Drop the newest frame** (what we do today). `mpsc::Sender::try_send`
//!    returns `TrySendError::Full`; we emit `tracing::warn!` and return
//!    `Ok(())`.
//! 2. **Drop the oldest frame** (what the plan §7 bullet asks for). This
//!    requires popping one element off the receiver side, which the
//!    registry does not own — only the writer task does. A true
//!    drop-oldest policy must therefore live in the writer loop, not here.
//! 3. **Queue across disconnect** (P1, out of scope for MVP).
//!
//! Option 1 is the pragmatic MVP default: it keeps the registry thin and
//! avoids a second channel layer just to drain the tail. If step 12's e2e
//! test reveals that drop-newest causes stale-state bugs (e.g. the newest
//! `peer_online` is dropped), revisit by threading the receiver's
//! `try_recv` into a `drain_one_then_retry` path owned by the writer.
//!
//! On [`TrySendError::Closed`] we translate to [`RelayError::PeerOffline`].
//! The receiver going away means the writer task has shut down; from the
//! caller's perspective the peer is effectively offline.

use std::sync::Arc;

use dashmap::DashMap;
use minos_domain::DeviceId;
use minos_protocol::Envelope;
use tokio::sync::{mpsc, RwLock};

use crate::error::RelayError;

/// Outbox capacity in frames. 256 matches spec §7 bullet "Bounded mpsc".
///
/// Sized for "a few seconds of chatty pair traffic before the TCP send
/// buffer drains". A reasonable default; tune later with e2e data.
pub const OUTBOX_CAPACITY: usize = 256;

/// A frame queued for push from the relay to a specific device.
///
/// Aliased to [`Envelope`] directly rather than wrapped in a narrower enum
/// — the envelope already carries the discriminator (`Forwarded`, `Event`,
/// `LocalRpcResponse`) the writer needs. The alias exists so call sites
/// at the registry surface can say "ServerFrame" (intent) without tying
/// themselves to the envelope type forever; if we ever need to attach
/// metadata (e.g. send-timestamp) it becomes a newtype around `Envelope`.
pub type ServerFrame = Envelope;

/// One live WebSocket session, indexed by its [`DeviceId`].
///
/// Constructed by the WS accept handler (step 8); removed by the same
/// handler on close. Cheaply clonable — clones share the outbox `Sender`
/// and the `paired_with` lock.
#[derive(Debug, Clone)]
pub struct SessionHandle {
    /// Identity of the device owning this session. Also the registry key.
    pub device_id: DeviceId,
    /// The peer this session is currently paired with, if any.
    ///
    /// Wrapped in `Arc<RwLock<_>>` so the registry holder and the WS
    /// reader/writer tasks can all observe / update pairing transitions
    /// (e.g. `Event::Paired`, `Event::Unpaired`) without re-issuing a
    /// whole new handle.
    pub paired_with: Arc<RwLock<Option<DeviceId>>>,
    /// Bounded outbox to the per-socket writer task.
    ///
    /// Sender end only; the receiver lives inside the writer. `Clone` on
    /// `mpsc::Sender` is a cheap `Arc` bump.
    pub outbox: mpsc::Sender<ServerFrame>,
}

/// Concurrent, lock-sharded map of `DeviceId → SessionHandle`.
///
/// `DashMap` gives us per-shard locking; the registry itself is cheap to
/// clone (just an `Arc` bump) so it can be handed to every async task that
/// needs to push frames.
#[derive(Debug, Clone, Default)]
pub struct SessionRegistry(Arc<DashMap<DeviceId, SessionHandle>>);

impl SessionRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a handle for `handle.device_id`.
    ///
    /// Returns the previous handle if the device already had one (e.g. the
    /// peer reconnected before we noticed its old socket dropped). The
    /// caller should typically drop the returned handle to close the old
    /// outbox and shut the prior writer task.
    pub fn insert(&self, handle: SessionHandle) -> Option<SessionHandle> {
        self.0.insert(handle.device_id, handle)
    }

    /// Remove and return the handle for `id`, or `None` if none was live.
    pub fn remove(&self, id: DeviceId) -> Option<SessionHandle> {
        self.0.remove(&id).map(|(_k, v)| v)
    }

    /// Clone the handle for `id` if a session is live.
    ///
    /// Returns a clone (cheap: one `Arc` bump on each field) rather than
    /// a `DashMap::Ref` guard so callers can perform async I/O without
    /// holding the shard lock.
    pub fn get(&self, id: DeviceId) -> Option<SessionHandle> {
        self.0.get(&id).map(|r| r.clone())
    }

    /// Current number of live sessions. Useful for metrics and tests;
    /// O(#shards) under the hood.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True if no sessions are live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Route `payload` from `from` to `to` as an [`Envelope::Forwarded`].
    ///
    /// Behaviour:
    /// - `to` not in the registry → [`RelayError::PeerOffline`].
    /// - Outbox accepts → `Ok(())`.
    /// - Outbox full → emit `tracing::warn!` and return `Ok(())`. See the
    ///   module-level "Backpressure (MVP)" note for the rationale.
    /// - Outbox closed (receiver dropped) → remove the stale handle and
    ///   return [`RelayError::PeerOffline`]. The close signals the writer
    ///   task has shut down, so the handle is effectively stale.
    ///
    /// # Errors
    ///
    /// See variants above.
    ///
    /// The function is declared `async` on purpose even though today's
    /// body uses only `try_send` (sync). The plan §7 signature is `async`,
    /// and step 8 may introduce a bounded `send_timeout` or a
    /// drain-one-then-retry path for true drop-oldest backpressure —
    /// both of which need `.await`. Keeping the signature stable now
    /// avoids churning every call site later.
    #[allow(clippy::unused_async)]
    pub async fn route(
        &self,
        from: DeviceId,
        to: DeviceId,
        payload: serde_json::Value,
    ) -> Result<(), RelayError> {
        let Some(handle) = self.get(to) else {
            return Err(RelayError::PeerOffline {
                peer_device_id: to.0.to_string(),
            });
        };

        let frame = Envelope::Forwarded {
            version: 1,
            from,
            payload,
        };

        match handle.outbox.try_send(frame) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    target: "minos_relay::session",
                    to = %to.0,
                    from = %from.0,
                    "outbox full; dropping forwarded frame (MVP drop-newest policy)"
                );
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Receiver dropped — session is effectively gone. Clean up
                // the stale entry so subsequent routes skip the dead
                // handle entirely. We `remove` rather than assume the
                // caller does it because any number of callers could
                // observe `Closed` first.
                self.0.remove(&to);
                Err(RelayError::PeerOffline {
                    peer_device_id: to.0.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_handle(id: DeviceId) -> (SessionHandle, mpsc::Receiver<ServerFrame>) {
        let (tx, rx) = mpsc::channel(OUTBOX_CAPACITY);
        (
            SessionHandle {
                device_id: id,
                paired_with: Arc::new(RwLock::new(None)),
                outbox: tx,
            },
            rx,
        )
    }

    // Small outbox variant so we can fill it deterministically in tests.
    fn make_tiny_handle(id: DeviceId, cap: usize) -> (SessionHandle, mpsc::Receiver<ServerFrame>) {
        let (tx, rx) = mpsc::channel(cap);
        (
            SessionHandle {
                device_id: id,
                paired_with: Arc::new(RwLock::new(None)),
                outbox: tx,
            },
            rx,
        )
    }

    // ── insert / remove / get round-trip ──────────────────────────────

    #[tokio::test]
    async fn insert_then_get_round_trips_handle() {
        let reg = SessionRegistry::new();
        let id = DeviceId::new();
        let (h, _rx) = make_handle(id);

        assert!(reg.insert(h.clone()).is_none());
        let got = reg.get(id).expect("session registered");
        assert_eq!(got.device_id, id);
        // Clones share the same outbox sender (cheap Arc bump).
        assert!(
            got.outbox.same_channel(&h.outbox),
            "clone must share underlying mpsc channel"
        );
    }

    #[tokio::test]
    async fn remove_returns_handle_and_subsequent_get_is_none() {
        let reg = SessionRegistry::new();
        let id = DeviceId::new();
        let (h, _rx) = make_handle(id);
        reg.insert(h);

        let removed = reg.remove(id).expect("session existed");
        assert_eq!(removed.device_id, id);
        assert!(reg.get(id).is_none());
        assert!(reg.is_empty());
    }

    #[tokio::test]
    async fn insert_duplicate_replaces_previous_handle() {
        let reg = SessionRegistry::new();
        let id = DeviceId::new();
        let (h1, _rx1) = make_handle(id);
        let (h2, _rx2) = make_handle(id);

        assert!(reg.insert(h1.clone()).is_none());
        let prev = reg
            .insert(h2.clone())
            .expect("replace returns prior handle");
        // Returned handle is the first one we inserted.
        assert!(prev.outbox.same_channel(&h1.outbox));
        // And `get` now yields the new one.
        let current = reg.get(id).expect("current session present");
        assert!(current.outbox.same_channel(&h2.outbox));
    }

    // ── routing: happy path ───────────────────────────────────────────

    #[tokio::test]
    async fn route_delivers_forwarded_envelope_via_outbox() {
        let reg = SessionRegistry::new();
        let a = DeviceId::new();
        let b = DeviceId::new();
        let (ha, _rxa) = make_handle(a);
        let (hb, mut rxb) = make_handle(b);
        reg.insert(ha);
        reg.insert(hb);

        let payload = serde_json::json!({"jsonrpc": "2.0", "method": "ping", "id": 1});
        reg.route(a, b, payload.clone()).await.unwrap();

        let frame = rxb.recv().await.expect("b must receive the frame");
        match frame {
            Envelope::Forwarded {
                version,
                from,
                payload: p,
            } => {
                assert_eq!(version, 1);
                assert_eq!(from, a);
                assert_eq!(p, payload);
            }
            other => panic!("expected Forwarded, got {other:?}"),
        }
    }

    // ── routing: peer absent ──────────────────────────────────────────

    #[tokio::test]
    async fn route_to_absent_peer_returns_peer_offline() {
        let reg = SessionRegistry::new();
        let a = DeviceId::new();
        let ghost = DeviceId::new();

        let err = reg
            .route(a, ghost, serde_json::json!({}))
            .await
            .unwrap_err();
        match err {
            RelayError::PeerOffline { peer_device_id } => {
                assert_eq!(peer_device_id, ghost.0.to_string());
            }
            other => panic!("expected PeerOffline, got {other:?}"),
        }
    }

    // ── routing: outbox full (drop-newest MVP) ────────────────────────

    #[tokio::test]
    async fn route_to_full_outbox_warns_and_returns_ok() {
        let reg = SessionRegistry::new();
        let a = DeviceId::new();
        let b = DeviceId::new();
        // Capacity 1: one successful send, the next must hit Full.
        let (hb, _rxb) = make_tiny_handle(b, 1);
        reg.insert(hb);

        // First route succeeds; the channel holds one un-received frame.
        reg.route(a, b, serde_json::json!({"n": 1})).await.unwrap();
        // Second route hits Full → warn! + Ok(()).
        reg.route(a, b, serde_json::json!({"n": 2})).await.unwrap();
        // Handle must still be live (we didn't treat Full as "peer gone").
        assert!(reg.get(b).is_some());
    }

    // ── routing: receiver dropped → PeerOffline + cleanup ─────────────

    #[tokio::test]
    async fn route_to_closed_outbox_returns_peer_offline_and_removes_handle() {
        let reg = SessionRegistry::new();
        let a = DeviceId::new();
        let b = DeviceId::new();
        let (hb, rxb) = make_handle(b);
        reg.insert(hb);
        // Simulate the writer task shutting down.
        drop(rxb);

        let err = reg.route(a, b, serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, RelayError::PeerOffline { .. }));
        // Stale entry was cleaned up.
        assert!(reg.get(b).is_none());
    }

    // ── concurrency: insert + remove under contention ─────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_insert_remove_stress_test() {
        const N: usize = 100;
        let reg = SessionRegistry::new();
        let inserts = Arc::new(AtomicUsize::new(0));
        let removes = Arc::new(AtomicUsize::new(0));

        let mut joins = Vec::with_capacity(N);
        for _ in 0..N {
            let reg = reg.clone();
            let inserts = inserts.clone();
            let removes = removes.clone();
            joins.push(tokio::spawn(async move {
                let id = DeviceId::new();
                let (h, _rx) = make_handle(id);
                reg.insert(h);
                inserts.fetch_add(1, Ordering::Relaxed);
                // Yield so the scheduler interleaves us with siblings
                // instead of serialising the 100 tasks.
                tokio::task::yield_now().await;
                reg.remove(id);
                removes.fetch_add(1, Ordering::Relaxed);
            }));
        }
        for j in joins {
            j.await.unwrap();
        }

        assert_eq!(inserts.load(Ordering::Relaxed), N);
        assert_eq!(removes.load(Ordering::Relaxed), N);
        assert!(reg.is_empty(), "all {N} sessions must be removed");
    }

    // ── Arc strong-count on session end (no leaks acceptance) ─────────

    #[tokio::test]
    async fn session_handle_drop_decrements_arc_count() {
        let reg = SessionRegistry::new();
        let id = DeviceId::new();
        let (h, _rx) = make_handle(id);

        // Before insert: we hold the only reference to `paired_with`.
        assert_eq!(Arc::strong_count(&h.paired_with), 1);

        reg.insert(h.clone());
        // After insert: we hold one, the registry's stored clone holds one.
        assert_eq!(Arc::strong_count(&h.paired_with), 2);

        // Drop our view of the inserted clone by never holding it past
        // `insert`; the registry still has its own copy.
        reg.remove(id);
        // After remove: the registry's clone is dropped, leaving only us.
        assert_eq!(Arc::strong_count(&h.paired_with), 1);
    }
}
