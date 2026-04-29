//! Agent-host → backend ingest WS client.
//!
//! Bespoke envelope-speaking loop that forwards raw JSON-RPC notifications
//! from the agent CLI to `minos-backend` as `Envelope::Ingest` frames.
//!
//! This is **not** `minos-transport::WsClient` (that crate wraps jsonrpsee;
//! we speak a plain envelope protocol here). Design:
//!
//! 1. [`Ingestor::connect`] opens a WS to `ws(s)://<host>/devices` with the
//!    agent-host's `X-Device-Id` (+ optional `X-Device-Secret`) headers.
//!    Pairing itself flows via the backend's HTTP `/v1/pairing/*` routes;
//!    first-boot pairing is driven by the outer `AgentRuntime` + daemon, not
//!    by this type.
//! 2. A bounded `mpsc` channel serialises outbound writes so `push()` is
//!    safe to call from the agent-runtime's event pump task without holding
//!    the socket.
//! 3. A per-thread seq counter lives here (`DashMap<String, u64>`). Seq is
//!    a transport concern — the agent runtime's `RawIngest` broadcast does
//!    not carry it. The backend idempotently inserts on `(thread_id, seq)`
//!    so retransmits are safe.
//! 4. Inbound `Envelope::Event` frames are logged at debug level for now;
//!    the agent-host does not need to consume them in Phase B. Later phases
//!    may thread a subscriber channel for `PeerOnline` / `ServerShutdown`.

use std::sync::Arc;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use minos_domain::{AgentName, MinosError};
use minos_protocol::Envelope;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Public handle for pushing raw events to the backend.
pub struct Ingestor {
    tx: mpsc::Sender<Envelope>,
    seqs: Arc<DashMap<String, u64>>,
}

/// Holds the inbound + outbound task handles. Drop the handle to tear the
/// WS down gracefully (both tasks observe channel close and exit).
pub struct IngestorHandle {
    _recv_handle: tokio::task::JoinHandle<()>,
    _send_handle: tokio::task::JoinHandle<()>,
}

impl Ingestor {
    /// Connect to the backend and spawn the receive + send tasks.
    ///
    /// `device_id` goes into the `X-Device-Id` header. `device_secret`, if
    /// supplied, goes into `X-Device-Secret`; omit it on first-boot before
    /// pairing has completed. `X-Device-Role: agent-host` is always set.
    ///
    /// Returns `(Ingestor, IngestorHandle)`. The handle keeps the tasks
    /// alive; dropping it closes the socket.
    pub async fn connect(
        url: &str,
        device_id: &str,
        device_secret: Option<&str>,
        cf_access: Option<(&str, &str)>,
    ) -> Result<(Self, IngestorHandle), MinosError> {
        let mut req = url
            .into_client_request()
            .map_err(|e| MinosError::ConnectFailed {
                url: url.to_string(),
                message: e.to_string(),
            })?;
        let headers = req.headers_mut();
        headers.insert(
            "X-Device-Id",
            device_id.parse().map_err(|_| MinosError::ConnectFailed {
                url: url.to_string(),
                message: "device_id is not a valid header value".into(),
            })?,
        );
        headers.insert(
            "X-Device-Role",
            "agent-host".parse().expect("static header value is valid"),
        );
        if let Some(sec) = device_secret {
            headers.insert(
                "X-Device-Secret",
                sec.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "device_secret is not a valid header value".into(),
                })?,
            );
        }
        if let Some((cf_id, cf_secret)) = cf_access {
            headers.insert(
                "CF-Access-Client-Id",
                cf_id.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "cf_client_id is not a valid header value".into(),
                })?,
            );
            headers.insert(
                "CF-Access-Client-Secret",
                cf_secret.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "cf_client_secret is not a valid header value".into(),
                })?,
            );
        }

        let (ws, _resp) = connect_async(req)
            .await
            .map_err(|e| MinosError::ConnectFailed {
                url: url.to_string(),
                message: e.to_string(),
            })?;
        let (mut write, mut read) = ws.split();

        let (tx, mut rx) = mpsc::channel::<Envelope>(256);

        let send_handle = tokio::spawn(async move {
            while let Some(env) = rx.recv().await {
                let text = match serde_json::to_string(&env) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(?e, "ingest envelope serialise failed");
                        continue;
                    }
                };
                if let Err(e) = write.send(Message::Text(text.into())).await {
                    tracing::warn!(?e, "ingest WS write failed; send loop exiting");
                    break;
                }
            }
        });

        let recv_handle = tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(t)) => {
                        // Backend Envelope::Event frames — not consumed yet.
                        tracing::debug!(text = %t, "ingest WS inbound");
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(?e, "ingest WS read error");
                        break;
                    }
                }
            }
        });

        Ok((
            Self {
                tx,
                seqs: Arc::new(DashMap::new()),
            },
            IngestorHandle {
                _recv_handle: recv_handle,
                _send_handle: send_handle,
            },
        ))
    }

    /// Push one raw event for ingest. Builds the `Envelope::Ingest` with a
    /// monotonic per-thread seq and sends it through the outbound channel.
    /// Blocks on backpressure (bounded channel capacity 256).
    pub async fn push(
        &self,
        agent: AgentName,
        thread_id: &str,
        payload: serde_json::Value,
    ) -> Result<(), MinosError> {
        let seq = self.next_seq(thread_id);
        let env = Envelope::Ingest {
            version: 1,
            agent,
            thread_id: thread_id.to_string(),
            seq,
            payload,
            ts_ms: current_unix_ms(),
        };
        self.tx
            .send(env)
            .await
            .map_err(|_| MinosError::Disconnected {
                reason: "ingest outbound channel closed".into(),
            })?;
        Ok(())
    }

    fn next_seq(&self, thread_id: &str) -> u64 {
        let mut entry = self.seqs.entry(thread_id.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }
}

fn current_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
