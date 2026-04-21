//! Public façade exposed to Swift via UniFFI in plan 02. This crate only
//! exposes the Rust shape; UniFFI annotations live in `minos-ffi-uniffi`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jsonrpsee::server::RpcModule;
use minos_cli_detect::{CommandRunner, RealCommandRunner};
use minos_domain::{ConnectionState, MinosError, PairingState};
use minos_pairing::{generate_qr_payload, ActiveToken, Pairing, PairingStore, QrPayload};
use minos_protocol::MinosRpcServer;
use minos_transport::WsServer;
use tokio::sync::watch;

use crate::file_store::FilePairingStore;
use crate::rpc_server::RpcServerImpl;
use crate::tailscale;

pub struct DaemonConfig {
    pub mac_name: String,
    pub bind_addr: SocketAddr,
}

pub struct DaemonHandle {
    server: Option<WsServer>,
    state_rx: watch::Receiver<ConnectionState>,
    #[allow(dead_code)]
    state_tx: watch::Sender<ConnectionState>,
    pairing: Arc<Mutex<Pairing>>,
    #[allow(dead_code)]
    store: Arc<dyn PairingStore>,
    active_token: Arc<Mutex<Option<ActiveToken>>>,
    addr: SocketAddr,
    mac_name: String,
}

impl DaemonHandle {
    /// Start the daemon. Binds to the supplied address and serves the RPC
    /// module in a background task. Returns once the listener is bound.
    #[allow(clippy::missing_errors_doc)]
    pub async fn start(cfg: DaemonConfig) -> Result<Self, MinosError> {
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

        let impl_ = RpcServerImpl {
            started_at: Instant::now(),
            pairing: pairing.clone(),
            store: store.clone(),
            runner,
            mac_name: cfg.mac_name.clone(),
            host: cfg.bind_addr.ip().to_string(),
            port: cfg.bind_addr.port(),
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

        Ok(Self {
            server: Some(server),
            state_rx,
            state_tx,
            pairing,
            store,
            active_token: Arc::new(Mutex::new(None)),
            addr,
            mac_name: cfg.mac_name,
        })
    }

    /// Generate (or refresh) the pairing QR.
    #[allow(clippy::missing_errors_doc)]
    pub fn pairing_qr(&self) -> Result<QrPayload, MinosError> {
        let mut p = self.pairing.lock().unwrap();
        if p.state() == PairingState::Paired {
            // Caller wants to re-pair — UI must have shown a "replace" confirm.
            p.replace()?;
        } else if p.state() == PairingState::Unpaired {
            p.begin_awaiting()?;
        }
        let (payload, active) = generate_qr_payload(
            self.addr.ip().to_string(),
            self.addr.port(),
            self.mac_name.clone(),
        );
        *self.active_token.lock().unwrap() = Some(active);
        Ok(payload)
    }

    pub async fn discover_tailscale_ip(&self) -> Option<String> {
        tailscale::discover_ip().await
    }

    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn stop(mut self) -> Result<(), MinosError> {
        if let Some(s) = self.server.take() {
            s.stop().await?;
        }
        Ok(())
    }
}
