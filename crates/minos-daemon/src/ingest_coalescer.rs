use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use minos_agent_runtime::RawIngest;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

use crate::ingest_chunk::IngestChunk;
use crate::store::event_writer::ProjectionTranslator;
use crate::store::LocalStore;

#[derive(Clone)]
pub struct IngestCoalescer {
    inner: Arc<Mutex<CoalescerState>>,
    store: Arc<LocalStore>,
}

#[derive(Default)]
struct CoalescerState {
    next_seq_by_thread: HashMap<String, u64>,
    projector: ProjectionTranslator,
}

impl IngestCoalescer {
    #[must_use]
    pub fn new(store: Arc<LocalStore>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CoalescerState::default())),
            store,
        }
    }

    pub async fn coalesce(&self, ingest: RawIngest) -> Result<IngestChunk> {
        wait_for_thread_parent(&self.store, &ingest.thread_id).await?;
        let seq = if let Some(seq) = self.take_next_seq(&ingest.thread_id).await {
            seq
        } else {
            let thread = self
                .store
                .get_thread(&ingest.thread_id)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("thread parent row missing: {}", ingest.thread_id)
                })?;
            let mut inner = self.inner.lock().await;
            let next = inner
                .next_seq_by_thread
                .entry(ingest.thread_id.clone())
                .or_insert_with(|| thread.last_seq.max(0) as u64 + 1);
            let seq = *next;
            *next += 1;
            seq
        };
        let mut inner = self.inner.lock().await;
        let projection = inner.projector.translate(&ingest, ingest.inline_bytes());
        Ok(IngestChunk::new(ingest, seq, projection))
    }

    async fn take_next_seq(&self, thread_id: &str) -> Option<u64> {
        let mut inner = self.inner.lock().await;
        let next = inner.next_seq_by_thread.get_mut(thread_id)?;
        let seq = *next;
        *next += 1;
        Some(seq)
    }
}

async fn wait_for_thread_parent(store: &LocalStore, thread_id: &str) -> Result<()> {
    let started = Instant::now();
    for delay_ms in [0, 10, 25, 50, 100, 200, 400, 400, 400, 400, 400] {
        if delay_ms > 0 {
            sleep(Duration::from_millis(delay_ms)).await;
        }
        if store.get_thread(thread_id).await?.is_some() {
            return Ok(());
        }
    }
    Err(anyhow::anyhow!(
        "thread parent row missing for {thread_id} after {:?}",
        started.elapsed()
    ))
}
