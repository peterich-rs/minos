//! WebSocket-side jsonrpsee client. Reconnect orchestration is the caller's
//! responsibility (use `backoff::delay_for_attempt`).

use std::sync::Arc;

use jsonrpsee::ws_client::{WsClient as JsonRpcWsClient, WsClientBuilder};
use minos_domain::MinosError;
use url::Url;

pub struct WsClient {
    inner: Arc<JsonRpcWsClient>,
}

impl WsClient {
    pub async fn connect(url: &Url) -> Result<Self, MinosError> {
        let inner = WsClientBuilder::default()
            .build(url.as_str())
            .await
            .map_err(|e| MinosError::ConnectFailed {
                url: url.to_string(),
                message: e.to_string(),
            })?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    #[must_use]
    pub fn inner(&self) -> Arc<JsonRpcWsClient> {
        self.inner.clone()
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WsServer;
    use jsonrpsee::server::RpcModule;

    #[tokio::test]
    async fn client_connects_to_local_server() {
        // Spin up an empty server, then connect.
        let server = WsServer::bind("127.0.0.1:0".parse().unwrap(), RpcModule::new(()))
            .await
            .unwrap();
        let url = format!("ws://{}", server.addr()).parse().unwrap();
        let client = WsClient::connect(&url).await.unwrap();
        assert!(client.is_connected());
        server.stop().await.unwrap();
    }
}
