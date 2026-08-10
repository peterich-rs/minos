//! Prepare live ingest for local commit.
//!
//! Seq is **not** allocated here. Assigning seq before `EventWriter` commit can
//! leave permanent holes when the write fails (or when the parent session row is
//! still missing). The writer assigns monotonic `last_seq + 1` inside the SQLite
//! transaction; only then may live upload / Desktop broadcast use that seq.
//!
//! Parent-row readiness: events whose session row is not yet inserted are
//! buffered (not dropped). Callers drain via [`IngestCoalescer::drain_ready`]
//! after inserts or on a short poll.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use anyhow::Result;
use minos_agent_runtime::RawIngest;
use minos_ui_protocol::UiEventMessage;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

use crate::store::event_writer::ProjectionTranslator;
use crate::store::LocalStore;

/// Max raw frames held while waiting for a session parent row (per session).
const MAX_DEFERRED_PER_SESSION: usize = 2048;
/// Max prepared frames waiting for a successful local write retry.
const MAX_WRITE_RETRY: usize = 2048;

/// Queue pressure exceeded; caller must fail the session (no silent drop).
#[derive(Debug, Clone)]
pub struct IngestQueueFull {
    pub session_id: String,
    pub queue: &'static str,
}

impl std::fmt::Display for IngestQueueFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ingest queue full for session {}: {}",
            self.session_id, self.queue
        )
    }
}

impl std::error::Error for IngestQueueFull {}

/// Projected ingest ready for `EventWriter` (seq still unassigned).
#[derive(Debug, Clone)]
pub struct PreparedIngest {
    pub ingest: RawIngest,
    pub projection: Vec<UiEventMessage>,
    pub conversation_id: Option<String>,
}

#[derive(Clone)]
pub struct IngestCoalescer {
    inner: Arc<Mutex<CoalescerState>>,
    store: Arc<LocalStore>,
}

#[derive(Default)]
struct CoalescerState {
    projector: ProjectionTranslator,
    /// session_id → frames waiting for parent `sessions` row.
    deferred: HashMap<String, VecDeque<RawIngest>>,
    /// Prepared frames whose write failed; retry without re-projecting.
    write_retry: VecDeque<PreparedIngest>,
}

impl IngestCoalescer {
    #[must_use]
    pub fn new(store: Arc<LocalStore>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CoalescerState::default())),
            store,
        }
    }

    /// Project `ingest` if the parent session row exists; otherwise buffer it.
    ///
    /// Returns `Err(IngestQueueFull)` when the parent-wait buffer is full —
    /// never silently drops frames.
    pub async fn admit(&self, ingest: RawIngest) -> Result<Option<PreparedIngest>> {
        let session_id = ingest.session_id.clone();
        if !self.parent_ready_quick(&session_id).await? {
            // Brief wait covers the common race (start_agent insert vs first frame).
            if !self.wait_for_thread_parent_short(&session_id).await? {
                self.defer(ingest).await?;
                return Ok(None);
            }
        }
        Ok(Some(self.prepare_now(ingest).await?))
    }

    /// Re-check deferred parents and pop write-retry queue.
    pub async fn drain_ready(&self) -> Result<Vec<PreparedIngest>> {
        let mut out = Vec::new();

        // Write retries first (already projected; preserve order).
        {
            let mut inner = self.inner.lock().await;
            while let Some(prepared) = inner.write_retry.pop_front() {
                out.push(prepared);
            }
        }

        let session_ids: Vec<String> = {
            let inner = self.inner.lock().await;
            inner.deferred.keys().cloned().collect()
        };
        for session_id in session_ids {
            if !self.parent_ready_quick(&session_id).await? {
                continue;
            }
            let pending = {
                let mut inner = self.inner.lock().await;
                inner.deferred.remove(&session_id).unwrap_or_default()
            };
            for ingest in pending {
                out.push(self.prepare_now(ingest).await?);
            }
        }
        Ok(out)
    }

    /// Notify that `session_id` parent row was inserted (flush that session).
    pub async fn on_session_parent_ready(&self, session_id: &str) -> Result<Vec<PreparedIngest>> {
        if !self.parent_ready_quick(session_id).await? {
            return Ok(Vec::new());
        }
        let pending = {
            let mut inner = self.inner.lock().await;
            inner.deferred.remove(session_id).unwrap_or_default()
        };
        let mut out = Vec::with_capacity(pending.len());
        for ingest in pending {
            out.push(self.prepare_now(ingest).await?);
        }
        Ok(out)
    }

    /// Re-queue a prepared frame after local write failure (seq never committed).
    ///
    /// Returns `Err(IngestQueueFull)` when the write-retry queue is full —
    /// never silently drops frames.
    pub async fn requeue_write_failure(
        &self,
        prepared: PreparedIngest,
    ) -> Result<(), IngestQueueFull> {
        let mut inner = self.inner.lock().await;
        if inner.write_retry.len() >= MAX_WRITE_RETRY {
            tracing::error!(
                target: "minos_daemon::ingest_coalescer",
                session_id = %prepared.ingest.session_id,
                cap = MAX_WRITE_RETRY,
                "write-retry queue full; refusing silent drop (session must fail)"
            );
            return Err(IngestQueueFull {
                session_id: prepared.ingest.session_id.clone(),
                queue: "write_retry",
            });
        }
        inner.write_retry.push_back(prepared);
        Ok(())
    }

    /// Restore a failed drain at the queue head without changing provider order.
    ///
    /// If capacity is exceeded, returns `IngestQueueFull` for the first
    /// overflowing session (caller fails that session).
    pub async fn restore_write_queue_front(
        &self,
        prepared: Vec<PreparedIngest>,
    ) -> Result<(), IngestQueueFull> {
        let mut inner = self.inner.lock().await;
        for item in prepared.into_iter().rev() {
            if inner.write_retry.len() >= MAX_WRITE_RETRY {
                tracing::error!(
                    target: "minos_daemon::ingest_coalescer",
                    session_id = %item.ingest.session_id,
                    cap = MAX_WRITE_RETRY,
                    "write-retry restore full; refusing silent drop"
                );
                return Err(IngestQueueFull {
                    session_id: item.ingest.session_id.clone(),
                    queue: "write_retry",
                });
            }
            inner.write_retry.push_front(item);
        }
        Ok(())
    }

    async fn defer(&self, ingest: RawIngest) -> Result<(), IngestQueueFull> {
        let session_id = ingest.session_id.clone();
        let mut inner = self.inner.lock().await;
        let q = inner.deferred.entry(session_id.clone()).or_default();
        if q.len() >= MAX_DEFERRED_PER_SESSION {
            tracing::error!(
                target: "minos_daemon::ingest_coalescer",
                session_id = %session_id,
                cap = MAX_DEFERRED_PER_SESSION,
                "deferred parent-wait queue full; refusing silent drop (session must fail)"
            );
            return Err(IngestQueueFull {
                session_id,
                queue: "deferred_parent_wait",
            });
        }
        tracing::warn!(
            target: "minos_daemon::ingest_coalescer",
            session_id = %session_id,
            queued = q.len() + 1,
            "session parent row missing; buffering ingest until insert",
        );
        q.push_back(ingest);
        Ok(())
    }

    async fn prepare_now(&self, ingest: RawIngest) -> Result<PreparedIngest> {
        let conversation_id = self
            .store
            .get_session(&ingest.session_id)
            .await?
            .map(|row| row.conversation_id)
            .filter(|id| !id.is_empty());
        let mut inner = self.inner.lock().await;
        let projection = inner.projector.translate(&ingest, ingest.inline_bytes());
        Ok(PreparedIngest {
            ingest,
            projection,
            conversation_id,
        })
    }

    async fn parent_ready_quick(&self, session_id: &str) -> Result<bool> {
        Ok(self.store.get_session(session_id).await?.is_some())
    }

    /// ~200ms total — enough for normal insert races; longer waits go to buffer.
    async fn wait_for_thread_parent_short(&self, session_id: &str) -> Result<bool> {
        let started = Instant::now();
        for delay_ms in [0_u64, 5, 10, 20, 40, 50, 75] {
            if delay_ms > 0 {
                sleep(Duration::from_millis(delay_ms)).await;
            }
            if self.store.get_session(session_id).await?.is_some() {
                return Ok(true);
            }
        }
        tracing::debug!(
            target: "minos_daemon::ingest_coalescer",
            session_id = %session_id,
            waited_ms = started.elapsed().as_millis() as u64,
            "parent still missing after short wait",
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use serde_json::json;
    use tempfile::tempdir;

    async fn seed_session(store: &LocalStore, session_id: &str) {
        store.upsert_workspace("/w", 1).await.unwrap();
        store
            .create_project("p", "Project", "project", Some("/w"), 0)
            .await
            .unwrap();
        store
            .create_conversation("c", "p", "Conversation", 0)
            .await
            .unwrap();
        store
            .insert_session_in_conversation(
                session_id,
                "c",
                "/w",
                "codex",
                Some("local-rt-codex"),
                None,
                None,
                "idle",
                1,
                true,
            )
            .await
            .unwrap();
    }

    fn sample_ingest(session_id: &str) -> RawIngest {
        RawIngest::from_json(
            AgentName::Codex,
            session_id.to_owned(),
            json!({"method": "turn/started", "params": {}}),
            1,
        )
    }

    #[tokio::test]
    async fn admit_defers_when_parent_missing_then_flushes_on_ready() {
        let dir = tempdir().unwrap();
        let store = Arc::new(
            LocalStore::open(&dir.path().join("t.sqlite"))
                .await
                .unwrap(),
        );
        let coalescer = IngestCoalescer::new(store.clone());
        let session_id = "sess-defer-1";

        let ready = coalescer
            .admit(sample_ingest(session_id))
            .await
            .expect("admit");
        assert!(ready.is_none(), "must buffer without parent");

        seed_session(&store, session_id).await;

        let drained = coalescer
            .on_session_parent_ready(session_id)
            .await
            .expect("flush");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].ingest.session_id, session_id);
        // No seq allocated at prepare time.
    }

    #[tokio::test]
    async fn admit_ready_when_parent_exists() {
        let dir = tempdir().unwrap();
        let store = Arc::new(
            LocalStore::open(&dir.path().join("t.sqlite"))
                .await
                .unwrap(),
        );
        let session_id = "sess-ready";
        seed_session(&store, session_id).await;
        let coalescer = IngestCoalescer::new(store);

        let prepared = coalescer
            .admit(sample_ingest(session_id))
            .await
            .expect("admit")
            .expect("should be ready");
        assert_eq!(prepared.ingest.session_id, session_id);
        assert_eq!(prepared.conversation_id.as_deref(), Some("c"));
    }

    #[tokio::test]
    async fn write_failure_requeue_drains_without_new_projection() {
        let dir = tempdir().unwrap();
        let store = Arc::new(
            LocalStore::open(&dir.path().join("t.sqlite"))
                .await
                .unwrap(),
        );
        let session_id = "sess-retry";
        seed_session(&store, session_id).await;
        let coalescer = IngestCoalescer::new(store);

        let prepared = coalescer
            .admit(sample_ingest(session_id))
            .await
            .unwrap()
            .unwrap();
        coalescer
            .requeue_write_failure(prepared.clone())
            .await
            .unwrap();
        let drained = coalescer.drain_ready().await.unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].ingest.session_id, session_id);
    }

    #[tokio::test]
    async fn failed_drain_restores_the_entire_batch_ahead_of_newer_frames() {
        let dir = tempdir().unwrap();
        let store = Arc::new(
            LocalStore::open(&dir.path().join("t.sqlite"))
                .await
                .unwrap(),
        );
        let session_id = "sess-retry-order";
        seed_session(&store, session_id).await;
        let coalescer = IngestCoalescer::new(store);

        let mut first = sample_ingest(session_id);
        first.ts_ms = 1;
        let mut second = sample_ingest(session_id);
        second.ts_ms = 2;
        let mut newer = sample_ingest(session_id);
        newer.ts_ms = 3;
        let first = coalescer.admit(first).await.unwrap().unwrap();
        let second = coalescer.admit(second).await.unwrap().unwrap();
        let newer = coalescer.admit(newer).await.unwrap().unwrap();

        coalescer
            .restore_write_queue_front(vec![first, second])
            .await
            .unwrap();
        coalescer.requeue_write_failure(newer).await.unwrap();

        let drained = coalescer.drain_ready().await.unwrap();
        assert_eq!(
            drained
                .iter()
                .map(|prepared| prepared.ingest.ts_ms)
                .collect::<Vec<_>>(),
            vec![1, 2, 3],
        );
    }
}
