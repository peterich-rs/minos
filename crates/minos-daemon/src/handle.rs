//! Public façade exposed to Swift via UniFFI, rewired for the relay-client
//! migration (plan 05 Phase F).
//!
//! `DaemonInner` owns the outbound [`RelayClient`] plus its two watch
//! receivers (relay link + peer) and the current in-memory trusted peer.
//! Sync FFI methods dispatch onto `rt_handle` so Swift's non-runtime
//! threads can still enter the Tokio reactor — same trick the old
//! WS-server façade used.

use std::sync::{Arc, Mutex as StdMutex};

use minos_domain::{DeviceId, DeviceSecret, MinosError};
use tokio::runtime::Handle;
use tokio::sync::watch;

use minos_protocol::Envelope;
use tokio::sync::mpsc;

use crate::agent::AgentGlue;
use crate::config::RelayConfig;
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
    /// Kept on the inner — future trace logging and eventual UniFFI
    /// getters need the display name that was minted into the relay
    /// handshake.
    #[allow(dead_code)]
    mac_name: String,
    /// Populated by the relay-client task on fatal exit paths (pre-upgrade
    /// HTTP 401 → `CfAuthFailed`; post-upgrade WS close 4401 →
    /// `DeviceNotTrusted`; close 4400 → `EnvelopeVersionUnsupported`).
    /// Drained on read so the UI sees each failure at most once per
    /// occurrence.
    last_error: Arc<StdMutex<Option<MinosError>>>,
    agent: Arc<AgentGlue>,
    /// Captured under `DaemonHandle::start` (which always runs inside a
    /// Tokio runtime — either the CLI's `#[tokio::main]` or UniFFI's
    /// tokio runtime) so sync FFI methods can spawn onto it from Swift
    /// threads that lack a current runtime.
    rt_handle: Handle,
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
        // Capture the user's login-shell env once. Failures fall back to
        // process env internally, so this never blocks bootstrap.
        let subprocess_env = Arc::new(minos_cli_detect::capture_user_shell_env().await);

        // Open the daemon's local SQLite store. The schema is migrated on
        // first open via sqlx::migrate! against `crates/minos-daemon/migrations`.
        let db_path = paths::minos_home()?.join("daemon.sqlite");
        let store = Arc::new(crate::store::LocalStore::open(&db_path).await.map_err(
            |e| MinosError::StoreIo {
                path: db_path.display().to_string(),
                message: format!("LocalStore::open failed: {e}"),
            },
        )?);

        // Build the agent glue ahead of the relay. The agent's `EventWriter`
        // consumes a relay-out mpsc; we cannot use `relay.outbound_sender()`
        // yet because the relay needs an `RpcServerImpl` that references the
        // agent. Solve the cycle with a local forwarder channel that's wired
        // to the relay's outbound queue once both halves exist.
        let (agent_out_tx, mut agent_out_rx) = mpsc::channel::<Envelope>(256);
        let agent = Arc::new(AgentGlue::new(
            paths::minos_home()?.join("workspaces"),
            subprocess_env.clone(),
            store.clone(),
            agent_out_tx,
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
        let last_error: Arc<StdMutex<Option<MinosError>>> = Arc::new(StdMutex::new(None));

        let backend_url = config.resolved_backend_url().to_owned();

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
                last_error: last_error.clone(),
            },
        );

        // Forward agent ingest envelopes (already persisted by the
        // EventWriter inside AgentGlue) into the relay's outbound queue.
        // The single `/devices` WS the dispatcher owns carries both
        // peer-to-peer `Forward` traffic and host `Ingest` frames — the
        // backend's session registry is keyed by `DeviceId` alone, so a
        // second WS handshake from the same id would supersede the first
        // in a tight loop.
        let relay_out = relay.outbound_sender();
        tokio::spawn(async move {
            while let Some(env) = agent_out_rx.recv().await {
                if relay_out.send(env).await.is_err() {
                    break;
                }
            }
        });

        Ok(Arc::new(Self {
            inner: Arc::new(DaemonInner {
                relay,
                link_rx,
                peer_rx,
                peer: peer_store,
                mac_name,
                last_error,
                agent,
                rt_handle: Handle::current(),
            }),
        }))
    }

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
        self.inner.relay.forget_peer().await?;
        *self.inner.peer.lock().unwrap() = None;
        Ok(())
    }

    /// Stop the relay client + the embedded agent runtime. Idempotent —
    /// calling twice is a benign no-op after the first success.
    #[allow(clippy::missing_errors_doc)]
    pub async fn stop(&self) -> Result<(), MinosError> {
        match self.inner.agent.shutdown().await {
            Ok(()) | Err(MinosError::AgentNotRunning) => {}
            Err(err) => return Err(err),
        }
        self.inner.relay.stop().await;
        Ok(())
    }

    /// Drain the last fatal relay-side error, if any. Consuming on read
    /// avoids repeatedly flagging the same failure in the UI.
    ///
    /// Populated by the relay-client dispatch task on three paths:
    /// - pre-upgrade HTTP 401 → `CfAuthFailed { message: <resp body> }`
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
        // Swift's "no current reactor" threads still land a `spawn`.
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
    pub async fn stop_agent(&self) -> Result<(), MinosError> {
        self.inner.agent.stop_agent().await
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
    pub fn current_agent_state(&self) -> crate::AgentState {
        self.inner.agent.current_state()
    }
}
