use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::UnixListener;
use tracing::{debug, warn};

use crate::mcp_socket::{SocketRequest, SocketResponse, MAX_FRAME_LEN};

pub type ToolCallback =
    Arc<dyn Fn(SocketRequest) -> tokio::task::JoinHandle<Result<SocketResponse>> + Send + Sync>;

pub struct McpSocketHandler {
    socket_path: PathBuf,
    callback: ToolCallback,
}

impl McpSocketHandler {
    pub fn new(socket_path: PathBuf, callback: ToolCallback) -> Self {
        Self {
            socket_path,
            callback,
        }
    }

    pub async fn run(&self) -> Result<()> {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(&self.socket_path).with_context(|| {
            format!(
                "failed to bind MCP socket at {}",
                self.socket_path.display()
            )
        })?;
        debug!(
            target: "minos_mcp_handler",
            path = %self.socket_path.display(),
            "MCP socket listener started"
        );
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let callback = self.callback.clone();
                    tokio::spawn(async move {
                        if let Err(error) = Self::handle_connection(stream, callback).await {
                            debug!(
                                target: "minos_mcp_handler",
                                error = %error,
                                "MCP socket connection ended"
                            );
                        }
                    });
                }
                Err(error) => {
                    warn!(
                        target: "minos_mcp_handler",
                        error = %error,
                        "failed to accept MCP socket connection"
                    );
                }
            }
        }
    }

    async fn handle_connection(
        stream: tokio::net::UnixStream,
        callback: ToolCallback,
    ) -> Result<()> {
        let (read_half, write_half) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(read_half);
        let mut writer = tokio::io::BufWriter::new(write_half);
        loop {
            let payload_len = read_u32(&mut reader).await?;
            anyhow::ensure!(
                payload_len <= MAX_FRAME_LEN,
                "socket frame too large: {payload_len} bytes"
            );
            let mut payload = vec![0u8; payload_len as usize];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut payload).await?;
            let request: SocketRequest =
                serde_json::from_slice(&payload).context("failed to deserialize socket request")?;
            debug!(
                target: "minos_mcp_handler",
                ?request,
                "received MCP socket request"
            );
            let handle = callback(request);
            let response = match handle.await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => SocketResponse::Error {
                    message: error.to_string(),
                },
                Err(join_error) => SocketResponse::Error {
                    message: join_error.to_string(),
                },
            };
            let response_bytes = serde_json::to_vec(&response)?;
            let response_len =
                u32::try_from(response_bytes.len()).context("response payload too large")?;
            use tokio::io::AsyncWriteExt;
            writer.write_all(&response_len.to_be_bytes()).await?;
            writer.write_all(&response_bytes).await?;
            writer.flush().await?;
        }
    }
}

impl Drop for McpSocketHandler {
    fn drop(&mut self) {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

async fn read_u32<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<u32> {
    let mut buf = [0u8; 4];
    tokio::io::AsyncReadExt::read_exact(reader, &mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}
