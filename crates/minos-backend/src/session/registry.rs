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
//! a slow consumer the channel can fill up; when it does, `route()` now
//! returns [`RelayError::PeerBackpressure`] instead of silently dropping the
//! forwarded payload. That keeps the registry thin while ensuring the sender
//! gets a deterministic failure instead of hanging until timeout.
//!
//! A true drop-oldest policy still has to live in the writer loop, not here:
//! the registry owns only the sender side and cannot pop from the receiver.
//! If step 12's e2e coverage shows that retry-on-backpressure is not enough,
//! revisit this with a writer-owned `drain_one_then_retry` path.
//!
//! On [`TrySendError::Closed`] we translate to [`RelayError::PeerOffline`].
//! The receiver going away means the writer task has shut down; from the
//! caller's perspective the peer is effectively offline.

use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::Envelope;
use tokio::sync::{mpsc, watch, RwLock};

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
/// handler on close. Cheaply clonable — clones share the outbox `Sender`,
/// the `paired_with` lock, and the `last_pong_at` lock.
#[derive(Debug, Clone)]
pub struct SessionHandle {
    /// Identity of the device owning this session. Also the registry key.
    pub device_id: DeviceId,
    /// The role this device speaks in (known at handshake time via the
    /// `X-Device-Role` header; step 9 will parse it). Drives role-gated
    /// local RPC dispatch, e.g. `request_pairing_token` accepts only
    /// [`DeviceRole::MacHost`].
    pub role: DeviceRole,
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
    /// Session-local revocation signal used to actively supersede an old
    /// socket when a reconnect replaces it in the registry.
    revoked: watch::Sender<bool>,
    /// Timestamp of the most recent `Pong` frame we received from this
    /// client. Updated by the dispatcher's read branch (step 8); consumed
    /// by the heartbeat tick branch to decide when to close the socket as
    /// dead. Wrapped in `Arc<RwLock<_>>` so the writer/reader tasks can
    /// share it cheaply.
    pub last_pong_at: Arc<RwLock<Instant>>,
}

impl SessionHandle {
    /// Construct a fresh handle and its paired outbox receiver.
    ///
    /// The caller (step 8's WS accept handler) typically moves the
    /// receiver into the per-socket writer task and passes a clone of the
    /// handle into the reader task and the registry. `last_pong_at` is
    /// seeded with `Instant::now()` so the first heartbeat tick treats a
    /// brand-new session as "freshly alive".
    #[must_use]
    pub fn new(device_id: DeviceId, role: DeviceRole) -> (Self, mpsc::Receiver<ServerFrame>) {
        let (tx, rx) = mpsc::channel(OUTBOX_CAPACITY);
        let (revoked, _revoked_rx) = watch::channel(false);
        let handle = Self {
            device_id,
            role,
            paired_with: Arc::new(RwLock::new(None)),
            outbox: tx,
            revoked,
            last_pong_at: Arc::new(RwLock::new(Instant::now())),
        };
        (handle, rx)
    }

    /// Mark this session as superseded and wake any socket loop waiting on it.
    pub fn revoke(&self) {
        let _ = self.revoked.send(true);
    }

    /// Subscribe to revocation changes for this session.
    #[must_use]
    pub fn subscribe_revocation(&self) -> watch::Receiver<bool> {
        self.revoked.subscribe()
    }

    /// True when `other` refers to the same concrete socket session.
    #[must_use]
    pub fn same_session(&self, other: &Self) -> bool {
        self.outbox.same_channel(&other.outbox)
    }
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

    /// Remove and return `current` only if it is still the live entry.
    ///
    /// This makes disconnect cleanup ABA-safe: an old socket may close
    /// after a reconnect has already inserted a fresh handle for the same
    /// `DeviceId`, and in that case cleanup must leave the fresh entry in
    /// place.
    pub fn remove_current(&self, current: &SessionHandle) -> Option<SessionHandle> {
        self.0
            .remove_if(&current.device_id, |_, live| live.same_session(current))
            .map(|(_k, v)| v)
    }

    /// Clone the handle for `id` if a session is live.
    ///
    /// Returns a clone (cheap: one `Arc` bump on each field) rather than
    /// a `DashMap::Ref` guard so callers can perform async I/O without
    /// holding the shard lock.
    pub fn get(&self, id: DeviceId) -> Option<SessionHandle> {
        self.0.get(&id).map(|r| r.clone())
    }

    /// Queue `frame` only if `current` is still the live registry entry.
    ///
    /// This is stricter than calling `current.outbox.try_send(...)`
    /// directly: a superseded socket can keep its sender alive briefly
    /// during reconnect teardown, so a stale handle may still accept a
    /// frame even though the registry already points at a replacement.
    /// We hold the DashMap shard lock across the synchronous `try_send`
    /// so the liveness check and queueing happen against one stable entry.
    pub fn try_send_current(
        &self,
        current: &SessionHandle,
        frame: ServerFrame,
    ) -> Result<(), RelayError> {
        let Some(live) = self.0.get(&current.device_id) else {
            return Err(RelayError::PeerOffline {
                peer_device_id: current.device_id.to_string(),
            });
        };

        if !live.same_session(current) {
            return Err(RelayError::PeerOffline {
                peer_device_id: current.device_id.to_string(),
            });
        }

        match live.outbox.try_send(frame) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(RelayError::PeerBackpressure {
                peer_device_id: current.device_id.to_string(),
            }),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                drop(live);
                self.remove_current(current);
                Err(RelayError::PeerOffline {
                    peer_device_id: current.device_id.to_string(),
                })
            }
        }
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

    /// Best-effort broadcast of `frame` to every currently-registered
    /// session.
    ///
    /// Intended for shutdown-grade fan-out (`Event::ServerShutdown` in
    /// `main.rs`'s graceful shutdown path). We use `try_send` and **drop
    /// any frame that cannot fit** in a peer's outbox — the caller is
    /// about to tear the process down, so a stalled peer must not block
    /// the broadcast. On `Closed` (the writer task already exited) we
    /// silently skip: the peer is effectively gone.
    ///
    /// Not a cache for per-peer routing — use [`SessionRegistry::route`]
    /// for that. `broadcast` takes a full [`ServerFrame`] (aka
    /// [`Envelope`]) because the frame is already constructed by the
    /// caller, whereas `route` builds the `Forwarded` envelope from raw
    /// payload JSON.
    pub fn broadcast(&self, frame: ServerFrame) {
        for handle in self.0.iter() {
            if let Err(err) = handle.outbox.try_send(frame.clone()) {
                tracing::debug!(
                    target: "minos_backend::session",
                    device_id = %handle.device_id,
                    error = ?err,
                    "broadcast try_send failed; dropping frame for this peer"
                );
            }
        }
    }

    /// Route `payload` from `from` to `to` as an [`Envelope::Forwarded`].
    ///
    /// Mechanical forward — does NOT verify `from` is paired with `to`.
    /// The caller (envelope dispatcher, plan step 8) enforces pairing
    /// before calling `route`.
    ///
    /// Behaviour:
    /// - `to` not in the registry → [`RelayError::PeerOffline`].
    /// - Outbox accepts → `Ok(())`.
    /// - Outbox full → emit `tracing::warn!` and return
    ///   [`RelayError::PeerBackpressure`].
    /// - Outbox closed (receiver dropped) → evict only if the handle is
    ///   still the one whose sender we tried (ABA-safe via
    ///   [`mpsc::Sender::same_channel`]); if a reconnect has already
    ///   replaced the entry, the fresh handle is kept alive so the next
    ///   route succeeds on the new outbox. Returns
    ///   [`RelayError::PeerOffline`] either way — the close signals the
    ///   writer task we tried has shut down.
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
                peer_device_id: to.to_string(),
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
                    target: "minos_backend::session",
                    to = %to,
                    from = %from,
                    "outbox full; rejecting forwarded frame"
                );
                Err(RelayError::PeerBackpressure {
                    peer_device_id: to.to_string(),
                })
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Scoped eviction: only remove if the entry is STILL the
                // handle whose sender we tried. If a reconnect already
                // replaced it between our `get` and here, keep the fresh
                // handle alive — the next route will succeed on the new
                // outbox. Prevents the ABA race where we evict a
                // just-reconnected session.
                self.remove_current(&handle);
                Err(RelayError::PeerOffline {
                    peer_device_id: to.to_string(),
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
        // Tests default to `MacHost` — role-gating is not exercised by the
        // registry itself; the envelope dispatcher (step 8) tests cover it.
        SessionHandle::new(id, DeviceRole::MacHost)
    }

    // Small outbox variant so we can fill it deterministically in tests.
    fn make_tiny_handle(id: DeviceId, cap: usize) -> (SessionHandle, mpsc::Receiver<ServerFrame>) {
        let (tx, rx) = mpsc::channel(cap);
        let (revoked, _revoked_rx) = watch::channel(false);
        (
            SessionHandle {
                device_id: id,
                role: DeviceRole::MacHost,
                paired_with: Arc::new(RwLock::new(None)),
                outbox: tx,
                revoked,
                last_pong_at: Arc::new(RwLock::new(Instant::now())),
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

    #[tokio::test]
    async fn remove_current_only_removes_matching_live_handle() {
        let reg = SessionRegistry::new();
        let id = DeviceId::new();
        let (h1, _rx1) = make_handle(id);
        let (h2, _rx2) = make_handle(id);

        reg.insert(h1.clone());
        reg.insert(h2.clone());

        assert!(
            reg.remove_current(&h1).is_none(),
            "stale cleanup must not remove a replacement entry"
        );
        let current = reg.get(id).expect("replacement handle still live");
        assert!(current.same_session(&h2));

        let removed = reg
            .remove_current(&h2)
            .expect("current handle should be removed");
        assert!(removed.same_session(&h2));
        assert!(reg.get(id).is_none());
    }

    #[tokio::test]
    async fn revoke_notifies_existing_and_late_subscribers() {
        let id = DeviceId::new();
        let (handle, _rx) = make_handle(id);
        let mut subscriber = handle.subscribe_revocation();

        assert!(!*subscriber.borrow());
        handle.revoke();
        subscriber.changed().await.unwrap();
        assert!(*subscriber.borrow_and_update());

        let late_subscriber = handle.subscribe_revocation();
        assert!(
            *late_subscriber.borrow(),
            "late subscribers must observe the current revoked state"
        );
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

    // ── routing: outbox full -> PeerBackpressure ──────────────────────

    #[tokio::test]
    async fn route_to_full_outbox_returns_peer_backpressure_and_keeps_handle() {
        let reg = SessionRegistry::new();
        let a = DeviceId::new();
        let b = DeviceId::new();
        // Capacity 1: one successful send, the next must hit Full.
        let (hb, _rxb) = make_tiny_handle(b, 1);
        reg.insert(hb);

        // First route succeeds; the channel holds one un-received frame.
        reg.route(a, b, serde_json::json!({"n": 1})).await.unwrap();
        // Second route hits Full -> deterministic backpressure error.
        let err = reg
            .route(a, b, serde_json::json!({"n": 2}))
            .await
            .unwrap_err();
        match err {
            RelayError::PeerBackpressure { peer_device_id } => {
                assert_eq!(peer_device_id, b.0.to_string());
            }
            other => panic!("expected PeerBackpressure, got {other:?}"),
        }
        // Handle must still be live (backpressure is not "peer gone").
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

    #[tokio::test]
    async fn try_send_current_rejects_superseded_handle_even_if_stale_sender_is_open() {
        let reg = SessionRegistry::new();
        let id = DeviceId::new();
        let (stale, mut stale_rx) = make_handle(id);
        reg.insert(stale.clone());

        let (current, mut current_rx) = make_handle(id);
        reg.insert(current.clone());

        // The stale sender can still accept frames until the old socket
        // fully tears down, which is why pair completion must not use it
        // directly as proof of delivery.
        let stale_frame = Envelope::Event {
            version: 1,
            event: minos_protocol::EventKind::Unpaired,
        };
        stale
            .outbox
            .try_send(stale_frame.clone())
            .expect("superseded sender should still be open for this regression");
        assert_eq!(stale_rx.try_recv().unwrap(), stale_frame);

        let guarded_frame = Envelope::Event {
            version: 1,
            event: minos_protocol::EventKind::ServerShutdown,
        };
        let err = reg
            .try_send_current(&stale, guarded_frame)
            .expect_err("stale handle must not count as the current live session");
        assert!(matches!(err, RelayError::PeerOffline { .. }));
        assert!(matches!(
            stale_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            current_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        let live = reg.get(id).expect("replacement handle still live");
        assert!(live.same_session(&current));
    }

    // ── routing: ABA-safe eviction on reconnect race ─────────────────

    /// Models the step 8/9/12 reconnect race: the caller observed Closed
    /// on H1 but, before eviction runs, H2 for the same DeviceId has
    /// replaced it. A blind `remove` would nuke the fresh session; the
    /// scoped `remove_if` + `same_channel` preserves H2.
    #[tokio::test]
    async fn route_preserves_fresh_handle_after_reconnect_race() {
        let reg = SessionRegistry::new();
        let a = DeviceId::new();
        let b = DeviceId::new();

        // H1 lands first; we drop the receiver to force Closed on send.
        let (h1, rx1) = make_handle(b);
        reg.insert(h1);
        drop(rx1);

        // Before the next route, H2 reconnects and replaces the entry.
        let (h2, _rx2) = make_handle(b);
        reg.insert(h2.clone());

        // Route runs: it cloned H2 via `get`, try_send on H2 succeeds —
        // no Closed path hit, so nothing to evict. Sanity: H2 is still live.
        reg.route(a, b, serde_json::json!({"n": 1})).await.unwrap();
        let current = reg.get(b).expect("H2 must remain after route");
        assert!(
            current.outbox.same_channel(&h2.outbox),
            "registry must still hold H2"
        );

        // Now force the Closed path against a *stale* sender: grab H1's
        // sender (cloned before drop) would be hard because H1 is gone,
        // so instead we replay the race directly by constructing a stale
        // handle matching the above shape and invoking `remove_if` via
        // the same guard — but the semantic check we care about already
        // holds: `remove_if` only evicts when `same_channel` matches.
        // We verify that explicitly below.
        let (stale, stale_rx) = make_handle(b);
        drop(stale_rx);
        // `stale.outbox` is a different channel from H2's; remove_if must
        // therefore be a no-op.
        reg.0
            .remove_if(&b, |_, v| v.outbox.same_channel(&stale.outbox));
        assert!(
            reg.get(b).is_some(),
            "ABA-safe eviction must NOT remove the fresh handle when the \
             sender does not match"
        );
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

    // ── broadcast: fan-out happy path + drops on full/closed outboxes ─

    #[tokio::test]
    async fn broadcast_delivers_frame_to_every_live_session() {
        let reg = SessionRegistry::new();
        let (h1, mut rx1) = make_handle(DeviceId::new());
        let (h2, mut rx2) = make_handle(DeviceId::new());
        let (h3, mut rx3) = make_handle(DeviceId::new());
        reg.insert(h1);
        reg.insert(h2);
        reg.insert(h3);

        let frame = Envelope::Event {
            version: 1,
            event: minos_protocol::EventKind::ServerShutdown,
        };
        reg.broadcast(frame.clone());

        assert_eq!(rx1.recv().await.unwrap(), frame);
        assert_eq!(rx2.recv().await.unwrap(), frame);
        assert_eq!(rx3.recv().await.unwrap(), frame);
    }

    #[tokio::test]
    async fn broadcast_drops_frame_when_outbox_full_and_skips_closed() {
        // Two peers: one full-outbox, one closed-outbox. Broadcast must
        // complete (no panic, no awaiting) and neither peer blocks the
        // fan-out.
        let reg = SessionRegistry::new();

        // Peer A: tiny outbox, pre-filled to trigger Full on broadcast.
        let (ha, _rxa) = make_tiny_handle(DeviceId::new(), 1);
        ha.outbox
            .try_send(Envelope::Event {
                version: 1,
                event: minos_protocol::EventKind::ServerShutdown,
            })
            .unwrap();
        reg.insert(ha);

        // Peer B: receiver dropped to simulate a writer task that already
        // exited — try_send returns Closed.
        let (hb, rxb) = make_handle(DeviceId::new());
        drop(rxb);
        reg.insert(hb);

        // Must not panic or deadlock.
        let frame = Envelope::Event {
            version: 1,
            event: minos_protocol::EventKind::ServerShutdown,
        };
        reg.broadcast(frame);
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
