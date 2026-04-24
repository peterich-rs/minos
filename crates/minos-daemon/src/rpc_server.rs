//! `MinosRpcServer` impl that routes to inner services.
//!
//! Holds `Arc`s only — cheap to clone once and pass into the jsonrpsee
//! `RpcModule`.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use jsonrpsee::core::async_trait;
use jsonrpsee::core::server::SubscriptionMessage;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::PendingSubscriptionSink;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::{ConnectionState, MinosError};
use minos_pairing::{ActiveToken, Pairing, PairingStore};
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
    async fn pair(&self, _req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse> {
        // Pairing is owned end-to-end by the relay broker (plan 05 Phase F.3).
        // The Mac receives a Paired event from the relay's `Pair` LocalRpc
        // handler — it never sees a peer-originated `pair` JSON-RPC. If a
        // forwarded JSON-RPC frame somehow reaches here, the right answer is
        // that the host explicitly does not trust this surface for pairing.
        Err(rpc_err(MinosError::Unauthorized {
            reason: "pair handled by relay, not host".into(),
        }))
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
