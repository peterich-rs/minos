//! `MobileClient` — what Dart calls into through frb (plan 03).

use std::sync::Arc;

use chrono::Utc;
use minos_domain::{ConnectionState, DeviceId, MinosError};
use minos_pairing::{PairingStore, QrPayload, TrustedDevice};
use minos_protocol::{MinosRpcClient, PairRequest, PairResponse};
use minos_transport::WsClient;
use tokio::sync::watch;
use url::Url;

pub struct MobileClient {
    store: Arc<dyn PairingStore>,
    ws: Arc<tokio::sync::Mutex<Option<WsClient>>>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    device_id: DeviceId,
    self_name: String,
}

impl MobileClient {
    #[must_use]
    pub fn new(store: Arc<dyn PairingStore>, self_name: String) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        Self {
            store,
            ws: Arc::new(tokio::sync::Mutex::new(None)),
            state_tx,
            state_rx,
            device_id: DeviceId::new(),
            self_name,
        }
    }

    /// Pair with a Mac whose QR was just scanned.
    #[allow(clippy::missing_errors_doc)]
    pub async fn pair_with(&self, qr: QrPayload) -> Result<PairResponse, MinosError> {
        let url: Url =
            format!("ws://{}:{}", qr.host, qr.port)
                .parse()
                .map_err(|e: url::ParseError| MinosError::ConnectFailed {
                    url: format!("ws://{}:{}", qr.host, qr.port),
                    message: e.to_string(),
                })?;

        let _ = self.state_tx.send(ConnectionState::Pairing);
        let ws = WsClient::connect(&url).await?;

        let resp = MinosRpcClient::pair(
            &*ws.inner(),
            PairRequest {
                device_id: self.device_id,
                name: self.self_name.clone(),
            },
        )
        .await
        .map_err(|e| MinosError::RpcCallFailed {
            method: "pair".into(),
            message: e.to_string(),
        })?;

        // Persist trusted Mac.
        let dev = TrustedDevice {
            device_id: self.device_id,
            name: resp.mac_name.clone(),
            host: qr.host,
            port: qr.port,
            paired_at: Utc::now(),
        };
        self.store.save(&[dev])?;

        *self.ws.lock().await = Some(ws);
        let _ = self.state_tx.send(ConnectionState::Connected);
        Ok(resp)
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn list_clis(&self) -> Result<Vec<minos_domain::AgentDescriptor>, MinosError> {
        let guard = self.ws.lock().await;
        let ws = guard.as_ref().ok_or(MinosError::Disconnected {
            reason: "no client".into(),
        })?;
        MinosRpcClient::list_clis(&*ws.inner())
            .await
            .map_err(|e| MinosError::RpcCallFailed {
                method: "list_clis".into(),
                message: e.to_string(),
            })
    }

    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    /// Subscribe to connection-state transitions. The first `borrow` returns
    /// the most recent value; each subsequent `changed().await` resolves on
    /// the next transition.
    #[must_use]
    pub fn events_stream(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Forget the current pairing. Clears the trusted-device store, drops
    /// the WS connection, and emits `Disconnected`. Idempotent.
    #[allow(clippy::missing_errors_doc)]
    pub async fn forget_device(&self) -> Result<(), MinosError> {
        self.store.save(&[])?;
        *self.ws.lock().await = None;
        let _ = self.state_tx.send(ConnectionState::Disconnected);
        Ok(())
    }
}
