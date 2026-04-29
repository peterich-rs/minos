//! Bridges `AgentGlue::ingest_stream()` (in-process broadcast of `RawIngest`
//! payloads from the codex JSONL pump) onto the `RelayClient`'s outbound
//! queue. Without this forwarder the daemon parses codex output but never
//! pushes it to the backend, so paired peers (mobile) never see assistant
//! text.
//!
//! Design:
//! - One long-lived task per daemon. It does **not** own a WebSocket.
//!   The single `/devices` socket is owned by [`crate::relay_client::RelayClient`];
//!   we hand `Envelope::Ingest` frames to its outbound queue via the
//!   sender returned by `RelayClient::outbound_sender`. The relay
//!   dispatcher drains the queue while connected and buffers up to its
//!   queue depth across reconnect gaps.
//! - Per-thread monotonic `seq` counters live here. The backend
//!   idempotently inserts on `(thread_id, seq)` so retransmits are safe;
//!   the counter is reset only when the daemon restarts (matching the
//!   prior bespoke `Ingestor`'s in-memory behaviour).
//! - On `RecvError::Lagged` we emit a warn log so operators see when the
//!   broadcast buffer overflows; on `Closed` the agent runtime has shut
//!   down and the task exits.
//!
//! Why not a second WebSocket: the backend's session registry is keyed
//! by `DeviceId` alone (see `minos_backend::session::registry`). A second
//! WS handshake from the same device id revokes the prior socket and
//! emits `Close(1000, "session_superseded")`. Two parallel WS clients in
//! one daemon would supersede each other in a tight loop.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use minos_agent_runtime::RawIngest;
use minos_protocol::Envelope;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

pub(crate) fn spawn(
    out_tx: mpsc::Sender<Envelope>,
    rx: broadcast::Receiver<RawIngest>,
) -> JoinHandle<()> {
    tokio::spawn(run(out_tx, rx))
}

async fn run(out_tx: mpsc::Sender<Envelope>, mut rx: broadcast::Receiver<RawIngest>) {
    let mut seqs: HashMap<String, u64> = HashMap::new();

    loop {
        match rx.recv().await {
            Ok(raw) => {
                let seq = next_seq(&mut seqs, &raw.thread_id);
                let envelope = Envelope::Ingest {
                    version: 1,
                    agent: raw.agent,
                    thread_id: raw.thread_id.clone(),
                    seq,
                    payload: raw.payload,
                    ts_ms: current_unix_ms(),
                };
                if let Err(e) = out_tx.send(envelope).await {
                    tracing::warn!(
                        target: "minos_daemon::agent_ingest",
                        error = %e,
                        thread_id = %raw.thread_id,
                        "relay outbound queue closed; agent ingest forwarder exiting",
                    );
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    target: "minos_daemon::agent_ingest",
                    dropped = n,
                    "agent ingest receiver lagged; some events skipped",
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!(
                    target: "minos_daemon::agent_ingest",
                    "agent ingest stream closed; forwarder exiting",
                );
                return;
            }
        }
    }
}

fn next_seq(seqs: &mut HashMap<String, u64>, thread_id: &str) -> u64 {
    let entry = seqs.entry(thread_id.to_string()).or_insert(0);
    *entry += 1;
    *entry
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use serde_json::json;

    fn raw(thread_id: &str, payload: serde_json::Value) -> RawIngest {
        RawIngest {
            agent: AgentName::Codex,
            thread_id: thread_id.to_string(),
            payload,
            ts_ms: 0,
        }
    }

    #[tokio::test]
    async fn forwards_broadcast_as_ingest_envelopes_with_monotonic_seq() {
        let (broadcast_tx, broadcast_rx) = broadcast::channel::<RawIngest>(8);
        let (out_tx, mut out_rx) = mpsc::channel::<Envelope>(8);

        let task = spawn(out_tx, broadcast_rx);

        broadcast_tx
            .send(raw("thr-1", json!({"method": "a"})))
            .unwrap();
        broadcast_tx
            .send(raw("thr-1", json!({"method": "b"})))
            .unwrap();
        broadcast_tx
            .send(raw("thr-2", json!({"method": "c"})))
            .unwrap();
        broadcast_tx
            .send(raw("thr-1", json!({"method": "d"})))
            .unwrap();

        let mut got = Vec::new();
        for _ in 0..4 {
            let env = tokio::time::timeout(std::time::Duration::from_secs(1), out_rx.recv())
                .await
                .expect("must not time out")
                .expect("envelope received");
            got.push(env);
        }

        let mut by_thread: HashMap<String, Vec<u64>> = HashMap::new();
        for env in &got {
            match env {
                Envelope::Ingest {
                    version,
                    thread_id,
                    seq,
                    ..
                } => {
                    assert_eq!(*version, 1);
                    by_thread.entry(thread_id.clone()).or_default().push(*seq);
                }
                other => panic!("expected Envelope::Ingest, got {other:?}"),
            }
        }
        assert_eq!(by_thread.get("thr-1").unwrap(), &vec![1, 2, 3]);
        assert_eq!(by_thread.get("thr-2").unwrap(), &vec![1]);

        drop(broadcast_tx);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("forwarder exits when broadcast closes");
    }

    #[tokio::test]
    async fn exits_when_outbound_queue_closes() {
        let (broadcast_tx, broadcast_rx) = broadcast::channel::<RawIngest>(8);
        let (out_tx, out_rx) = mpsc::channel::<Envelope>(1);

        let task = spawn(out_tx, broadcast_rx);
        drop(out_rx);

        broadcast_tx
            .send(raw("thr-1", json!({"method": "x"})))
            .unwrap();

        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("forwarder exits on closed outbound queue");
    }
}
