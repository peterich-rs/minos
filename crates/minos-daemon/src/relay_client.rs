//! Outbound WebSocket client of the `minos-backend` broker.
//!
//! The Mac daemon runs exactly one `RelayClient` in steady state. It owns
//! a single background task that:
//!
//!   1. stays `Unpaired` without dialing realtime until a host installation
//!      token exists;
//!   2. opens a WSS handshake to `/ws/host` with `Authorization: Bearer hit_*`
//!      (Host Bearer only; no host ws-ticket);
//!   3. receives `ServerFrame` messages from the topic-based realtime gateway;
//!   4. consumes `BotInboxDelivery` / still handles `HostCommandIssued` for
//!      CLI start adapters;
//!   5. routes host ingest ack/pull frames for the daemon sync worker.
//!
//! Pairing QR issuance, redeem polling, and `forget_peer` go through the
//! backend's HTTP `/v1/*` control plane on a separate [`RelayHttpClient`]
//! handle.
//!
//! # Error handling
//!
//! - A missing host installation token is the normal unpaired state.
//! - Explicit token invalid/revoked (HTTP 401 with clear invalid token body,
//!   or WS close `4401` auth_revoked) clears the local `hit_` and returns to
//!   unpaired wait. Transient upgrade failures reconnect without clearing.
//! - WS close code `4400` (malformed frame) records
//!   `MinosError::EnvelopeVersionUnsupported` but reconnects.
//! - All other errors fall back to exponential-backoff reconnect
//!   (1s → 2s → 4s → 8s → 16s → 30s cap, no max attempts).

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use futures_util::{SinkExt, StreamExt};
use http::{header::AUTHORIZATION, HeaderValue, Request as HttpRequest};
use minos_domain::{DeviceId, DeviceSecret, MinosError, PeerState, RelayLinkState};
use minos_protocol::realtime::{
    BotLaunchSnapshot, ClientFrame, ServerFrame, SessionBinding, PRESENCE_STREAM_KIND,
};
use minos_protocol::HostPeerSummary;
use minos_transport::backoff::delay_for_attempt;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};
use tokio_tungstenite::tungstenite::Error as WsError;

use crate::config::RelayConfig;
use crate::ingest_sync::IngestSyncHandle;
use crate::relay_http::RelayHttpClient;
use crate::relay_pairing::PeerRecord;
use crate::rpc_server::{invoke_host_command, RpcServerImpl};

/// Bounded queue for outbound client frames — deep enough to absorb a brief
/// handshake pause without back-pressuring callers. The dispatch loop
/// drains continuously, so the steady-state depth is effectively zero.
const OUTBOUND_QUEUE_DEPTH: usize = 64;
const LIVE_OUTBOUND_QUEUE_DEPTH: usize = 64;
const BACKFILL_OUTBOUND_QUEUE_DEPTH: usize = 16;
const RELAY_PING_INTERVAL: Duration = Duration::from_secs(30);
const RELAY_PONG_TIMEOUT: Duration = Duration::from_secs(45);
const HOST_COMMAND_RESULT_CACHE_CAPACITY: usize = 512;
const TOKEN_WAIT_POLL_INTERVAL: Duration = Duration::from_secs(5);

type HostCommandCache = Arc<Mutex<HashMap<String, HostCommandCacheEntry>>>;

#[derive(Clone)]
enum HostCommandCacheEntry {
    InFlight,
    Completed(HostCommandResultSnapshot),
}

#[derive(Clone)]
struct HostCommandResultSnapshot {
    succeeded: bool,
    result: Option<Value>,
    error: Option<Value>,
    finished_at_ms: i64,
}

enum HostCommandRouteAction {
    Start,
    InFlight,
    Replay(HostCommandResultSnapshot),
}

#[allow(dead_code)]
struct Inner {
    /// Shutdown signal — one-shot, captured behind a `Mutex` so a repeat
    /// `stop()` after the first call is a benign no-op.
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    /// The dispatch task join handle; taken on `stop()`.
    task: Mutex<Option<JoinHandle<()>>>,
    /// The Mac's display name — sent to the backend in `RequestPairingQr`
    /// so the assembled QR carries it through to the iPhone.
    mac_name: String,
    /// Live host installation token. Pairing redeem can mint it while this
    /// relay client is already running, so reconnects read it through a shared
    /// slot instead of a spawn-time snapshot.
    secret: Arc<StdMutex<Option<DeviceSecret>>>,
    /// Wakes the dispatch loop when pairing redeem writes a new host
    /// installation token.
    secret_notify: Arc<Notify>,
    /// Bumped on apply/clear so a live `/ws/host` session closes immediately
    /// when the credential is revoked or replaced (not only on daemon stop).
    credential_epoch: watch::Sender<u64>,
    /// HTTP client for the backend's `/v1/*` control plane.
    http: Arc<RelayHttpClient>,
    /// Peer-state publisher kept so `request_pairing_token` can enter the
    /// `Pairing` axis immediately, before the realtime socket is available.
    peer_tx: watch::Sender<PeerState>,
    peer_store: Arc<StdMutex<Option<PeerRecord>>>,
    peers_store: Arc<StdMutex<Vec<HostPeerSummary>>>,
    last_error: Arc<StdMutex<Option<MinosError>>>,
    /// Cloneable producer side of the dispatcher's control outbound queue.
    out_tx: mpsc::Sender<ClientFrame>,
    live_tx: mpsc::Sender<ClientFrame>,
    backfill_tx: mpsc::Sender<ClientFrame>,
    /// Link-state publisher so account leave can force `/ws/host` offline.
    link_tx: watch::Sender<RelayLinkState>,
}

pub struct RelayClient {
    inner: Arc<Inner>,
}

impl RelayClient {
    /// Spawn the relay-client background task. Returns immediately with a
    /// handle plus two watch receivers the caller can wire into UI.
    ///
    /// The task reconnects forever unless the relay rejects the handshake
    /// with HTTP 401, in which case it exits after broadcasting
    /// `RelayLinkState::Disconnected`. Call [`Self::stop`] to tear the task
    /// down cleanly; the returned `JoinHandle` is awaited internally.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        _config: RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        mac_name: String,
        backend_url: String,
        rpc_server: Option<Arc<RpcServerImpl>>,
        persistence: PersistenceCtx,
    ) -> (
        Arc<Self>,
        watch::Receiver<RelayLinkState>,
        watch::Receiver<PeerState>,
    ) {
        let (link_tx, link_rx) = watch::channel(RelayLinkState::Disconnected);
        let initial_peer = peer
            .as_ref()
            .map_or(PeerState::Unpaired, |p| PeerState::Paired {
                peer_id: p.device_id,
                peer_name: p.name.clone(),
                online: false,
            });
        let (peer_tx, peer_rx) = watch::channel(initial_peer);

        let (out_tx, out_rx) = mpsc::channel::<ClientFrame>(OUTBOUND_QUEUE_DEPTH);
        let (live_tx, live_rx) = mpsc::channel::<ClientFrame>(LIVE_OUTBOUND_QUEUE_DEPTH);
        let (backfill_tx, backfill_rx) =
            mpsc::channel::<ClientFrame>(BACKFILL_OUTBOUND_QUEUE_DEPTH);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let host_command_cache = Arc::new(Mutex::new(HashMap::new()));

        let secret_store = Arc::new(StdMutex::new(secret));
        let secret_notify = Arc::new(Notify::new());
        let (credential_epoch_tx, _credential_epoch_rx) = watch::channel(0_u64);
        let http = match RelayHttpClient::new(&backend_url, self_device_id, mac_name.clone()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::error!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    backend_url = %backend_url,
                    "failed to construct RelayHttpClient; pairing/forget HTTP calls will fail",
                );
                Arc::new(
                    RelayHttpClient::new(
                        "ws://invalid.localhost/devices",
                        self_device_id,
                        mac_name.clone(),
                    )
                    .expect("placeholder RelayHttpClient builds against canonical URL"),
                )
            }
        };
        let peer_store = persistence.peer_store;
        let peers_store = persistence.peers_store;
        let last_error = persistence.last_error;
        let dispatch_ctx = DispatchCtx {
            self_device_id,
            secret: secret_store.clone(),
            secret_notify: secret_notify.clone(),
            credential_epoch: credential_epoch_tx.clone(),
            backend_url: backend_url.clone(),
            link_tx: link_tx.clone(),
            peer_tx: peer_tx.clone(),
            out_tx: out_tx.clone(),
            out_rx,
            live_rx,
            backfill_rx,
            host_command_cache,
            store: persistence.store,
            http: http.clone(),
            rpc_server,
            peer_store: peer_store.clone(),
            peers_store: peers_store.clone(),
            last_error: last_error.clone(),
            ingest_sync: persistence.ingest_sync,
            host_topic_seq: Arc::new(StdMutex::new(0)),
        };

        let task = tokio::spawn(run_dispatch(dispatch_ctx, shutdown_rx));

        let inner = Arc::new(Inner {
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            task: Mutex::new(Some(task)),
            mac_name,
            secret: secret_store,
            secret_notify,
            credential_epoch: credential_epoch_tx,
            http,
            peer_tx,
            peer_store,
            peers_store,
            last_error,
            out_tx,
            live_tx,
            backfill_tx,
            link_tx,
        });

        (Arc::new(Self { inner }), link_rx, peer_rx)
    }

    /// Prepare Host Link proof material: installation id, public key, nonce.
    pub async fn prepare_link(
        &self,
    ) -> Result<minos_protocol::HostPrepareLinkResponse, MinosError> {
        let nonce = self.inner.http.fetch_bootstrap_nonce().await?;
        Ok(minos_protocol::HostPrepareLinkResponse {
            installation_id: self.inner.http.device_id().to_string(),
            public_key: self.inner.http.host_public_key(),
            nonce,
        })
    }

    /// Sign Host Link proof for the local installation.
    pub fn sign_link_proof(
        &self,
        installation_id: &str,
        nonce: &str,
    ) -> Result<minos_protocol::HostSignLinkProofResponse, MinosError> {
        if installation_id != self.inner.http.device_id().to_string() {
            return Err(MinosError::BackendInternal {
                message: "installation_id does not match this host".into(),
            });
        }
        Ok(minos_protocol::HostSignLinkProofResponse {
            signature: self.inner.http.host_link_signature(nonce),
        })
    }

    /// Persist a host installation token and wake the realtime dial loop.
    ///
    /// Bumps the credential epoch so any already-open `/ws/host` session for a
    /// prior token closes before the dialer reconnects with the new secret.
    pub fn apply_link_token(
        &self,
        host_installation_token: &str,
    ) -> Result<minos_protocol::HostApplyLinkTokenResponse, MinosError> {
        if !host_installation_token.starts_with("hit_") {
            return Err(MinosError::BackendInternal {
                message: "host_installation_token must start with hit_".into(),
            });
        }
        let token = DeviceSecret(host_installation_token.to_string());
        crate::device_secret_store::write(&token)?;
        if let Ok(mut guard) = self.inner.secret.lock() {
            *guard = Some(token);
        }
        bump_credential_epoch(&self.inner.credential_epoch);
        self.inner.secret_notify.notify_waiters();
        tracing::info!(
            target: "minos_daemon::relay_client",
            "host installation token applied for Host Link; waking /ws/host dialer"
        );
        Ok(minos_protocol::HostApplyLinkTokenResponse { linked: true })
    }

    /// Drop the local host installation token and force `/ws/host` offline.
    ///
    /// Desktop calls this on account leave so a subsequent login cannot inherit
    /// the previous account's host credential or online bit. Bumps the credential
    /// epoch so a live dispatch loop closes the socket immediately (not only on
    /// daemon shutdown).
    pub fn clear_link_token(
        &self,
    ) -> Result<minos_protocol::HostClearCredentialResponse, MinosError> {
        if let Ok(mut guard) = self.inner.secret.lock() {
            *guard = None;
        }
        crate::device_secret_store::delete()?;
        bump_credential_epoch(&self.inner.credential_epoch);
        let _ = self.inner.link_tx.send(RelayLinkState::Disconnected);
        self.inner.secret_notify.notify_waiters();
        tracing::info!(
            target: "minos_daemon::relay_client",
            "host installation token cleared; /ws/host dialer disabled until re-link"
        );
        Ok(minos_protocol::HostClearCredentialResponse { cleared: true })
    }

    /// Back-compat helper for callers that still think in terms of a
    /// single paired device. Deletes the first currently paired row, if
    /// any, via the host-scoped `/v1/me/peers/{mobile_device_id}` route.
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        let secret = self
            .inner
            .secret
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| MinosError::DeviceNotTrusted {
                device_id: "(none)".into(),
            })?;
        let Some(mobile_device_id) = self
            .inner
            .http
            .get_me_peers(&secret)
            .await?
            .into_iter()
            .next()
            .map(|peer| peer.mobile_device_id)
        else {
            return Ok(());
        };
        self.inner
            .http
            .forget_peer_device(&secret, mobile_device_id)
            .await
    }

    /// Issue `DELETE /v1/me/peers/{mobile_device_id}` for one specific
    /// mobile/account row on the current host.
    pub async fn forget_peer_device(&self, mobile_device_id: DeviceId) -> Result<(), MinosError> {
        let secret = self
            .inner
            .secret
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| MinosError::DeviceNotTrusted {
                device_id: "(none)".into(),
            })?;
        self.inner
            .http
            .forget_peer_device(&secret, mobile_device_id)
            .await
    }

    /// Clone the producer side of the dispatcher's outbound queue.
    ///
    /// Used by in-process producers to push host-side WS frames through the
    /// same socket the dispatcher owns.
    #[must_use]
    pub fn outbound_sender(&self) -> mpsc::Sender<ClientFrame> {
        self.inner.out_tx.clone()
    }

    #[must_use]
    pub fn live_ingest_sender(&self) -> mpsc::Sender<ClientFrame> {
        self.inner.live_tx.clone()
    }

    #[must_use]
    pub fn backfill_sender(&self) -> mpsc::Sender<ClientFrame> {
        self.inner.backfill_tx.clone()
    }

    /// Signal the dispatch task to exit and await its join. Idempotent:
    /// calling twice is a benign no-op after the first success.
    pub async fn stop(&self) {
        if let Some(tx) = self.inner.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.inner.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

/// Shared persistence handles threaded into the dispatcher.
pub struct PersistenceCtx {
    pub peer_store: Arc<StdMutex<Option<PeerRecord>>>,
    pub peers_store: Arc<StdMutex<Vec<HostPeerSummary>>>,
    pub last_error: Arc<StdMutex<Option<MinosError>>>,
    pub ingest_sync: Arc<StdMutex<Option<IngestSyncHandle>>>,
    /// Local SQLite — durable BotInboxDelivery ledger authority.
    pub store: Arc<crate::store::LocalStore>,
}

struct DispatchCtx {
    self_device_id: DeviceId,
    secret: Arc<StdMutex<Option<DeviceSecret>>>,
    secret_notify: Arc<Notify>,
    credential_epoch: watch::Sender<u64>,
    backend_url: String,
    link_tx: watch::Sender<RelayLinkState>,
    peer_tx: watch::Sender<PeerState>,
    out_tx: mpsc::Sender<ClientFrame>,
    out_rx: mpsc::Receiver<ClientFrame>,
    live_rx: mpsc::Receiver<ClientFrame>,
    backfill_rx: mpsc::Receiver<ClientFrame>,
    host_command_cache: HostCommandCache,
    /// Local SQLite ledger for BotInboxDelivery exactly-once inject.
    store: Arc<crate::store::LocalStore>,
    http: Arc<RelayHttpClient>,
    rpc_server: Option<Arc<RpcServerImpl>>,
    peer_store: Arc<StdMutex<Option<PeerRecord>>>,
    peers_store: Arc<StdMutex<Vec<HostPeerSummary>>>,
    last_error: Arc<StdMutex<Option<MinosError>>>,
    ingest_sync: Arc<StdMutex<Option<IngestSyncHandle>>>,
    /// Last applied durable `topic_seq` on `host:{self}` for Subscribe resume.
    host_topic_seq: Arc<StdMutex<i64>>,
}

fn bump_credential_epoch(tx: &watch::Sender<u64>) {
    let next = tx.borrow().wrapping_add(1);
    let _ = tx.send(next);
}

/// True when the live session's credential epoch no longer matches (clear/apply).
fn credential_epoch_changed(rx: &watch::Receiver<u64>, session_epoch: u64) -> bool {
    *rx.borrow() != session_epoch
}

fn clear_peer_snapshot(
    peer_store: &Arc<StdMutex<Option<PeerRecord>>>,
    peers_store: &Arc<StdMutex<Vec<HostPeerSummary>>>,
    peer_tx: &watch::Sender<PeerState>,
) {
    if let Ok(mut guard) = peers_store.lock() {
        guard.clear();
    }
    if let Ok(mut guard) = peer_store.lock() {
        *guard = None;
    }
    let _ = peer_tx.send(PeerState::Unpaired);
}

fn clear_host_installation_token(ctx: &DispatchCtx) {
    if let Ok(mut guard) = ctx.secret.lock() {
        *guard = None;
    }
    if let Err(error) = crate::device_secret_store::delete() {
        tracing::warn!(
            target: "minos_daemon::relay_client",
            error = %error,
            "failed to delete rejected host installation token"
        );
        store_last_error(&ctx.last_error, error);
    }
    clear_peer_snapshot(&ctx.peer_store, &ctx.peers_store, &ctx.peer_tx);
}

enum CycleOutcome {
    Reconnect,
    TokenRejected,
    /// Local clear/apply invalidated the session; re-read secret without backoff.
    CredentialRevoked,
    Shutdown,
}

async fn run_dispatch(mut ctx: DispatchCtx, mut shutdown_rx: oneshot::Receiver<()>) {
    let mut attempt: u32 = 0;
    let mut logged_missing_token = false;
    let mut epoch_rx = ctx.credential_epoch.subscribe();

    // Restart-safe host topic resume watermark (memory is only a hot cache).
    {
        let host_topic = format!("host:{}", ctx.self_device_id);
        match ctx.store.get_host_topic_cursor(&host_topic).await {
            Ok(seq) => {
                if let Ok(mut guard) = ctx.host_topic_seq.lock() {
                    *guard = seq;
                }
                if seq > 0 {
                    tracing::info!(
                        target: "minos_daemon::relay_client",
                        topic = %host_topic,
                        resume_after = seq,
                        "hydrated host topic cursor from SQLite"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    error = %error,
                    "failed to hydrate host topic cursor"
                );
            }
        }
    }

    loop {
        let Some(secret) = secret_snapshot_or_reload(&ctx.secret, &ctx.last_error) else {
            let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
            if !logged_missing_token {
                tracing::info!(
                    target: "minos_daemon::relay_client",
                    backend_url = %ctx.backend_url,
                    "no host installation token available; /ws/host Bearer dial is disabled until pairing redeem completes"
                );
                logged_missing_token = true;
            }
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => {
                    let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
                    return;
                }
                () = ctx.secret_notify.notified() => {}
                // clear/apply may wake us before secret_notify if races reorder.
                changed = epoch_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(TOKEN_WAIT_POLL_INTERVAL) => {}
            }
            attempt = 0;
            continue;
        };
        logged_missing_token = false;
        let session_epoch = *epoch_rx.borrow();
        let _ = ctx.link_tx.send(RelayLinkState::Connecting { attempt });

        let outcome = Box::pin(run_once(&mut ctx, &mut shutdown_rx, &secret, session_epoch)).await;

        match outcome {
            CycleOutcome::Shutdown => {
                let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
                return;
            }
            CycleOutcome::TokenRejected => {
                clear_host_installation_token(&ctx);
                let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
                attempt = 0;
            }
            CycleOutcome::CredentialRevoked => {
                // Secret already cleared or replaced; force offline then re-enter
                // the top of the loop so we only dial with a current credential.
                let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
                attempt = 0;
            }
            CycleOutcome::Reconnect => {
                attempt = attempt.saturating_add(1);
                let delay = delay_for_attempt(attempt);
                tracing::info!(
                    target: "minos_daemon::relay_client",
                    attempt,
                    delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                    "relay link dropped, backing off before reconnect"
                );
                let _ = ctx.link_tx.send(RelayLinkState::Connecting { attempt });

                tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => {
                        let _ = ctx.link_tx.send(RelayLinkState::Disconnected);
                        return;
                    }
                    changed = epoch_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // Credential change aborts backoff; re-read secret immediately.
                        attempt = 0;
                    }
                    () = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

async fn run_once(
    ctx: &mut DispatchCtx,
    shutdown_rx: &mut oneshot::Receiver<()>,
    secret: &DeviceSecret,
    session_epoch: u64,
) -> CycleOutcome {
    let mut epoch_rx = ctx.credential_epoch.subscribe();
    // Dial /ws/host with Authorization: Bearer hit_* (no ticket dance).
    let ws_url = build_host_ws_url(&ctx.backend_url);
    let request = match build_host_ws_request(&ws_url, secret) {
        Ok(req) => req,
        Err(e) => {
            tracing::error!(
                target: "minos_daemon::relay_client",
                error = %e,
                "failed to build host websocket request"
            );
            store_last_error(&ctx.last_error, e);
            return CycleOutcome::Reconnect;
        }
    };

    if credential_epoch_changed(&epoch_rx, session_epoch) {
        return CycleOutcome::CredentialRevoked;
    }

    let ws = tokio::select! {
        biased;
        _ = &mut *shutdown_rx => return CycleOutcome::Shutdown,
        changed = epoch_rx.changed() => {
            if changed.is_err() {
                return CycleOutcome::Shutdown;
            }
            if credential_epoch_changed(&epoch_rx, session_epoch) {
                return CycleOutcome::CredentialRevoked;
            }
            return CycleOutcome::Reconnect;
        }
        res = tokio_tungstenite::connect_async(request) => match res {
            Ok((stream, _resp)) => stream,
            Err(WsError::Http(resp)) if resp.status().as_u16() == 401 => {
                let body = resp
                    .body()
                    .as_ref()
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned);
                let message = body.unwrap_or_else(|| {
                    "relay handshake returned HTTP 401".into()
                });
                // Only clear hit_ when the backend explicitly says the token is
                // invalid/revoked/missing — not on every transient 401 race.
                if is_explicit_token_invalid_message(&message) {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        %message,
                        "host installation token rejected on /ws/host; re-pairing required"
                    );
                    store_last_error(
                        &ctx.last_error,
                        MinosError::Unauthorized { reason: message },
                    );
                    return CycleOutcome::TokenRejected;
                }
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    %message,
                    "relay handshake returned HTTP 401 without explicit token-invalid; reconnecting without clearing hit_"
                );
                store_last_error(
                    &ctx.last_error,
                    MinosError::Unauthorized { reason: message },
                );
                return CycleOutcome::Reconnect;
            }
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    error = %e,
                    "relay handshake failed; will reconnect with backoff"
                );
                return CycleOutcome::Reconnect;
            }
        }
    };

    if credential_epoch_changed(&epoch_rx, session_epoch) {
        // Token was cleared/replaced during handshake; drop the new socket.
        let (mut sink, _stream) = ws.split();
        let _ = sink.send(Message::Close(None)).await;
        return CycleOutcome::CredentialRevoked;
    }

    tracing::info!(target: "minos_daemon::relay_client", "relay websocket upgraded via host bearer");
    refresh_peers_from_backend(ctx, Some(secret)).await;

    Box::pin(dispatch_loop(ws, ctx, shutdown_rx, session_epoch)).await
}

/// Cold snapshot of linked account clients (online / last_active).
/// Call sites: WS upgrade, presence StreamEvent, host-command path that
/// needs fresh peer routing — never on a timer.
async fn refresh_peers_from_backend(ctx: &DispatchCtx, secret: Option<&DeviceSecret>) {
    let Some(secret) = secret else {
        apply_peers_snapshot(ctx, Vec::new());
        return;
    };
    match ctx.http.get_host_peers(secret).await {
        Ok(peers) => apply_peers_snapshot(ctx, peers),
        Err(e) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                error = %e,
                "failed to refresh paired peers",
            );
        }
    }
}

fn apply_peers_snapshot(ctx: &DispatchCtx, peers: Vec<HostPeerSummary>) {
    if let Ok(mut guard) = ctx.peers_store.lock() {
        guard.clone_from(&peers);
    }
    if let Ok(mut guard) = ctx.peer_store.lock() {
        *guard = peers.first().map(peer_record_from_summary);
    }
    let _ = ctx.peer_tx.send(aggregate_peer_state(&peers));
}

fn aggregate_peer_state(peers: &[HostPeerSummary]) -> PeerState {
    let Some(primary) = peers
        .iter()
        .find(|peer| peer.online)
        .or_else(|| peers.first())
    else {
        return PeerState::Unpaired;
    };
    PeerState::Paired {
        peer_id: primary.mobile_device_id,
        peer_name: primary.mobile_device_name.clone(),
        online: primary.online,
    }
}

fn peer_record_from_summary(summary: &HostPeerSummary) -> PeerRecord {
    let paired_at = Utc
        .timestamp_millis_opt(summary.paired_at_ms)
        .single()
        .unwrap_or_else(Utc::now);
    PeerRecord {
        device_id: summary.mobile_device_id,
        name: summary.mobile_device_name.clone(),
        paired_at,
    }
}

/// Inbound + outbound dispatch pump over an upgraded WebSocket.
#[allow(clippy::too_many_lines)]
async fn dispatch_loop(
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ctx: &mut DispatchCtx,
    shutdown_rx: &mut oneshot::Receiver<()>,
    session_epoch: u64,
) -> CycleOutcome {
    let (mut sink, mut stream) = ws.split();
    let mut epoch_rx = ctx.credential_epoch.subscribe();
    let mut heartbeat = tokio::time::interval(RELAY_PING_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pong_at = Instant::now();
    // First tick is immediate; skip so we do not probe right after Hello.
    heartbeat.tick().await;

    // Peer presence is event-driven only:
    // - snapshot once after WS upgrade (caller)
    // - StreamEvent kind=presence → refresh_peers_from_backend
    // No periodic poll (avoids dual SSOT with live presence pushes).
    loop {
        if credential_epoch_changed(&epoch_rx, session_epoch) {
            tracing::info!(
                target: "minos_daemon::relay_client",
                "host credential revoked/replaced; closing /ws/host"
            );
            let _ = sink.send(Message::Close(None)).await;
            return CycleOutcome::CredentialRevoked;
        }
        tokio::select! {
            biased;
            _ = &mut *shutdown_rx => {
                let _ = sink.send(Message::Close(None)).await;
                return CycleOutcome::Shutdown;
            }
            changed = epoch_rx.changed() => {
                if changed.is_err() {
                    let _ = sink.send(Message::Close(None)).await;
                    return CycleOutcome::Shutdown;
                }
                if credential_epoch_changed(&epoch_rx, session_epoch) {
                    tracing::info!(
                        target: "minos_daemon::relay_client",
                        "host credential revoked/replaced; closing /ws/host"
                    );
                    let _ = sink.send(Message::Close(None)).await;
                    return CycleOutcome::CredentialRevoked;
                }
            }
            out = ctx.out_rx.recv() => {
                let Some(frame) = out else {
                    return CycleOutcome::Shutdown;
                };
                if send_outbound_frame(&mut sink, frame).await.is_err() {
                    return CycleOutcome::Reconnect;
                }
            }
            out = ctx.live_rx.recv() => {
                let Some(frame) = out else {
                    continue;
                };
                if send_outbound_frame(&mut sink, frame).await.is_err() {
                    return CycleOutcome::Reconnect;
                }
            }
            out = ctx.backfill_rx.recv() => {
                let Some(frame) = out else {
                    continue;
                };
                if send_outbound_frame(&mut sink, frame).await.is_err() {
                    return CycleOutcome::Reconnect;
                }
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = Box::pin(handle_inbound_text(&text, ctx)).await {
                            tracing::warn!(
                                target: "minos_daemon::relay_client",
                                error = %e,
                                "failed to handle inbound frame"
                            );
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        if let Err(e) = sink.send(Message::Pong(p)).await {
                            tracing::warn!(
                                target: "minos_daemon::relay_client",
                                error = %e,
                                "failed to send relay pong; reconnecting"
                            );
                            return CycleOutcome::Reconnect;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_pong_at = Instant::now();
                    }
                    Some(Ok(Message::Binary(_) | Message::Frame(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        return classify_close(frame, ctx);
                    }
                    Some(Err(e)) => {
                        tracing::warn!(
                            target: "minos_daemon::relay_client",
                            error = %e,
                            "ws read error; reconnecting"
                        );
                        return CycleOutcome::Reconnect;
                    }
                    None => {
                        tracing::info!(
                            target: "minos_daemon::relay_client",
                            "ws stream ended; reconnecting"
                        );
                        return CycleOutcome::Reconnect;
                    }
                }
            }
            _ = heartbeat.tick() => {
                let elapsed = last_pong_at.elapsed();
                if elapsed > RELAY_PONG_TIMEOUT {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                        "relay pong timeout; reconnecting"
                    );
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code: 1011u16.into(),
                            reason: "ping_timeout".into(),
                        })))
                        .await;
                    return CycleOutcome::Reconnect;
                }
                if let Err(e) = sink.send(Message::Ping(Vec::new().into())).await {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        error = %e,
                        "failed to send relay ping; reconnecting"
                    );
                    return CycleOutcome::Reconnect;
                }
            }
        }
    }
}

async fn send_outbound_frame<S>(sink: &mut S, frame: ClientFrame) -> Result<(), ()>
where
    S: futures_util::Sink<Message, Error = WsError> + Unpin,
{
    let text = match serde_json::to_string(&frame) {
        Ok(text) => text,
        Err(error) => {
            tracing::error!(
                target: "minos_daemon::relay_client",
                error = %error,
                "failed to serialize outbound ClientFrame",
            );
            return Ok(());
        }
    };
    if let Err(error) = sink.send(Message::Text(text.into())).await {
        tracing::warn!(
            target: "minos_daemon::relay_client",
            error = %error,
            "failed to send outbound frame; reconnecting",
        );
        return Err(());
    }
    Ok(())
}

/// Parse an inbound text frame as a `ServerFrame` and route it.
async fn handle_inbound_text(text: &str, ctx: &DispatchCtx) -> Result<(), serde_json::Error> {
    let frame: ServerFrame = serde_json::from_str(text)?;
    Box::pin(route_server_frame(frame, ctx)).await;
    Ok(())
}

/// Route a parsed `ServerFrame` to the appropriate handler.
async fn route_server_frame(frame: ServerFrame, ctx: &DispatchCtx) {
    match frame {
        ServerFrame::Hello {
            conn_id,
            heartbeat_interval_ms,
            ..
        } => {
            tracing::info!(
                target: "minos_daemon::relay_client",
                conn_id,
                heartbeat_interval_ms,
                "received Hello from realtime gateway"
            );
            // Hello is register-only; catch-up via Subscribe + resume_after.
            let host_topic = format!("host:{}", ctx.self_device_id);
            let resume_seq = ctx.host_topic_seq.lock().ok().map(|g| *g).unwrap_or(0);
            let mut resume_after = HashMap::new();
            if resume_seq > 0 {
                resume_after.insert(host_topic.clone(), resume_seq);
            }
            let subscribe = ClientFrame::Subscribe {
                topics: vec![host_topic],
                resume_after: if resume_after.is_empty() {
                    None
                } else {
                    Some(resume_after)
                },
                client_request_id: None,
            };
            if ctx.out_tx.send(subscribe).await.is_err() {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    "failed to enqueue host topic subscribe"
                );
            }
        }
        ServerFrame::SubscribeAck { topics, .. } => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                ?topics,
                "subscription acknowledged"
            );
            let host_topic = format!("host:{}", ctx.self_device_id);
            // Default-topic ack after Hello is register-only; Connected after
            // our catch-up SubscribeAck (or default ack if we treat either as armed).
            if topics.iter().any(|topic| topic == &host_topic) {
                let _ = ctx.link_tx.send(RelayLinkState::Connected);
                if let Some(sync) = ingest_sync(ctx) {
                    sync.send_manifest().await;
                }
            }
        }
        ServerFrame::DurableEvent {
            topic,
            topic_seq,
            kind,
            payload,
            ..
        } => {
            let host_topic = format!("host:{}", ctx.self_device_id);
            if topic == host_topic {
                if let Ok(mut guard) = ctx.host_topic_seq.lock() {
                    if topic_seq > *guard {
                        *guard = topic_seq;
                    }
                }
                let now_ms = chrono::Utc::now().timestamp_millis();
                if let Err(error) = ctx
                    .store
                    .set_host_topic_cursor(&host_topic, topic_seq, now_ms)
                    .await
                {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        error = %error,
                        topic_seq,
                        "failed to persist host topic cursor"
                    );
                }
            }
            route_durable_event(&kind, &payload, ctx).await;
        }
        ServerFrame::SnapshotRequired {
            topic,
            last_known_seq,
            retention_floor_seq,
        } => {
            let host_topic = format!("host:{}", ctx.self_device_id);
            if topic == host_topic {
                // Real catch-up: resume from retention floor (not 0). Clearing to 0
                // would re-trigger SnapshotRequired forever when floor > 0.
                let resume_at = retention_floor_seq.max(0);
                if let Ok(mut guard) = ctx.host_topic_seq.lock() {
                    *guard = resume_at;
                }
                let now_ms = chrono::Utc::now().timestamp_millis();
                if let Err(error) = ctx
                    .store
                    .replace_host_topic_cursor(&host_topic, resume_at, now_ms)
                    .await
                {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        error = %error,
                        "failed to persist SnapshotRequired host cursor"
                    );
                }
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    topic,
                    last_known_seq,
                    retention_floor_seq,
                    resume_at,
                    "host topic SnapshotRequired — resume cursor advanced to retention floor"
                );
                // Re-subscribe immediately with the floor cursor so catch-up continues
                // without waiting for the next reconnect cycle.
                let mut resume_after = HashMap::new();
                if resume_at > 0 {
                    resume_after.insert(host_topic.clone(), resume_at);
                }
                let subscribe = ClientFrame::Subscribe {
                    topics: vec![host_topic],
                    resume_after: if resume_after.is_empty() {
                        None
                    } else {
                        Some(resume_after)
                    },
                    client_request_id: None,
                };
                if ctx.out_tx.send(subscribe).await.is_err() {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        "failed to enqueue post-SnapshotRequired host subscribe"
                    );
                }
            }
        }
        ServerFrame::HostIngestAck {
            session_id,
            accepted_to_seq,
            ..
        } => {
            if let Some(sync) = ingest_sync(ctx) {
                sync.mark_backend_acked(&session_id, accepted_to_seq).await;
            }
        }
        ServerFrame::PullIngestRange {
            request_id,
            session_id,
            from_seq,
            to_seq,
            max_bytes,
            priority,
            reason,
        } => {
            if let Some(sync) = ingest_sync(ctx) {
                sync.handle_pull_range(
                    request_id, session_id, from_seq, to_seq, max_bytes, priority, reason,
                )
                .await;
            }
        }
        ServerFrame::PullAck {
            session_id,
            accepted_to_seq,
            ..
        } => {
            if let Some(sync) = ingest_sync(ctx) {
                sync.mark_backend_acked(&session_id, accepted_to_seq).await;
            }
        }
        ServerFrame::HostForceClose { reason, close_code } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                reason,
                close_code,
                "server force-closed the connection"
            );
        }
        ServerFrame::BotInboxDelivery {
            delivery_id,
            conversation_id,
            message,
            bot,
            session,
            lease_expires_at_ms,
        } => {
            // Do not await inject on the dispatch loop — it can fill out_tx and
            // self-deadlock while this branch holds the only out_rx consumer.
            let store = Arc::clone(&ctx.store);
            let out_tx = ctx.out_tx.clone();
            let rpc_server = ctx.rpc_server.clone();
            let self_device_id = ctx.self_device_id;
            tokio::spawn(async move {
                // Reconstruct a minimal ctx-like bundle via a dedicated helper.
                handle_bot_inbox_delivery_spawned(
                    store,
                    out_tx,
                    rpc_server,
                    self_device_id,
                    delivery_id,
                    conversation_id,
                    message,
                    bot,
                    session,
                    lease_expires_at_ms,
                )
                .await;
            });
        }
        ServerFrame::CancelDelivery { delivery_id } => {
            tracing::info!(
                target: "minos_daemon::relay_client",
                delivery_id,
                "received CancelDelivery; marking delivery cancelled if known"
            );
            let now_ms = chrono::Utc::now().timestamp_millis();
            if let Err(error) = ctx
                .store
                .mark_bot_delivery_rejected(&delivery_id, "cancelled by hub", now_ms)
                .await
            {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    delivery_id = %delivery_id,
                    error = %error,
                    "failed to mark cancelled delivery on ledger"
                );
            }
        }
        ServerFrame::ChatSendAck { .. } | ServerFrame::ChatSendNack { .. } => {
            // Account-only frames; ignore on host rail.
        }
        ServerFrame::StreamEvent {
            kind,
            topic,
            payload,
            ..
        } => {
            if kind == PRESENCE_STREAM_KIND {
                tracing::info!(
                    target: "minos_daemon::relay_client",
                    topic,
                    payload = %payload,
                    "presence stream event; refreshing peer snapshot"
                );
                // Account-client online/offline on host:{id}; refresh HTTP list.
                let secret = ctx.secret.lock().ok().and_then(|g| g.clone());
                refresh_peers_from_backend(ctx, secret.as_ref()).await;
            } else {
                tracing::debug!(
                    target: "minos_daemon::relay_client",
                    kind,
                    topic,
                    "ignoring stream event on host side"
                );
            }
        }
        ServerFrame::SubscriptionDenied { topic, reason } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                topic,
                reason,
                "subscription denied"
            );
        }
        ServerFrame::SubscriptionLimitExceeded { limit, current } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                limit,
                current,
                "subscription limit exceeded"
            );
        }
        ServerFrame::Pong { .. } => {}
        ServerFrame::Error {
            code,
            message,
            request_id,
        } => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                code,
                message,
                request_id,
                "server error frame"
            );
        }
    }
}

fn ingest_sync(ctx: &DispatchCtx) -> Option<IngestSyncHandle> {
    ctx.ingest_sync.lock().ok().and_then(|guard| guard.clone())
}

/// Route a durable event extracted from a `ServerFrame::DurableEvent`.
async fn route_durable_event(kind: &str, payload: &Value, ctx: &DispatchCtx) {
    match kind {
        "host_command_issued" => {
            let command_id = payload
                .get("command_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let method = payload
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let params = payload.get("params").cloned().unwrap_or(Value::Null);

            tracing::debug!(
                target: "minos_daemon::relay_client",
                command_id,
                method,
                "received host command"
            );
            let Some(rpc_server) = ctx.rpc_server.clone() else {
                tracing::warn!(
                    target: "minos_daemon::relay_client",
                    command_id,
                    "no rpc_server wired — dropping host command"
                );
                return;
            };

            let ack = build_host_command_ack(&command_id, chrono::Utc::now().timestamp_millis());
            let _ = ctx.out_tx.send(ack).await;

            match remember_host_command_start(&ctx.host_command_cache, ctx.store.as_ref(), &command_id).await {
                HostCommandRouteAction::Start => {}
                HostCommandRouteAction::InFlight => return,
                HostCommandRouteAction::Replay(snapshot) => {
                    let response = build_host_command_result(
                        &command_id,
                        snapshot.succeeded,
                        snapshot.result,
                        snapshot.error,
                        snapshot.finished_at_ms,
                    );
                    let _ = ctx.out_tx.send(response).await;
                    return;
                }
            }

            let out_tx = ctx.out_tx.clone();
            let host_command_cache = Arc::clone(&ctx.host_command_cache);
            let store = Arc::clone(&ctx.store);
            tokio::spawn(async move {
                let result = invoke_host_command(&method, params, &rpc_server).await;
                let finished_at_ms = chrono::Utc::now().timestamp_millis();
                let (succeeded, result, error) = match result {
                    Ok(result) => (true, Some(result), None),
                    Err(error) => (false, None, Some(error)),
                };
                remember_host_command_result(
                    &host_command_cache,
                    store.as_ref(),
                    &command_id,
                    HostCommandResultSnapshot {
                        succeeded,
                        result: result.clone(),
                        error: error.clone(),
                        finished_at_ms,
                    },
                )
                .await;
                let response = build_host_command_result(
                    &command_id,
                    succeeded,
                    result,
                    error,
                    finished_at_ms,
                );
                if let Err(e) = out_tx.send(response).await {
                    tracing::warn!(
                        target: "minos_daemon::relay_client",
                        error = %e,
                        command_id,
                        "failed to enqueue host command result"
                    );
                }
            });
        }
        "host_linked" | "host_unlinked" => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                kind,
                "peer state changed, refreshing"
            );
            let secret = ctx.secret.lock().ok().and_then(|guard| guard.clone());
            refresh_peers_from_backend(ctx, secret.as_ref()).await;
        }
        "host_force_close" => {
            let reason = payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            tracing::warn!(
                target: "minos_daemon::relay_client",
                reason,
                "host force close durable event"
            );
        }
        other => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                kind = other,
                "ignoring durable event on host side"
            );
        }
    }
}

async fn remember_host_command_start(
    cache: &HostCommandCache,
    store: &crate::store::LocalStore,
    command_id: &str,
) -> HostCommandRouteAction {
    if command_id.is_empty() {
        return HostCommandRouteAction::Start;
    }

    // Durable ledger is restart authority; memory cache is a process-local fast path.
    let now_ms = chrono::Utc::now().timestamp_millis();
    match store.begin_host_command(command_id, now_ms).await {
        Ok(crate::store::host_command_ledger::HostCommandBegin::InFlight) => {
            let mut entries = cache.lock().await;
            entries.insert(command_id.to_string(), HostCommandCacheEntry::InFlight);
            return HostCommandRouteAction::InFlight;
        }
        Ok(crate::store::host_command_ledger::HostCommandBegin::Replay {
            succeeded,
            result_json,
            error_json,
            finished_at_ms,
        }) => {
            let snapshot = HostCommandResultSnapshot {
                succeeded,
                result: result_json.and_then(|s| serde_json::from_str(&s).ok()),
                error: error_json.and_then(|s| serde_json::from_str(&s).ok()),
                finished_at_ms,
            };
            let mut entries = cache.lock().await;
            entries.insert(
                command_id.to_string(),
                HostCommandCacheEntry::Completed(snapshot.clone()),
            );
            prune_host_command_cache(&mut entries);
            return HostCommandRouteAction::Replay(snapshot);
        }
        Ok(crate::store::host_command_ledger::HostCommandBegin::Start) => {}
        Err(error) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                error = %error,
                command_id,
                "host_command_ledger begin failed; falling back to memory cache"
            );
        }
    }

    let mut entries = cache.lock().await;
    match entries.get(command_id).cloned() {
        Some(HostCommandCacheEntry::InFlight) => HostCommandRouteAction::InFlight,
        Some(HostCommandCacheEntry::Completed(snapshot)) => {
            HostCommandRouteAction::Replay(snapshot)
        }
        None => {
            entries.insert(command_id.to_string(), HostCommandCacheEntry::InFlight);
            prune_host_command_cache(&mut entries);
            HostCommandRouteAction::Start
        }
    }
}

async fn remember_host_command_result(
    cache: &HostCommandCache,
    store: &crate::store::LocalStore,
    command_id: &str,
    snapshot: HostCommandResultSnapshot,
) {
    if command_id.is_empty() {
        return;
    }

    let result_json = snapshot
        .result
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok());
    let error_json = snapshot
        .error
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok());
    if let Err(error) = store
        .complete_host_command(
            command_id,
            snapshot.succeeded,
            result_json.as_deref(),
            error_json.as_deref(),
            snapshot.finished_at_ms,
        )
        .await
    {
        tracing::warn!(
            target: "minos_daemon::relay_client",
            error = %error,
            command_id,
            "failed to persist host_command_ledger completion"
        );
    }

    let mut entries = cache.lock().await;
    entries.insert(
        command_id.to_string(),
        HostCommandCacheEntry::Completed(snapshot),
    );
    prune_host_command_cache(&mut entries);
}

fn prune_host_command_cache(entries: &mut HashMap<String, HostCommandCacheEntry>) {
    while entries.len() > HOST_COMMAND_RESULT_CACHE_CAPACITY {
        let Some(key) = entries.iter().find_map(|(command_id, entry)| {
            matches!(entry, HostCommandCacheEntry::Completed(_)).then(|| command_id.clone())
        }) else {
            break;
        };
        entries.remove(&key);
    }
}

fn classify_close(frame: Option<CloseFrame>, ctx: &DispatchCtx) -> CycleOutcome {
    let code: Option<u16> = frame.as_ref().map(|f| f.code.into());
    let reason: Option<String> = frame
        .as_ref()
        .map(|f| f.reason.to_string())
        .filter(|s| !s.is_empty());
    match code {
        Some(4401) => {
            // Explicit auth revocation from gateway (token invalid / role changed).
            tracing::warn!(
                target: "minos_daemon::relay_client",
                code = 4401,
                ?reason,
                "relay closed socket with 4401 auth_revoked; clearing host installation token"
            );
            store_last_error(
                &ctx.last_error,
                MinosError::DeviceNotTrusted {
                    device_id: ctx.self_device_id.to_string(),
                },
            );
            CycleOutcome::TokenRejected
        }
        Some(4400) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                code = 4400,
                ?reason,
                "relay closed socket with 4400 — frame rejected; will reconnect"
            );
            store_last_error(
                &ctx.last_error,
                MinosError::EnvelopeVersionUnsupported { version: 1 },
            );
            CycleOutcome::Reconnect
        }
        other => {
            tracing::info!(
                target: "minos_daemon::relay_client",
                code = ?other,
                ?reason,
                "relay sent Close; reconnecting"
            );
            CycleOutcome::Reconnect
        }
    }
}

fn store_last_error(slot: &Arc<StdMutex<Option<MinosError>>>, err: MinosError) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(err);
    }
}

fn secret_snapshot(slot: &Arc<StdMutex<Option<DeviceSecret>>>) -> Option<DeviceSecret> {
    slot.lock().ok().and_then(|guard| guard.clone())
}

fn secret_snapshot_or_reload(
    slot: &Arc<StdMutex<Option<DeviceSecret>>>,
    last_error: &Arc<StdMutex<Option<MinosError>>>,
) -> Option<DeviceSecret> {
    if let Some(secret) = secret_snapshot(slot) {
        return Some(secret);
    }

    match crate::device_secret_store::read() {
        Ok(Some(secret)) => {
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(secret.clone());
            }
            tracing::info!(
                target: "minos_daemon::relay_client",
                "reloaded persisted host installation token before realtime connect"
            );
            Some(secret)
        }
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                target: "minos_daemon::relay_client",
                error = %error,
                "failed to reload persisted host installation token before realtime connect"
            );
            store_last_error(last_error, error);
            None
        }
    }
}

// ─── ClientFrame builders ──────────────────────────────────────────────

fn build_host_command_ack(command_id: &str, ack_at_ms: i64) -> ClientFrame {
    ClientFrame::HostCommandAck {
        command_id: command_id.to_string(),
        ack_at_ms,
    }
}

fn build_host_command_result(
    command_id: &str,
    succeeded: bool,
    result: Option<Value>,
    error: Option<Value>,
    finished_at_ms: i64,
) -> ClientFrame {
    ClientFrame::HostCommandResult {
        command_id: command_id.to_string(),
        status: if succeeded {
            "succeeded".into()
        } else {
            "failed".into()
        },
        result,
        error,
        finished_at_ms,
    }
}

// ─── Bot mailbox consumer ──────────────────────────────────────────────

async fn handle_bot_inbox_delivery_spawned(
    store: Arc<crate::store::LocalStore>,
    out_tx: mpsc::Sender<ClientFrame>,
    rpc_server: Option<Arc<RpcServerImpl>>,
    _self_device_id: DeviceId,
    delivery_id: String,
    conversation_id: String,
    message: minos_protocol::ChatMessageSummary,
    bot: BotLaunchSnapshot,
    session: SessionBinding,
    lease_expires_at_ms: i64,
) {
    use crate::store::bot_delivery_ledger::BotDeliveryBegin;

    tracing::info!(
        target: "minos_daemon::relay_client",
        delivery_id = %delivery_id,
        conversation_id = %conversation_id,
        bot_id = %bot.bot_id,
        lease_expires_at_ms,
        "received BotInboxDelivery"
    );

    let now_ms = chrono::Utc::now().timestamp_millis();
    let session_hint = session.session_id.as_deref().filter(|s| !s.is_empty());
    let begin = match store
        .begin_bot_delivery(
            &delivery_id,
            &conversation_id,
            &bot.bot_id,
            session_hint,
            now_ms,
        )
        .await
    {
        Ok(action) => action,
        Err(error) => {
            tracing::error!(
                target: "minos_daemon::relay_client",
                delivery_id = %delivery_id,
                error = %error,
                "bot delivery ledger begin failed"
            );
            let _ = out_tx
                .send(ClientFrame::DeliveryRejected {
                    delivery_id,
                    code: "host_ledger_error".into(),
                    detail: error.to_string().chars().take(500).collect(),
                })
                .await;
            return;
        }
    };
    match begin {
        BotDeliveryBegin::Start => {}
        BotDeliveryBegin::InFlight => {
            tracing::debug!(
                target: "minos_daemon::relay_client",
                delivery_id = %delivery_id,
                "BotInboxDelivery already in-flight or terminal; ignoring duplicate"
            );
            return;
        }
        BotDeliveryBegin::ReplayAccepted => {
            let _ = out_tx
                .send(ClientFrame::DeliveryAccepted {
                    delivery_id: delivery_id.clone(),
                })
                .await;
            return;
        }
        BotDeliveryBegin::ReplayRejected => {
            let _ = out_tx
                .send(ClientFrame::DeliveryRejected {
                    delivery_id: delivery_id.clone(),
                    code: "already_rejected".into(),
                    detail: "delivery previously rejected".into(),
                })
                .await;
            return;
        }
    }

    let Some(rpc_server) = rpc_server.clone() else {
        tracing::warn!(
            target: "minos_daemon::relay_client",
            delivery_id = %delivery_id,
            "no rpc_server wired — rejecting BotInboxDelivery"
        );
        let _ = store
            .mark_bot_delivery_rejected(&delivery_id, "rpc_server not wired", now_ms)
            .await;
        let _ = out_tx
            .send(ClientFrame::DeliveryRejected {
                delivery_id,
                code: "host_not_ready".into(),
                detail: "rpc_server not wired".into(),
            })
            .await;
        return;
    };

    // Inject first; only then DeliveryAccepted. Accept-before-inject races with
    // Hub state and poisons the durable ledger on transient failure.
    let origin_message_id = message.message_id.clone();
    let text = message.text.clone();
    let inject = inject_mailbox_delivery(
        &rpc_server,
        &delivery_id,
        &conversation_id,
        &bot,
        &session,
        &text,
        &origin_message_id,
    )
    .await;
    match inject {
        Ok(()) => {
            if let Err(error) = store
                .mark_bot_delivery_injected(&delivery_id, session_hint, now_ms)
                .await
            {
                tracing::error!(
                    target: "minos_daemon::relay_client",
                    delivery_id = %delivery_id,
                    error = %error,
                    "bot delivery ledger inject mark failed after successful inject"
                );
            }
            tracing::info!(
                target: "minos_daemon::relay_client",
                delivery_id = %delivery_id,
                "BotInboxDelivery injected into local session"
            );
            let _ = out_tx
                .send(ClientFrame::DeliveryAccepted {
                    delivery_id: delivery_id.clone(),
                })
                .await;
        }
        Err((code, detail)) => {
            // Retryable inject failure: clear non-terminal received so Hub requeue
            // can redeliver. Do not mark rejected (that would poison redelivery).
            let _ = store.clear_bot_delivery_inflight(&delivery_id).await;
            tracing::warn!(
                target: "minos_daemon::relay_client",
                delivery_id = %delivery_id,
                code = %code,
                detail = %detail,
                "BotInboxDelivery inject failed"
            );
            let _ = out_tx
                .send(ClientFrame::DeliveryRejected {
                    delivery_id,
                    code,
                    detail,
                })
                .await;
        }
    }
}

async fn inject_mailbox_delivery(
    rpc_server: &Arc<RpcServerImpl>,
    delivery_id: &str,
    conversation_id: &str,
    bot: &BotLaunchSnapshot,
    session: &SessionBinding,
    text: &str,
    origin_message_id: &str,
) -> Result<(), (String, String)> {
    // P0: Hub is SSOT for session_id — must honor SessionBinding.session_id.
    // Never invent a second mailbox-* formula on the host.
    let session_id = session
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            (
                "missing_session".into(),
                "Hub SessionBinding.session_id is required".into(),
            )
        })?;

    if session.create_if_missing {
        // Cold start with Hub-assigned canonical session id.
        let workspace = bot
            .workspace_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        let system_prompt = bot.system_prompt.trim();
        let mut params = serde_json::json!({
            "session_id": session_id,
            "agent_id": bot.bot_id,
            "bot_id": bot.bot_id,
            "runtime_agent": bot.runtime_agent,
            "workspace": workspace,
            "initial_user_message": text,
            "origin_message_id": origin_message_id,
            "conversation_id": conversation_id,
            "model": bot.model,
            "reasoning_effort": bot.default_reasoning_effort,
            "delivery_id": delivery_id,
        });
        if !system_prompt.is_empty() {
            params["system_prompt"] = serde_json::json!(system_prompt);
            params["instructions"] = serde_json::json!(system_prompt);
        }
        invoke_host_command("agent_session.start", params, rpc_server)
            .await
            .map(|_| ())
            .map_err(|err| {
                (
                    "start_failed".into(),
                    err.to_string().chars().take(500).collect(),
                )
            })
    } else {
        let params = serde_json::json!({
            "session_id": session_id,
            "text": text,
            "origin_message_id": origin_message_id,
            "delivery_id": delivery_id,
            "bot_id": bot.bot_id,
        });
        invoke_host_command("agent_session.send_input", params, rpc_server)
            .await
            .map(|_| ())
            .map_err(|err| {
                (
                    "send_input_failed".into(),
                    err.to_string().chars().take(500).collect(),
                )
            })
    }
}

// ─── URL helpers ───────────────────────────────────────────────────────

fn build_host_ws_url(base_url: &str) -> String {
    let ws_url = websocket_base(base_url);
    format!("{ws_url}/ws/host")
}

fn build_host_ws_request(
    ws_url: &str,
    secret: &DeviceSecret,
) -> Result<HttpRequest<()>, MinosError> {
    let mut request =
        ws_url
            .into_client_request()
            .map_err(|error| MinosError::BackendInternal {
                message: format!("invalid host websocket url: {error}"),
            })?;
    let auth = format!("Bearer {}", secret.as_str());
    let value = HeaderValue::from_str(&auth).map_err(|error| MinosError::BackendInternal {
        message: format!("invalid host bearer header: {error}"),
    })?;
    request.headers_mut().insert(AUTHORIZATION, value);
    Ok(request)
}

fn is_explicit_token_invalid_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("invalid host installation token")
        || lower.contains("missing host installation token")
        || lower.contains("token revoked")
        || lower.contains("revoked")
        || lower == "invalid"
}

fn websocket_base(base_url: &str) -> String {
    let Ok(url) = url::Url::parse(base_url) else {
        return base_url.trim_end_matches('/').to_string();
    };
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => other,
    };
    let Some(host) = url.host_str() else {
        return base_url.trim_end_matches('/').to_string();
    };
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!("{scheme}://{host}{port}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn build_host_ws_url_ws() {
        assert_eq!(
            build_host_ws_url("ws://127.0.0.1:8787"),
            "ws://127.0.0.1:8787/ws/host"
        );
    }

    #[test]
    fn build_host_ws_url_wss() {
        assert_eq!(
            build_host_ws_url("wss://example.com"),
            "wss://example.com/ws/host"
        );
    }

    #[test]
    fn build_host_ws_url_strips_legacy_devices_path() {
        assert_eq!(
            build_host_ws_url("ws://127.0.0.1:8787/devices"),
            "ws://127.0.0.1:8787/ws/host"
        );
    }

    #[test]
    fn explicit_token_invalid_detects_host_auth_messages() {
        assert!(is_explicit_token_invalid_message(
            "invalid host installation token"
        ));
        assert!(is_explicit_token_invalid_message(
            "missing host installation token"
        ));
        assert!(!is_explicit_token_invalid_message(
            "relay handshake returned HTTP 401"
        ));
    }

    #[test]
    fn build_host_ws_request_sets_authorization_bearer() {
        let secret = DeviceSecret("hit_testtoken123".into());
        let req = build_host_ws_request("ws://127.0.0.1:8787/ws/host", &secret).unwrap();
        let auth = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .expect("Authorization header");
        assert_eq!(auth, "Bearer hit_testtoken123");
    }
}
