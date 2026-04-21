//! `MinosRpcServer` impl that routes to inner services.
//!
//! Holds `Arc`s only — cheap to clone once and pass into the jsonrpsee
//! `RpcModule`.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use chrono::Utc;
use jsonrpsee::core::async_trait;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::PendingSubscriptionSink;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::MinosError;
use minos_pairing::{Pairing, PairingStore, TrustedDevice};
use minos_protocol::{HealthResponse, ListClisResponse, MinosRpcServer, PairRequest, PairResponse};

pub struct RpcServerImpl {
    pub started_at: Instant,
    pub pairing: Arc<Mutex<Pairing>>,
    pub store: Arc<dyn PairingStore>,
    pub runner: Arc<dyn CommandRunner>,
    pub mac_name: String,
    pub host: String,
    pub port: u16,
}

#[async_trait]
impl MinosRpcServer for RpcServerImpl {
    async fn pair(&self, req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse> {
        // Token validation happens at the WS-upgrade layer (future task); by
        // the time `pair` is called the token is already verified. Here we
        // only gate on the state machine.
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

    async fn subscribe_events(&self, pending: PendingSubscriptionSink) -> SubscriptionResult {
        // MVP: not implemented; reject the subscription.
        // jsonrpsee 0.24: `pending.reject()` closes the upgrade with an error.
        pending
            .reject(ErrorObjectOwned::owned(
                4001,
                "subscribe_events not yet implemented (P1)",
                None::<()>,
            ))
            .await;
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
