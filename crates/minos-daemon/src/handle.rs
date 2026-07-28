//! Public façade exposed to Swift via UniFFI, rewired for the relay-client
//! migration (plan 05 Phase F).
//!
//! `DaemonInner` owns the outbound [`RelayClient`] plus its two watch
//! receivers (relay link + peer) and the current in-memory trusted peer.
//! Sync FFI methods dispatch onto `rt_handle` so Swift's non-runtime
//! sessions can still enter the Tokio reactor — same trick the old
//! WS-server façade used.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use chrono::{TimeZone, Utc};
use minos_domain::{DeviceId, DeviceSecret, MinosError};
use tokio::runtime::Handle;
use tokio::sync::watch;

use minos_protocol::HostPeerSummary;

use crate::agent::AgentGlue;
use crate::config::RelayConfig;
use crate::ingest_sync::IngestSyncHandle;
use crate::local_rpc::{start_local_rpc_server, LocalRpcConfig};
use crate::paths;
use crate::relay_client::{PersistenceCtx, RelayClient};
use crate::relay_pairing::{PeerRecord, RelayQrPayload};

struct DaemonInner {
    relay: Arc<RelayClient>,
    link_rx: watch::Receiver<minos_domain::RelayLinkState>,
    peer_rx: watch::Receiver<minos_domain::PeerState>,
    /// In-memory mirror of the trusted peer. Shared `Arc` with the
    /// relay-client dispatch task, which updates it on every
    /// `EventKind::Paired` / `Unpaired` so warm reads via
    /// `current_trusted_device` always see the newest record.
    peer: Arc<StdMutex<Option<PeerRecord>>>,
    /// Full host-side mobile/account snapshot from `GET /v1/me/peers`.
    peers: Arc<StdMutex<Vec<HostPeerSummary>>>,
    /// Kept on the inner — future trace logging and eventual UniFFI
    /// getters need the display name that was minted into the relay
    /// handshake.
    #[allow(dead_code)]
    mac_name: String,
    /// Populated by the relay-client task on fatal exit paths (pre-upgrade
    /// HTTP 401 → `Unauthorized`; post-upgrade WS close 4401 →
    /// `DeviceNotTrusted`; close 4400 → `EnvelopeVersionUnsupported`).
    /// Drained on read so the UI sees each failure at most once per
    /// occurrence.
    last_error: Arc<StdMutex<Option<MinosError>>>,
    agent: Arc<AgentGlue>,
    /// Captured under `DaemonHandle::start` (which always runs inside a
    /// Tokio runtime — either the CLI's `#[tokio::main]` or UniFFI's
    /// tokio runtime) so sync FFI methods can spawn onto it from Swift
    /// sessions that lack a current runtime.
    rt_handle: Handle,
    /// Handle for the optional local RPC server (TUI daemon). `None` when
    /// the daemon runs without a local control plane.
    #[allow(dead_code)]
    local_rpc_handle: Option<jsonrpsee::server::ServerHandle>,
    local_rpc_discovery_path: Option<PathBuf>,
    /// Bound local RPC WebSocket URL when `local_rpc_config` was provided.
    local_rpc_url: Option<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct DaemonHandle {
    inner: Arc<DaemonInner>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl DaemonHandle {
    /// Production entry point. Spawns a single `RelayClient` that dials
    /// the resolved relay backend URL and publishes two
    /// independent watch channels: relay-link and peer-pairing.
    ///
    /// `peer` and `secret` are optional warm-start inputs. The macOS app
    /// now passes `None` for both and starts from a fresh in-memory pairing
    /// state on every launch.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    #[allow(clippy::missing_errors_doc, clippy::unused_async)]
    pub async fn start(
        config: RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        mac_name: String,
    ) -> Result<Arc<Self>, MinosError> {
        Self::start_with_local_rpc(config, self_device_id, peer, secret, mac_name, None).await
    }
}

impl DaemonHandle {
    /// Extended entry point that optionally starts a local JSON-RPC server
    /// for TUI-daemon communication. When `local_rpc_config` is `None`, the
    /// behaviour is identical to [`start`].
    #[allow(clippy::missing_errors_doc, clippy::unused_async)]
    pub async fn start_with_local_rpc(
        config: RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        mac_name: String,
        local_rpc_config: Option<LocalRpcConfig>,
    ) -> Result<Arc<Self>, MinosError> {
        let secret = match secret {
            Some(secret) => Some(secret),
            None => crate::device_secret_store::read()?,
        };

        // Capture the user's login-shell env once. Failures fall back to
        // process env internally, so this never blocks bootstrap.
        let subprocess_env = Arc::new(minos_cli_detect::capture_user_shell_env().await);

        // Open the daemon's local SQLite store. The schema is migrated on
        // first open via sqlx::migrate! against `crates/minos-daemon/migrations`.
        let db_path = paths::minos_home()?.join("daemon.sqlite");
        let store = Arc::new(
            crate::store::LocalStore::open(&db_path)
                .await
                .map_err(|e| MinosError::StoreIo {
                    path: db_path.display().to_string(),
                    message: format!("LocalStore::open failed: {e}"),
                })?,
        );

        // C21: any thread that was running / idle when the previous daemon
        // exited gets flipped to `suspended { daemon_restart }` so the mobile
        // UI can re-render the right state on reconnect.
        match store.mark_orphans_suspended().await {
            Ok(n) if n > 0 => tracing::info!(
                target: "minos_daemon::handle",
                rows = n,
                "startup recovery flipped {n} orphan sessions to suspended",
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(
                target: "minos_daemon::handle",
                error = %e,
                "mark_orphans_suspended failed; non-fatal, continuing startup",
            ),
        }

        // Prune `.minos-worktrees/*` directories not referenced by any conversation
        // (crash / kill without `git_remove_worktree(delete_files)` leaves orphans).
        match (
            store.list_registered_worktree_paths().await,
            store.list_projects().await,
        ) {
            (Ok(paths), Ok(projects)) => {
                let registered: Vec<std::path::PathBuf> =
                    paths.into_iter().map(std::path::PathBuf::from).collect();
                let workspaces: Vec<std::path::PathBuf> = projects
                    .into_iter()
                    .filter_map(|p| p.workspace_path.map(std::path::PathBuf::from))
                    .collect();
                let report = crate::git::prune_orphan_worktrees(&registered, &workspaces);
                if report.pruned > 0 || !report.errors.is_empty() {
                    tracing::info!(
                        target: "minos_daemon::handle",
                        scanned_roots = report.scanned_roots,
                        pruned = report.pruned,
                        errors = report.errors.len(),
                        "startup worktree orphan reconciliation finished",
                    );
                }
                for err in report.errors {
                    tracing::warn!(
                        target: "minos_daemon::handle",
                        error = %err,
                        "worktree orphan prune error",
                    );
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!(
                    target: "minos_daemon::handle",
                    error = %e,
                    "worktree orphan reconciliation skipped; non-fatal",
                );
            }
        }

        // Build the agent glue ahead of the relay. Live ingest upload is
        // attached after the relay exists; local SQLite persistence works
        // immediately and no longer depends on a relay outbound queue.
        let agent = Arc::new(AgentGlue::new(
            paths::minos_home()?.join("workspaces"),
            subprocess_env.clone(),
            store.clone(),
        ));

        // The relay-client dispatches forwarded peer JSON-RPC into this
        // server impl. Pre-relay it lived behind a jsonrpsee WS server;
        // now there is exactly one shared instance threaded through.
        let rpc_server = Arc::new(crate::rpc_server::RpcServerImpl {
            started_at: std::time::Instant::now(),
            runner: Arc::new(minos_cli_detect::RealCommandRunner::new(
                subprocess_env.clone(),
            )),
            agent: agent.clone(),
        });

        // Shared between `DaemonInner` and the relay dispatch task — the
        // latter writes on every Paired/Unpaired event so warm reads here
        // always see the freshest record without round-tripping the
        // watch channel.
        let peer_store: Arc<StdMutex<Option<PeerRecord>>> = Arc::new(StdMutex::new(peer.clone()));
        let peers_store: Arc<StdMutex<Vec<HostPeerSummary>>> = Arc::new(StdMutex::new(Vec::new()));
        let last_error: Arc<StdMutex<Option<MinosError>>> = Arc::new(StdMutex::new(None));

        let backend_url = config.resolved_backend_url().to_owned();
        let backend_url_source = if config.backend_url.trim().is_empty() {
            "baked-rust-default"
        } else {
            "runtime-config"
        };
        tracing::info!(
            target: "minos_daemon::handle",
            self_device_id = %self_device_id,
            backend_url = %backend_url,
            backend_url_source,
            "daemon runtime config resolved"
        );

        let ingest_sync_slot: Arc<StdMutex<Option<IngestSyncHandle>>> =
            Arc::new(StdMutex::new(None));

        let (relay, link_rx, peer_rx) = RelayClient::spawn(
            config,
            self_device_id,
            peer.clone(),
            secret,
            mac_name.clone(),
            backend_url,
            Some(rpc_server),
            PersistenceCtx {
                peer_store: peer_store.clone(),
                peers_store: peers_store.clone(),
                last_error: last_error.clone(),
                ingest_sync: ingest_sync_slot.clone(),
            },
        );

        let ingest_sync = IngestSyncHandle::spawn(
            self_device_id,
            store.clone(),
            relay.outbound_sender(),
            relay.live_ingest_sender(),
            relay.backfill_sender(),
            link_rx.clone(),
        );
        if let Ok(mut guard) = ingest_sync_slot.lock() {
            *guard = Some(ingest_sync.clone());
        }
        agent.set_ingest_sync(ingest_sync);

        let local_rpc_discovery_path = local_rpc_config
            .as_ref()
            .map(|config| config.discovery_path.clone());

        let (local_rpc_handle, local_rpc_url) = if let Some(lr_config) = local_rpc_config {
            let runner = Arc::new(minos_cli_detect::RealCommandRunner::new(
                subprocess_env.clone(),
            ));
            let started = start_local_rpc_server(lr_config, runner, agent.clone()).await?;
            (Some(started.handle), Some(started.url))
        } else {
            (None, None)
        };

        Ok(Arc::new(Self {
            inner: Arc::new(DaemonInner {
                relay,
                link_rx,
                peer_rx,
                peer: peer_store,
                peers: peers_store,
                mac_name,
                last_error,
                agent,
                rt_handle: Handle::current(),
                local_rpc_handle,
                local_rpc_discovery_path,
                local_rpc_url,
            }),
        }))
    }

    /// WebSocket URL of the in-process local JSON-RPC server, if started.
    #[must_use]
    pub fn local_rpc_url(&self) -> Option<String> {
        self.inner.local_rpc_url.clone()
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl DaemonHandle {
    /// Snapshot the current relay-link state. Cheap — just a `watch`
    /// borrow.
    #[must_use]
    pub fn current_relay_link(&self) -> minos_domain::RelayLinkState {
        *self.inner.link_rx.borrow()
    }

    /// Snapshot the current peer-pairing state. Cloned because
    /// `PeerState::Paired` carries a String.
    #[must_use]
    pub fn current_peer(&self) -> minos_domain::PeerState {
        self.inner.peer_rx.borrow().clone()
    }

    /// Return the currently trusted peer record (from our in-memory
    /// mirror). Returns `Ok(None)` if we have no paired peer yet.
    #[allow(clippy::missing_errors_doc, clippy::unused_async)]
    pub async fn current_trusted_device(&self) -> Result<Option<PeerRecord>, MinosError> {
        // `async fn` kept for UniFFI parity with the other getters — the
        // underlying lock is sync and never held across an await point.
        Ok(self.inner.peer.lock().unwrap().clone())
    }

    /// Return the full host-side mobile/account snapshot.
    #[allow(clippy::missing_errors_doc, clippy::unused_async)]
    pub async fn current_peers(&self) -> Result<Vec<HostPeerSummary>, MinosError> {
        Ok(self.inner.peers.lock().unwrap().clone())
    }

    /// Mint a pairing QR by round-tripping `request_pairing_token` to
    /// the relay and packaging the token with the baked-in mac name and
    /// backend URL.
    #[allow(clippy::missing_errors_doc)]
    pub async fn pairing_qr(&self) -> Result<RelayQrPayload, MinosError> {
        self.inner.relay.request_pairing_token().await
    }

    /// Forget the currently paired peer. Calls the relay first and, on
    /// success, clears the in-memory mirror. The relay will still echo an
    /// `Event::Unpaired`, which is now just a benign in-memory re-apply.
    #[allow(clippy::missing_errors_doc)]
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        let mobile_device_id = self
            .inner
            .peers
            .lock()
            .unwrap()
            .first()
            .map(|peer| peer.mobile_device_id);
        let Some(mobile_device_id) = mobile_device_id else {
            return Ok(());
        };
        self.forget_peer_device(mobile_device_id).await
    }

    /// Forget one specific mobile/account row by its mobile device id.
    #[allow(clippy::missing_errors_doc)]
    pub async fn forget_peer_device(&self, mobile_device_id: DeviceId) -> Result<(), MinosError> {
        self.inner
            .relay
            .forget_peer_device(mobile_device_id)
            .await?;

        let next_peers = {
            let mut guard = self.inner.peers.lock().unwrap();
            guard.retain(|peer| peer.mobile_device_id != mobile_device_id);
            guard.clone()
        };
        *self.inner.peer.lock().unwrap() = next_peers.first().map(peer_record_from_summary);
        Ok(())
    }

    /// Stop the relay client + the embedded agent runtime. Idempotent —
    /// calling twice is a benign no-op after the first success.
    ///
    /// Shutdown sequence:
    /// 1. Best-effort `AgentGlue::shutdown` — suspend (not close) every live
    ///    thread and **synchronously** persist `suspended` + `needs_continue`.
    /// 2. SIGTERM every provider child with a 5s grace, then SIGKILL.
    /// 3. Tear down local RPC discovery + the relay WS client.
    ///
    /// Orphan recovery for unclean kills runs on the **next** start via
    /// `mark_orphans_suspended` (not here).
    #[allow(clippy::missing_errors_doc)]
    pub async fn stop(&self) -> Result<(), MinosError> {
        match self.inner.agent.shutdown().await {
            Ok(()) | Err(MinosError::AgentNotRunning) => {}
            Err(err) => return Err(err),
        }
        self.inner
            .agent
            .manager
            .shutdown_instances(std::time::Duration::from_secs(5))
            .await;
        if let Some(handle) = &self.inner.local_rpc_handle {
            let _ = handle.stop();
        }
        if let Some(path) = &self.inner.local_rpc_discovery_path {
            if let Err(error) = std::fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        target: "minos_daemon::handle",
                        error = %error,
                        path = %path.display(),
                        "failed to remove local RPC discovery file",
                    );
                }
            }
        }
        self.inner.relay.stop().await;
        Ok(())
    }

    /// Drain the last fatal relay-side error, if any. Consuming on read
    /// avoids repeatedly flagging the same failure in the UI.
    ///
    /// Populated by the relay-client dispatch task on three paths:
    /// - pre-upgrade HTTP 401 → `Unauthorized { reason: <resp body> }`
    /// - WS close 4401 → `DeviceNotTrusted { device_id: self_device_id }`
    /// - WS close 4400 → `EnvelopeVersionUnsupported { version: 1 }`
    ///
    /// Swift reads this after observing a `RelayLinkState::Disconnected`
    /// and promotes the value into `AppState.bootError` / `displayError`
    /// so the onboarding or settings sheet can explain *why* the link
    /// went down.
    #[must_use]
    pub fn last_error(&self) -> Option<MinosError> {
        self.inner.last_error.lock().unwrap().take()
    }

    /// Push-model relay-link subscription for UniFFI. Delivers the
    /// current snapshot synchronously, then one callback per transition
    /// until the `Subscription` is cancelled.
    #[must_use]
    pub fn subscribe_relay_link(
        &self,
        observer: Arc<dyn crate::subscription::RelayLinkStateObserver>,
    ) -> Arc<crate::subscription::Subscription> {
        // Match `subscribe_agent_state`: enter the captured runtime so
        // Swift's "no current reactor" sessions still land a `spawn`.
        let _guard = self.inner.rt_handle.enter();
        crate::subscription::spawn_relay_link_observer(self.inner.link_rx.clone(), observer)
    }

    /// Push-model peer-pairing subscription. Symmetric to
    /// `subscribe_relay_link` — see that method's doc for the runtime
    /// contract.
    #[must_use]
    pub fn subscribe_peer(
        &self,
        observer: Arc<dyn crate::subscription::PeerStateObserver>,
    ) -> Arc<crate::subscription::Subscription> {
        let _guard = self.inner.rt_handle.enter();
        crate::subscription::spawn_peer_observer(self.inner.peer_rx.clone(), observer)
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

// ── Agent-runtime methods (unchanged from the pre-relay surface) ──
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl DaemonHandle {
    #[allow(clippy::missing_errors_doc)]
    pub async fn start_agent(
        &self,
        req: minos_protocol::StartAgentRequest,
    ) -> Result<minos_protocol::StartAgentResponse, MinosError> {
        self.inner.agent.start_agent(req).await
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn send_user_message(
        &self,
        req: minos_protocol::SendUserMessageRequest,
    ) -> Result<(), MinosError> {
        self.inner.agent.send_user_message(req).await
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn interrupt_session(
        &self,
        req: minos_protocol::InterruptSessionRequest,
    ) -> Result<(), MinosError> {
        self.inner.agent.interrupt_session(req).await
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn close_session(
        &self,
        req: minos_protocol::CloseSessionRequest,
    ) -> Result<(), MinosError> {
        self.inner.agent.close_session(req).await
    }

    #[must_use]
    pub fn subscribe_agent_state(
        &self,
        observer: Arc<dyn crate::subscription::AgentStateObserver>,
    ) -> Arc<crate::subscription::Subscription> {
        let _guard = self.inner.rt_handle.enter();
        self.inner.agent.subscribe_state(observer)
    }

    #[must_use]
    pub fn current_agent_state(&self) -> crate::SessionState {
        self.inner.agent.current_state()
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn current_agent_session(
        &self,
    ) -> Result<Option<crate::agent::AgentSessionSnapshot>, MinosError> {
        self.inner.agent.current_agent_session().await
    }
}

// ── Agent-runtime methods served only over JSON-RPC, not UniFFI. ──
//
// `list_sessions` and `get_session` traffic in `minos_protocol::SessionSummary`
// / `SessionState` mirrors that intentionally do not derive `uniffi::*` (the
// canonical FFI-side `SessionState` is the runtime crate's enum; duplicating
// it via UniFFI would collide in the shared Swift module). Mobile (frb)
// reaches these methods via the JSON-RPC server in `rpc_server.rs`, which
// is unaffected. Macos Swift does not call them today.
impl DaemonHandle {
    #[allow(clippy::missing_errors_doc)]
    pub async fn list_sessions(
        &self,
        req: minos_protocol::ListSessionsParams,
    ) -> Result<minos_protocol::ListSessionsResponse, MinosError> {
        self.inner.agent.list_sessions(req).await
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn get_session(
        &self,
        req: minos_protocol::GetSessionParams,
    ) -> Result<minos_protocol::GetSessionResponse, MinosError> {
        self.inner.agent.get_session(req).await
    }
}
