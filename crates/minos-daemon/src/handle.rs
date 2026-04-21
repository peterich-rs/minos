//! Public façade exposed to Swift via UniFFI in plan 02.
//!
//! Plan 02 Phase 0 refactor: all fields live inside `DaemonInner` owned by
//! an `Arc`, so every `DaemonHandle` method takes `&self` — a requirement
//! for UniFFI `#[uniffi::Object]` exports. `WsServer` uses interior
//! mutability via `Mutex<Option<_>>` so `stop(&self)` can take it out
//! without consuming the handle.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jsonrpsee::server::RpcModule;
use minos_cli_detect::{CommandRunner, RealCommandRunner};
use minos_domain::{ConnectionState, DeviceId, MinosError, PairingState};
use minos_pairing::{generate_qr_payload, ActiveToken, Pairing, PairingStore, QrPayload};
use minos_protocol::MinosRpcServer;
use minos_transport::WsServer;
use tokio::sync::watch;

use crate::file_store::FilePairingStore;
use crate::rpc_server::RpcServerImpl;

pub struct DaemonConfig {
    pub mac_name: String,
    pub bind_addr: SocketAddr,
}

struct DaemonInner {
    server: Mutex<Option<WsServer>>,
    state_rx: watch::Receiver<ConnectionState>,
    state_tx: Arc<watch::Sender<ConnectionState>>,
    pairing: Arc<Mutex<Pairing>>,
    store: Arc<dyn PairingStore>,
    active_token: Arc<Mutex<Option<ActiveToken>>>,
    addr: SocketAddr,
    mac_name: String,
}

pub struct DaemonHandle {
    inner: Arc<DaemonInner>,
}

impl DaemonHandle {
    /// Start the daemon on an explicit bind address. Tests use this path;
    /// production code uses `start_autobind` (Task 8).
    #[allow(clippy::missing_errors_doc)]
    pub async fn start(cfg: DaemonConfig) -> Result<Arc<Self>, MinosError> {
        let store: Arc<dyn PairingStore> =
            Arc::new(FilePairingStore::new(FilePairingStore::default_path()));
        let runner: Arc<dyn CommandRunner> = Arc::new(RealCommandRunner);

        let initial_state = if store.load()?.is_empty() {
            PairingState::Unpaired
        } else {
            PairingState::Paired
        };
        let pairing = Arc::new(Mutex::new(Pairing::new(initial_state)));

        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let state_tx = Arc::new(state_tx);
        let active_token: Arc<Mutex<Option<ActiveToken>>> = Arc::new(Mutex::new(None));

        let impl_ = RpcServerImpl {
            started_at: Instant::now(),
            pairing: pairing.clone(),
            store: store.clone(),
            runner,
            mac_name: cfg.mac_name.clone(),
            host: cfg.bind_addr.ip().to_string(),
            port: cfg.bind_addr.port(),
            active_token: active_token.clone(),
            conn_state_tx: state_tx.clone(),
        };

        let mut module = RpcModule::new(());
        module
            .merge(impl_.into_rpc())
            .map_err(|e| MinosError::BindFailed {
                addr: cfg.bind_addr.to_string(),
                message: e.to_string(),
            })?;

        let server = WsServer::bind(cfg.bind_addr, module).await?;
        let addr = server.addr();

        let _ = state_tx.send(ConnectionState::Disconnected);

        Ok(Arc::new(Self {
            inner: Arc::new(DaemonInner {
                server: Mutex::new(Some(server)),
                state_rx,
                state_tx,
                pairing,
                store,
                active_token,
                addr,
                mac_name: cfg.mac_name,
            }),
        }))
    }

    /// Generate (or refresh) the pairing QR.
    #[allow(clippy::missing_errors_doc)]
    pub fn pairing_qr(&self) -> Result<QrPayload, MinosError> {
        let mut p = self.inner.pairing.lock().unwrap();
        if p.state() == PairingState::Paired {
            p.replace()?;
        } else if p.state() == PairingState::Unpaired {
            p.begin_awaiting()?;
        }
        let (payload, active) = generate_qr_payload(
            self.inner.addr.ip().to_string(),
            self.inner.addr.port(),
            self.inner.mac_name.clone(),
        );
        *self.inner.active_token.lock().unwrap() = Some(active);
        let _ = self.inner.state_tx.send(ConnectionState::Pairing);
        Ok(payload)
    }

    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.inner.state_rx.borrow()
    }

    /// Subscribe to connection-state transitions (Rust-only — UniFFI callers
    /// use the callback-interface `subscribe` added in Task 9).
    #[must_use]
    pub fn events_stream(&self) -> watch::Receiver<ConnectionState> {
        self.inner.state_rx.clone()
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.inner.addr
    }

    /// Bound host as a string (Tailscale 100.x or the loopback 127.0.0.1
    /// used by tests). Exported to Swift via UniFFI.
    #[must_use]
    pub fn host(&self) -> String {
        self.inner.addr.ip().to_string()
    }

    /// Bound TCP port after auto-retry. Exported to Swift via UniFFI.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.inner.addr.port()
    }

    /// Forget a previously trusted device.
    #[allow(clippy::missing_errors_doc, clippy::unused_async)]
    pub async fn forget_device(&self, id: DeviceId) -> Result<(), MinosError> {
        let mut current = self.inner.store.load()?;
        current.retain(|d| d.device_id != id);
        self.inner.store.save(&current)?;
        self.inner.pairing.lock().unwrap().forget();
        let _ = self.inner.state_tx.send(ConnectionState::Disconnected);
        Ok(())
    }

    /// Stop the WS server and transition to `Disconnected`. Idempotent —
    /// calling twice is a no-op after the first success.
    #[allow(clippy::missing_errors_doc)]
    pub async fn stop(&self) -> Result<(), MinosError> {
        let server = self.inner.server.lock().unwrap().take();
        if let Some(s) = server {
            s.stop().await?;
        }
        let _ = self.inner.state_tx.send(ConnectionState::Disconnected);
        Ok(())
    }
}
