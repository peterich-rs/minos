// `seq` and `ts_ms` are stored as i64 in SQLite; the Rust-side semantics use
// u64 (sequence numbers are always positive and ts_ms is positive epoch).
// Permit the bind-site casts to keep the SQL surface readable.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use crate::ingest_chunk::IngestChunk;
use crate::store::LocalStore;
use anyhow::Result;
use minos_agent_runtime::{RawBody, RawIngest, INLINE_RAW_BODY_THRESHOLD};
use minos_domain::AgentName;
use minos_ui_protocol::{
    translate_claude, translate_codex, translate_gemini, translate_opencode, ClaudeTranslatorState,
    CodexTranslatorState, GeminiTranslatorState, OpencodeTranslatorState, UiEventMessage,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSource {
    Live,
    JsonlRecovery,
}

#[derive(Clone)]
pub struct EventWriter {
    tx: mpsc::Sender<WriteJob>,
}

#[derive(Debug, Clone)]
pub struct CommittedIngest {
    pub seq: u64,
    pub projection: Vec<UiEventMessage>,
}

#[derive(Debug)]
struct WriteJob {
    ingest: RawIngest,
    seq: Option<u64>,
    projection: Option<Vec<UiEventMessage>>,
    source: EventSource,
    ack: tokio::sync::oneshot::Sender<Result<CommittedIngest>>,
}

impl EventWriter {
    pub fn spawn(store: Arc<LocalStore>) -> Self {
        let (tx, rx) = mpsc::channel::<WriteJob>(1024);
        tokio::spawn(writer_loop(store, rx));
        Self { tx }
    }

    pub async fn write_live(&self, ingest: RawIngest) -> Result<CommittedIngest> {
        self.write_internal(ingest, None, None, EventSource::Live)
            .await
    }

    pub async fn write_chunk(&self, chunk: IngestChunk) -> Result<CommittedIngest> {
        self.write_internal(
            chunk.ingest,
            Some(chunk.seq),
            Some(chunk.projection),
            EventSource::Live,
        )
        .await
    }

    pub async fn write_recovery(&self, ingest: RawIngest) -> Result<CommittedIngest> {
        self.write_internal(ingest, None, None, EventSource::JsonlRecovery)
            .await
    }

    async fn write_internal(
        &self,
        ingest: RawIngest,
        seq: Option<u64>,
        projection: Option<Vec<UiEventMessage>>,
        source: EventSource,
    ) -> Result<CommittedIngest> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(WriteJob {
                ingest,
                seq,
                projection,
                source,
                ack: tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("event writer task gone"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("event writer dropped"))?
    }
}

async fn writer_loop(store: Arc<LocalStore>, mut rx: mpsc::Receiver<WriteJob>) {
    use tokio::time::{Duration, Instant};
    const BATCH_MAX: usize = 100;
    const BATCH_WINDOW: Duration = Duration::from_millis(5);

    let mut buf: Vec<WriteJob> = Vec::with_capacity(BATCH_MAX);
    let mut projector = ProjectionTranslator::default();
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
        process_batch(&store, &mut projector, std::mem::take(&mut buf)).await;
    }
}

async fn process_batch(
    store: &LocalStore,
    projector: &mut ProjectionTranslator,
    jobs: Vec<WriteJob>,
) {
    if jobs.is_empty() {
        return;
    }
    let mut checked_threads = HashSet::new();
    let mut parent_errors = HashMap::new();
    for job in &jobs {
        if checked_threads.insert(job.ingest.thread_id.clone()) {
            if let Err(e) = wait_for_thread_parent(store, &job.ingest.thread_id).await {
                parent_errors.insert(job.ingest.thread_id.clone(), e.to_string());
            }
        }
    }

    let mut ready_jobs = Vec::with_capacity(jobs.len());
    for job in jobs {
        if let Some(error) = parent_errors.get(&job.ingest.thread_id) {
            let _ = job.ack.send(Err(anyhow::anyhow!(error.clone())));
        } else {
            ready_jobs.push(job);
        }
    }
    if ready_jobs.is_empty() {
        return;
    }

    let mut tx = match store.pool().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            let err = std::sync::Arc::new(e);
            for j in ready_jobs {
                let _ = j.ack.send(Err(anyhow::anyhow!("begin tx: {err}")));
            }
            return;
        }
    };
    let mut results: Vec<Result<CommittedIngest>> = Vec::with_capacity(ready_jobs.len());
    for job in &ready_jobs {
        let seq = if let Some(seq) = job.seq {
            seq
        } else {
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
            let Some(prev) = prev else {
                results.push(Err(anyhow::anyhow!(
                    "thread parent row disappeared before event write: {}",
                    job.ingest.thread_id
                )));
                continue;
            };
            (prev + 1) as u64
        };
        let stored_body = match prepare_raw_body(store, &job.ingest).await {
            Ok(body) => body,
            Err(e) => {
                results.push(Err(e));
                continue;
            }
        };
        let projection = job.projection.clone().unwrap_or_else(|| {
            projector.translate(&job.ingest, stored_body.inline_for_projection.as_deref())
        });
        let projection_json = match serde_json::to_vec(&projection) {
            Ok(bytes) => bytes,
            Err(e) => {
                results.push(Err(e.into()));
                continue;
            }
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO events( \
                thread_id, seq, body_kind, body_inline, artifact_id, artifact_size_bytes, \
                artifact_sha256, artifact_media_type, projection_json, ts_ms, source \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&job.ingest.thread_id)
        .bind(seq as i64)
        .bind(stored_body.body_kind)
        .bind(stored_body.body_inline)
        .bind(stored_body.artifact_id)
        .bind(stored_body.artifact_size_bytes)
        .bind(stored_body.artifact_sha256)
        .bind(stored_body.artifact_media_type)
        .bind(projection_json)
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
        let provider_session_id = provider_session_id_from_ingest(&job.ingest);
        let update_result = if let Some(provider_session_id) = provider_session_id.as_deref() {
            sqlx::query(
                "UPDATE threads SET \
                    last_seq = CASE WHEN last_seq < ? THEN ? ELSE last_seq END, \
                    last_activity_at = ?, codex_session_id = ? WHERE thread_id = ?",
            )
            .bind(seq as i64)
            .bind(seq as i64)
            .bind(job.ingest.ts_ms)
            .bind(provider_session_id)
            .bind(&job.ingest.thread_id)
            .execute(&mut *tx)
            .await
        } else {
            sqlx::query(
                "UPDATE threads SET \
                    last_seq = CASE WHEN last_seq < ? THEN ? ELSE last_seq END, \
                    last_activity_at = ? WHERE thread_id = ?",
            )
            .bind(seq as i64)
            .bind(seq as i64)
            .bind(job.ingest.ts_ms)
            .bind(&job.ingest.thread_id)
            .execute(&mut *tx)
            .await
        };
        if let Err(e) = update_result {
            results.push(Err(e.into()));
            continue;
        }
        results.push(Ok(CommittedIngest {
            seq,
            projection: projection.clone(),
        }));
    }
    if let Err(e) = tx.commit().await {
        for (job, _) in ready_jobs.into_iter().zip(results) {
            let _ = job.ack.send(Err(anyhow::anyhow!("commit: {e}")));
        }
        return;
    }
    for (job, r) in ready_jobs.into_iter().zip(results) {
        let _ = job.ack.send(r);
    }
}

struct StoredRawBody {
    body_kind: &'static str,
    body_inline: Option<Vec<u8>>,
    artifact_id: Option<String>,
    artifact_size_bytes: Option<i64>,
    artifact_sha256: Option<String>,
    artifact_media_type: Option<String>,
    inline_for_projection: Option<Vec<u8>>,
}

async fn prepare_raw_body(store: &LocalStore, ingest: &RawIngest) -> Result<StoredRawBody> {
    match &ingest.body {
        RawBody::InlineBytes { bytes, media_type } if bytes.len() >= INLINE_RAW_BODY_THRESHOLD => {
            let artifact = store
                .artifacts()
                .write_bytes(&ingest.thread_id, bytes, media_type)
                .await?;
            sqlx::query(
                "INSERT OR IGNORE INTO artifacts( \
                    thread_id, artifact_id, size_bytes, sha256, media_type, created_at \
                 ) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&artifact.thread_id)
            .bind(&artifact.artifact_id)
            .bind(artifact.size_bytes as i64)
            .bind(&artifact.sha256)
            .bind(&artifact.media_type)
            .bind(ingest.ts_ms)
            .execute(store.pool())
            .await?;
            Ok(StoredRawBody {
                body_kind: "artifact",
                body_inline: None,
                artifact_id: Some(artifact.artifact_id),
                artifact_size_bytes: Some(artifact.size_bytes as i64),
                artifact_sha256: Some(artifact.sha256),
                artifact_media_type: Some(artifact.media_type),
                inline_for_projection: Some(bytes.clone()),
            })
        }
        RawBody::InlineBytes { bytes, .. } => Ok(StoredRawBody {
            body_kind: "inline",
            body_inline: Some(bytes.clone()),
            artifact_id: None,
            artifact_size_bytes: None,
            artifact_sha256: None,
            artifact_media_type: None,
            inline_for_projection: Some(bytes.clone()),
        }),
        RawBody::Artifact { artifact } => Ok(StoredRawBody {
            body_kind: "artifact",
            body_inline: None,
            artifact_id: Some(artifact.artifact_id.clone()),
            artifact_size_bytes: Some(artifact.size_bytes as i64),
            artifact_sha256: Some(artifact.sha256.clone()),
            artifact_media_type: Some(artifact.media_type.clone()),
            inline_for_projection: None,
        }),
    }
}

#[derive(Default)]
pub(crate) struct ProjectionTranslator {
    codex: HashMap<String, CodexTranslatorState>,
    claude: HashMap<String, ClaudeTranslatorState>,
    gemini: HashMap<String, GeminiTranslatorState>,
    opencode: HashMap<String, OpencodeTranslatorState>,
}

impl ProjectionTranslator {
    pub(crate) fn translate(
        &mut self,
        ingest: &RawIngest,
        raw_bytes: Option<&[u8]>,
    ) -> Vec<UiEventMessage> {
        let Some(raw_bytes) = raw_bytes else {
            return vec![raw_projection_fallback(ingest)];
        };
        let payload = match serde_json::from_slice::<Value>(raw_bytes) {
            Ok(payload) => payload,
            Err(error) => {
                return vec![UiEventMessage::Error {
                    code: "raw_projection_failed".into(),
                    message: error.to_string(),
                    message_id: None,
                }];
            }
        };
        let translated = match ingest.agent {
            AgentName::Codex => {
                let state = self
                    .codex
                    .entry(ingest.thread_id.clone())
                    .or_insert_with(|| CodexTranslatorState::new(ingest.thread_id.clone()));
                translate_codex(state, &payload)
            }
            AgentName::Claude => {
                let state = self
                    .claude
                    .entry(ingest.thread_id.clone())
                    .or_insert_with(|| ClaudeTranslatorState::new(ingest.thread_id.clone()));
                translate_claude(state, &payload)
            }
            AgentName::Gemini => {
                let state = self
                    .gemini
                    .entry(ingest.thread_id.clone())
                    .or_insert_with(|| GeminiTranslatorState::new(ingest.thread_id.clone()));
                translate_gemini(state, &payload)
            }
            AgentName::Opencode => {
                let state = self
                    .opencode
                    .entry(ingest.thread_id.clone())
                    .or_insert_with(|| OpencodeTranslatorState::new(ingest.thread_id.clone()));
                translate_opencode(state, &payload)
            }
        };
        match translated {
            Ok(events) => events,
            Err(error) => vec![UiEventMessage::Error {
                code: "raw_projection_failed".into(),
                message: error.to_string(),
                message_id: None,
            }],
        }
    }
}

fn raw_projection_fallback(ingest: &RawIngest) -> UiEventMessage {
    UiEventMessage::Raw {
        kind: ingest
            .event_type
            .clone()
            .unwrap_or_else(|| "agent_event".to_string()),
        payload_json: serde_json::to_string(&ingest.body).unwrap_or_default(),
    }
}

async fn wait_for_thread_parent(store: &LocalStore, thread_id: &str) -> Result<()> {
    let started = Instant::now();
    for delay_ms in [0, 10, 25, 50, 100, 200, 400, 400, 400, 400, 400] {
        if delay_ms > 0 {
            sleep(Duration::from_millis(delay_ms)).await;
        }
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM threads WHERE thread_id = ?")
            .bind(thread_id)
            .fetch_optional(store.pool())
            .await?;
        if exists.is_some() {
            return Ok(());
        }
    }
    Err(anyhow::anyhow!(
        "thread parent row missing for {thread_id} after {:?}",
        started.elapsed()
    ))
}

pub(crate) fn provider_session_id_from_ingest(ingest: &RawIngest) -> Option<String> {
    ingest.provider_session_id.clone()
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

    #[test]
    fn provider_session_id_from_ingest_supports_non_codex_agents() {
        let claude = RawIngest::from_json(
            AgentName::Claude,
            "thr-claude".into(),
            serde_json::json!({"type":"result","session_id":"claude-session"}),
            1,
        );
        assert_eq!(
            provider_session_id_from_ingest(&claude).as_deref(),
            Some("claude-session")
        );

        let gemini = RawIngest::from_json(
            AgentName::Gemini,
            "thr-gemini".into(),
            serde_json::json!({
                "kind":"acp_notification",
                "params":{"sessionId":"gemini-session"}
            }),
            1,
        );
        assert_eq!(
            provider_session_id_from_ingest(&gemini).as_deref(),
            Some("gemini-session")
        );

        let opencode = RawIngest::from_json(
            AgentName::Opencode,
            "thr-opencode".into(),
            serde_json::json!({
                "type":"message.part.updated",
                "properties":{"part":{"sessionID":"opencode-session"}}
            }),
            1,
        );
        assert_eq!(
            provider_session_id_from_ingest(&opencode).as_deref(),
            Some("opencode-session")
        );
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
        let writer = EventWriter::spawn(store.clone());

        let start = Instant::now();
        let mut handles = Vec::new();
        for i in 0..50 {
            let w = writer.clone();
            handles.push(tokio::spawn(async move {
                w.write_live(RawIngest::from_json(
                    minos_agent_runtime::AgentKind::Codex,
                    "thr-B".into(),
                    serde_json::json!({"i": i}),
                    i as i64,
                ))
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

        let rows = store.read_events("thr-B", 1, 50).await.unwrap();
        assert_eq!(rows.len(), 50);
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
        let writer = EventWriter::spawn(store.clone());

        for i in 0..5 {
            let ingest = RawIngest::from_json(
                minos_agent_runtime::AgentKind::Codex,
                "thr-A".into(),
                serde_json::json!({"i": i}),
                i,
            );
            let committed = writer.write_live(ingest).await.unwrap();
            assert_eq!(committed.seq, (i + 1) as u64);
        }

        let rows = store.read_events("thr-A", 1, 5).await.unwrap();
        assert_eq!(rows.len(), 5);
    }

    #[tokio::test]
    async fn write_live_waits_for_delayed_thread_parent_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            LocalStore::open(&tmp.path().join("t.sqlite"))
                .await
                .unwrap(),
        );
        let writer = EventWriter::spawn(store.clone());

        let pending = {
            let writer = writer.clone();
            tokio::spawn(async move {
                writer
                    .write_live(RawIngest::from_json(
                        minos_agent_runtime::AgentKind::Codex,
                        "thr-delayed".into(),
                        serde_json::json!({"kind": "delayed-parent"}),
                        42,
                    ))
                    .await
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;
        seed_thread(&store, "thr-delayed").await;

        let committed = tokio::time::timeout(Duration::from_secs(3), pending)
            .await
            .expect("write_live should finish after the parent row appears")
            .expect("writer task should not panic")
            .expect("delayed parent row should prevent FK failure");
        assert_eq!(committed.seq, 1);

        let rows = store.read_events("thr-delayed", 1, 1).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seq, 1);

        let rows = store.read_events("thr-delayed", 1, 1).await.unwrap();
        assert_eq!(rows.len(), 1);
    }
}
