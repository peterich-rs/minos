use crate::store::LocalStore;
use anyhow::Result;
use minos_agent_runtime::RawIngest;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSource {
    Live,
    JsonlRecovery,
}

#[derive(Clone)]
pub struct EventWriter {
    tx: mpsc::Sender<WriteJob>,
}

#[derive(Debug)]
struct WriteJob {
    ingest: RawIngest,
    source: EventSource,
    ack: tokio::sync::oneshot::Sender<Result<u64>>,
}

impl EventWriter {
    pub fn spawn(
        store: Arc<LocalStore>,
        relay_out: mpsc::Sender<minos_protocol::Envelope>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<WriteJob>(1024);
        tokio::spawn(writer_loop(store, relay_out, rx));
        Self { tx }
    }

    pub async fn write_live(&self, ingest: RawIngest) -> Result<u64> {
        self.write_internal(ingest, EventSource::Live).await
    }

    pub async fn write_recovery(&self, ingest: RawIngest) -> Result<u64> {
        self.write_internal(ingest, EventSource::JsonlRecovery).await
    }

    async fn write_internal(&self, ingest: RawIngest, source: EventSource) -> Result<u64> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(WriteJob {
                ingest,
                source,
                ack: tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("event writer task gone"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("event writer dropped"))?
    }
}

async fn writer_loop(
    store: Arc<LocalStore>,
    relay_out: mpsc::Sender<minos_protocol::Envelope>,
    mut rx: mpsc::Receiver<WriteJob>,
) {
    while let Some(job) = rx.recv().await {
        let res = process_one(&store, &relay_out, job.ingest.clone(), job.source).await;
        let _ = job.ack.send(res);
    }
}

async fn process_one(
    store: &LocalStore,
    relay_out: &mpsc::Sender<minos_protocol::Envelope>,
    ingest: RawIngest,
    source: EventSource,
) -> Result<u64> {
    let mut tx = store.pool().begin().await?;
    let prev: Option<i64> = sqlx::query_scalar("SELECT last_seq FROM threads WHERE thread_id = ?")
        .bind(&ingest.thread_id)
        .fetch_optional(&mut *tx)
        .await?;
    let seq = (prev.unwrap_or(0) + 1) as u64;
    let payload_bytes = serde_json::to_vec(&ingest.payload)?;
    sqlx::query(
        "INSERT INTO events(thread_id, seq, payload, ts_ms, source) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&ingest.thread_id)
    .bind(seq as i64)
    .bind(&payload_bytes)
    .bind(ingest.ts_ms)
    .bind(match source {
        EventSource::Live => "live",
        EventSource::JsonlRecovery => "jsonl_recovery",
    })
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE threads SET last_seq = ?, last_activity_at = ? WHERE thread_id = ?")
        .bind(seq as i64)
        .bind(ingest.ts_ms)
        .bind(&ingest.thread_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let env = minos_protocol::Envelope::Ingest {
        version: 1,
        agent: ingest.agent,
        thread_id: ingest.thread_id.clone(),
        seq,
        payload: ingest.payload.clone(),
        ts_ms: ingest.ts_ms,
    };
    let _ = relay_out.send(env).await;
    Ok(seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_thread(store: &LocalStore, tid: &str) {
        sqlx::query(
            "INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/tmp/ws', 0, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, '/tmp/ws', 'codex', 'idle', 0, 0, 0)",
        )
        .bind(tid)
        .execute(store.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn write_live_assigns_monotonic_seq() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            LocalStore::open(&tmp.path().join("t.sqlite"))
                .await
                .unwrap(),
        );
        seed_thread(&store, "thr-A").await;
        let (relay_tx, mut relay_rx) = mpsc::channel(16);
        let writer = EventWriter::spawn(store.clone(), relay_tx);

        for i in 0..5 {
            let ingest = RawIngest {
                agent: minos_agent_runtime::AgentKind::Codex,
                thread_id: "thr-A".into(),
                payload: serde_json::json!({"i": i}),
                ts_ms: i,
            };
            let seq = writer.write_live(ingest).await.unwrap();
            assert_eq!(seq, (i + 1) as u64);
        }

        for i in 0..5 {
            let env = relay_rx.recv().await.unwrap();
            match env {
                minos_protocol::Envelope::Ingest { seq, .. } => assert_eq!(seq, (i + 1) as u64),
                _ => panic!("unexpected envelope"),
            }
        }
    }
}
