//! `MinosRpcServer` impl that routes to inner services.
//!
//! Holds `Arc`s only — cheap to clone once and pass into the jsonrpsee
//! `RpcModule`.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use chrono::Utc;
use jsonrpsee::core::async_trait;
use jsonrpsee::core::server::SubscriptionMessage;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::PendingSubscriptionSink;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::{ConnectionState, MinosError};
use minos_pairing::{ActiveToken, Pairing, PairingStore, TrustedDevice};
use minos_protocol::{
    HealthResponse, ListClisResponse, MinosRpcServer, PairRequest, PairResponse,
    SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
};
use tokio::sync::{broadcast, watch};

use crate::agent::AgentGlue;

pub struct RpcServerImpl {
    pub started_at: Instant,
    pub pairing: Arc<Mutex<Pairing>>,
    pub store: Arc<dyn PairingStore>,
    pub runner: Arc<dyn CommandRunner>,
    pub mac_name: String,
    pub host: String,
    pub port: u16,
    /// Active pairing token shared with the `DaemonHandle` that issued the QR.
    /// `pair()` validates the request token against this and clears it on
    /// successful consumption.
    pub active_token: Arc<Mutex<Option<ActiveToken>>>,
    /// Connection-state broadcaster shared with the `DaemonHandle`. After a
    /// successful `pair()`, this emits `Connected` so UI receivers learn
    /// about the new peer without a separate transport-layer event.
    pub conn_state_tx: Arc<watch::Sender<ConnectionState>>,
    pub agent: Arc<AgentGlue>,
}

#[async_trait]
impl MinosRpcServer for RpcServerImpl {
    async fn pair(&self, req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse> {
        // Validate the one-shot pairing token before mutating any state.
        // Spec §6.4: token IS validated server-side; the WS-upgrade layer
        // is a future hardening, not a substitute for this check.
        {
            let token_guard = self.active_token.lock().unwrap();
            let active = token_guard
                .as_ref()
                .ok_or_else(|| rpc_err(MinosError::PairingTokenInvalid))?;
            if active.is_expired(Utc::now()) {
                return Err(rpc_err(MinosError::PairingTokenInvalid));
            }
            if active.token != req.token {
                return Err(rpc_err(MinosError::PairingTokenInvalid));
            }
        }
        // Token consumed — invalidate so it cannot be replayed.
        *self.active_token.lock().unwrap() = None;

        {
            let mut p = self.pairing.lock().unwrap();
            p.accept_peer().map_err(rpc_err)?;
        }

        let mut current = self.store.load().map_err(rpc_err)?;
        let dev = TrustedDevice {
            device_id: req.device_id,
            name: req.name,
            host: self.host.clone(),
            port: self.port,
            paired_at: Utc::now(),
        };
        // Replace any existing entry for the same device_id; otherwise append.
        if let Some(idx) = current.iter().position(|d| d.device_id == req.device_id) {
            current[idx] = dev;
        } else {
            current.push(dev);
        }
        self.store.save(&current).map_err(rpc_err)?;

        // Surface the new peer to events_stream subscribers. jsonrpsee does
        // not give us a connection-lifecycle hook; emit on successful pair.
        let _ = self.conn_state_tx.send(ConnectionState::Connected);

        Ok(PairResponse {
            ok: true,
            mac_name: self.mac_name.clone(),
        })
    }

    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse> {
        Ok(HealthResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_secs: self.started_at.elapsed().as_secs(),
        })
    }

    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<ListClisResponse> {
        Ok(detect_all(self.runner.clone()).await)
    }

    async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        self.agent.start_agent(req).await.map_err(rpc_err)
    }

    async fn send_user_message(
        &self,
        req: SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.send_user_message(req).await.map_err(rpc_err)
    }

    async fn stop_agent(&self) -> jsonrpsee::core::RpcResult<()> {
        self.agent.stop_agent().await.map_err(rpc_err)
    }

    async fn subscribe_events(&self, pending: PendingSubscriptionSink) -> SubscriptionResult {
        let mut rx = self.agent.event_stream();
        let sink = pending.accept().await?;

        loop {
            match rx.recv().await {
                Ok(evt) => {
                    let message = SubscriptionMessage::from_json(&evt)?;
                    if sink.send(message).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "subscribe_events subscriber lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }

        Ok(())
    }
}

fn rpc_err(e: MinosError) -> ErrorObjectOwned {
    let code = match e {
        MinosError::PairingStateMismatch { .. } => -32001,
        MinosError::PairingTokenInvalid => -32002,
        MinosError::DeviceNotTrusted { .. } => -32003,
        _ => -32000,
    };
    ErrorObjectOwned::owned(code, e.to_string(), None::<()>)
}
