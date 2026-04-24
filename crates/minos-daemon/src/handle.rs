//! Public façade exposed to Swift via UniFFI, rewired for the relay-client
//! migration (plan 05 Phase F).
//!
//! `DaemonInner` owns the outbound [`RelayClient`] plus its two watch
//! receivers (relay link + peer) and the non-secret local state (self
//! `DeviceId`, optional `PeerRecord`, on-disk `local-state.json` path).
//! Sync FFI methods dispatch onto `rt_handle` so Swift's non-runtime
//! threads can still enter the Tokio reactor — same trick the old
//! WS-server façade used.

use std::path::PathBuf;
use std::sync::Arc;

use minos_domain::{DeviceId, DeviceSecret, MinosError};
use tokio::runtime::Handle;
use tokio::sync::{watch, Mutex};

use crate::agent::AgentGlue;
use crate::config::{RelayConfig, BACKEND_URL};
use crate::local_state::LocalState;
use crate::paths;
use crate::relay_client::RelayClient;
use crate::relay_pairing::{PeerRecord, RelayQrPayload};

struct DaemonInner {
    relay: Arc<RelayClient>,
    link_rx: watch::Receiver<minos_domain::RelayLinkState>,
    peer_rx: watch::Receiver<minos_domain::PeerState>,
    self_device_id: DeviceId,
    /// In-memory mirror of the trusted peer. `RelayClient::forget_peer` +
    /// an `Unpaired` event from the relay update this; `DaemonHandle::
    /// current_trusted_device` reads it. Behind a tokio `Mutex` because
    /// `forget_peer` is async.
    peer: Arc<Mutex<Option<PeerRecord>>>,
    local_state_path: PathBuf,
    /// Kept on the inner — future trace logging and eventual UniFFI
    /// getters need the display name that was minted into the relay
    /// handshake.
    #[allow(dead_code)]
    mac_name: String,
    /// Spec §6.5 gap: the relay-client task populates this via a shared
    /// `Arc` when a fatal error tears the link down (CF auth, corrupt
    /// handshake, …). Population is wired in a later Phase F task; here
    /// we only declare the field and expose the getter so Swift callers
    /// have a stable surface.
    last_error: Arc<std::sync::Mutex<Option<MinosError>>>,
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
    /// the compile-time [`BACKEND_URL`] over WSS and publishes two
    /// independent watch channels: relay-link and peer-pairing.
    ///
    /// `peer` and `secret` are the persisted parts of a prior pairing —
    /// callers pass `None` for a first run, or the loaded `LocalState`
    /// + Keychain lookup on warm start.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    #[allow(clippy::missing_errors_doc, clippy::unused_async)]
    pub async fn start(
        config: RelayConfig,
        self_device_id: DeviceId,
        peer: Option<PeerRecord>,
        secret: Option<DeviceSecret>,
        mac_name: String,
    ) -> Result<Arc<Self>, MinosError> {
        let local_state_path = LocalState::default_path();

        let (relay, link_rx, peer_rx) = RelayClient::spawn(
            config,
            self_device_id,
            peer.clone(),
            secret,
            mac_name.clone(),
            BACKEND_URL.to_owned(),
        );

        let agent = Arc::new(AgentGlue::new(paths::minos_home()?.join("workspaces")));

        Ok(Arc::new(Self {
            inner: Arc::new(DaemonInner {
                relay,
                link_rx,
                peer_rx,
                self_device_id,
                peer: Arc::new(Mutex::new(peer)),
                local_state_path,
                mac_name,
                last_error: Arc::new(std::sync::Mutex::new(None)),
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
    #[allow(clippy::missing_errors_doc)]
    pub async fn current_trusted_device(&self) -> Result<Option<PeerRecord>, MinosError> {
        Ok(self.inner.peer.lock().await.clone())
    }

    /// Mint a pairing QR by round-tripping `request_pairing_token` to
    /// the relay and packaging the token with the baked-in mac name and
    /// backend URL.
    #[allow(clippy::missing_errors_doc)]
    pub async fn pairing_qr(&self) -> Result<RelayQrPayload, MinosError> {
        self.inner.relay.request_pairing_token().await
    }

    /// Forget the currently paired peer. Calls the relay first; on
    /// success, clears the in-memory mirror, persists an empty
    /// `local-state.json`, and — on macOS — wipes the Keychain entry.
    #[allow(clippy::missing_errors_doc)]
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        self.inner.relay.forget_peer().await?;
        *self.inner.peer.lock().await = None;
        let ls = LocalState {
            self_device_id: self.inner.self_device_id,
            peer: None,
        };
        ls.save(&self.inner.local_state_path)?;
        #[cfg(target_os = "macos")]
        {
            let _ = crate::KeychainTrustedDeviceStore.delete();
        }
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
    /// avoids repeatedly flagging the same failure in the UI. Population
    /// is wired up in a later Phase F task — for now this always returns
    /// `None` (the field starts empty and nothing sets it yet).
    #[must_use]
    pub fn last_error(&self) -> Option<MinosError> {
        // TODO(plan-05 F.x): RelayClient must set this via a shared
        // `Arc<Mutex<Option<MinosError>>>` before exiting on fatal
        // errors (CF auth, bad handshake). See spec §6.5 gap note.
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
