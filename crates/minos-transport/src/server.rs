//! Thin wrapper over `jsonrpsee::server::Server` that binds it to a TCP
//! listener (typically the Mac's Tailscale IP) and serves a `MinosRpcServer`.

use std::net::SocketAddr;

use jsonrpsee::server::{RpcModule, Server, ServerHandle};
use minos_domain::MinosError;
use tracing::info;

pub struct WsServer {
    handle: ServerHandle,
    addr: SocketAddr,
}

impl WsServer {
    /// Bind a jsonrpsee server to `addr` and start serving the supplied module.
    /// Returns once the listener is bound (the server runs in a background task).
    pub async fn bind(addr: SocketAddr, module: RpcModule<()>) -> Result<Self, MinosError> {
        let server = Server::builder()
            .build(addr)
            .await
            .map_err(|e| MinosError::BindFailed {
                addr: addr.to_string(),
                message: e.to_string(),
            })?;
        let bound = server.local_addr().map_err(|e| MinosError::BindFailed {
            addr: addr.to_string(),
            message: e.to_string(),
        })?;
        let handle = server.start(module);
        info!(?bound, "WsServer started");
        Ok(Self {
            handle,
            addr: bound,
        })
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn stop(self) -> Result<(), MinosError> {
        self.handle.stop().map_err(|e| MinosError::Disconnected {
            reason: e.to_string(),
        })?;
        self.handle.stopped().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonrpsee::server::RpcModule;

    #[tokio::test]
    async fn binds_to_ephemeral_port() {
        let module = RpcModule::new(());
        let s = WsServer::bind("127.0.0.1:0".parse().unwrap(), module)
            .await
            .unwrap();
        assert_ne!(s.addr().port(), 0);
        s.stop().await.unwrap();
    }
}
