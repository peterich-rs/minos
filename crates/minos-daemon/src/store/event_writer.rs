// `seq` and `ts_ms` are stored as i64 in SQLite; the Rust-side semantics use
// u64 (sequence numbers are always positive and ts_ms is positive epoch).
// Permit the bind-site casts to keep the SQL surface readable.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

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
        self.write_internal(ingest, EventSource::JsonlRecovery)
            .await
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
    use tokio::time::{Duration, Instant};
    const BATCH_MAX: usize = 100;
    const BATCH_WINDOW: Duration = Duration::from_millis(5);

    let mut buf: Vec<WriteJob> = Vec::with_capacity(BATCH_MAX);
    while let Some(first) = rx.recv().await {
        buf.push(first);
        let deadline = Instant::now() + BATCH_WINDOW;
        while buf.len() < BATCH_MAX {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(job)) => buf.push(job),
                Ok(None) | Err(_) => break,
            }
        }
        process_batch(&store, &relay_out, std::mem::take(&mut buf)).await;
    }
}

async fn process_batch(
    store: &LocalStore,
    relay_out: &mpsc::Sender<minos_protocol::Envelope>,
    jobs: Vec<WriteJob>,
) {
    if jobs.is_empty() {
        return;
    }
    let mut tx = match store.pool().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            let err = std::sync::Arc::new(e);
            for j in jobs {
                let _ = j.ack.send(Err(anyhow::anyhow!("begin tx: {err}")));
            }
            return;
        }
    };
    let mut results: Vec<Result<u64>> = Vec::with_capacity(jobs.len());
    let mut envs: Vec<minos_protocol::Envelope> = Vec::with_capacity(jobs.len());
    for job in &jobs {
        let prev: Option<i64> =
            match sqlx::query_scalar("SELECT last_seq FROM threads WHERE thread_id = ?")
                .bind(&job.ingest.thread_id)
                .fetch_optional(&mut *tx)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    results.push(Err(e.into()));
                    continue;
                }
            };
        let seq = (prev.unwrap_or(0) + 1) as u64;
        let payload = match serde_json::to_vec(&job.ingest.payload) {
            Ok(v) => v,
            Err(e) => {
                results.push(Err(e.into()));
                continue;
            }
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO events(thread_id, seq, payload, ts_ms, source) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&job.ingest.thread_id)
        .bind(seq as i64)
        .bind(&payload)
        .bind(job.ingest.ts_ms)
        .bind(match job.source {
            EventSource::Live => "live",
            EventSource::JsonlRecovery => "jsonl_recovery",
        })
        .execute(&mut *tx)
        .await
        {
            results.push(Err(e.into()));
            continue;
        }
        if let Err(e) =
            sqlx::query("UPDATE threads SET last_seq = ?, last_activity_at = ? WHERE thread_id = ?")
                .bind(seq as i64)
                .bind(job.ingest.ts_ms)
                .bind(&job.ingest.thread_id)
                .execute(&mut *tx)
                .await
        {
            results.push(Err(e.into()));
            continue;
        }
        results.push(Ok(seq));
        envs.push(minos_protocol::Envelope::Ingest {
            version: 1,
            agent: job.ingest.agent,
            thread_id: job.ingest.thread_id.clone(),
            seq,
            payload: job.ingest.payload.clone(),
            ts_ms: job.ingest.ts_ms,
        });
    }
    if let Err(e) = tx.commit().await {
        for (job, _) in jobs.into_iter().zip(results) {
            let _ = job.ack.send(Err(anyhow::anyhow!("commit: {e}")));
        }
        return;
    }
    for (job, r) in jobs.into_iter().zip(results) {
        let _ = job.ack.send(r);
    }
    for env in envs {
        let _ = relay_out.send(env).await;
    }
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
    async fn batches_within_5ms_window() {
        use std::time::Duration;
        use tokio::time::Instant;
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            LocalStore::open(&tmp.path().join("t.sqlite"))
                .await
                .unwrap(),
        );
        seed_thread(&store, "thr-B").await;
        let (relay_tx, mut relay_rx) = mpsc::channel(256);
        let writer = EventWriter::spawn(store.clone(), relay_tx);

        let start = Instant::now();
        let mut handles = Vec::new();
        for i in 0..50 {
            let w = writer.clone();
            handles.push(tokio::spawn(async move {
                w.write_live(RawIngest {
                    agent: minos_agent_runtime::AgentKind::Codex,
                    thread_id: "thr-B".into(),
                    payload: serde_json::json!({"i": i}),
                    ts_ms: i as i64,
                })
                .await
                .unwrap()
            }));
        }
        for h in handles {
            let _ = h.await;
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "50 events should commit fast: {elapsed:?}"
        );

        let mut got = 0;
        while tokio::time::timeout(Duration::from_millis(200), relay_rx.recv())
            .await
            .is_ok()
        {
            got += 1;
            if got == 50 {
                break;
            }
        }
        assert_eq!(got, 50);
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
