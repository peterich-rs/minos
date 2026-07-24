use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use minos_domain::{DeviceId, RelayLinkState};
use minos_protocol::realtime::{
    ClientFrame, HostGapManifest, HostIngestLiveBatch, HostIngestPullResponse, PullPriority,
    PullReason, SeqRange, SessionGapManifest,
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::ingest_chunk::{wire_chunk_from_event_row, IngestChunk};
use crate::store::LocalStore;

const LIVE_QUEUE_DEPTH: usize = 256;
const LIVE_BATCH_MAX: usize = 50;
const LIVE_BATCH_WINDOW: Duration = Duration::from_millis(5);

#[derive(Clone)]
pub struct IngestSyncHandle {
    host_id: DeviceId,
    store: Arc<LocalStore>,
    control_out: mpsc::Sender<ClientFrame>,
    live_out: mpsc::Sender<ClientFrame>,
    backfill_out: mpsc::Sender<ClientFrame>,
    link_rx: watch::Receiver<RelayLinkState>,
    live_tx: mpsc::Sender<IngestChunk>,
}

impl IngestSyncHandle {
    #[must_use]
    pub fn spawn(
        host_id: DeviceId,
        store: Arc<LocalStore>,
        control_out: mpsc::Sender<ClientFrame>,
        live_out: mpsc::Sender<ClientFrame>,
        backfill_out: mpsc::Sender<ClientFrame>,
        link_rx: watch::Receiver<RelayLinkState>,
    ) -> Self {
        let (live_tx, live_rx) = mpsc::channel(LIVE_QUEUE_DEPTH);
        let handle = Self {
            host_id,
            store,
            control_out,
            live_out,
            backfill_out,
            link_rx,
            live_tx,
        };
        tokio::spawn(live_upload_loop(handle.clone(), live_rx));
        handle
    }

    pub async fn submit_live(&self, chunk: IngestChunk) {
        if !self.is_connected() {
            self.mark_dirty_chunk(&chunk).await;
            return;
        }
        match self.live_tx.try_send(chunk) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(chunk))
            | Err(mpsc::error::TrySendError::Closed(chunk)) => {
                self.mark_dirty_chunk(&chunk).await;
            }
        }
    }

    pub async fn mark_backend_acked(&self, session_id: &str, accepted_to_seq: u64) {
        if let Err(error) = mark_backend_acked(&self.store, session_id, accepted_to_seq).await {
            tracing::warn!(
                target: "minos_daemon::ingest_sync",
                error = %error,
                session_id,
                accepted_to_seq,
                "failed to persist backend ingest ack",
            );
        }
    }

    pub async fn send_manifest(&self) {
        match build_manifest(&self.store, self.host_id).await {
            Ok(Some(manifest)) => {
                if let Err(error) = self
                    .control_out
                    .try_send(ClientFrame::HostGapManifest { manifest })
                {
                    tracing::warn!(
                        target: "minos_daemon::ingest_sync",
                        error = %error,
                        "failed to enqueue host gap manifest",
                    );
                }
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                target: "minos_daemon::ingest_sync",
                error = %error,
                "failed to build host gap manifest",
            ),
        }
    }

    pub async fn handle_pull_range(
        &self,
        request_id: String,
        session_id: String,
        from_seq: u64,
        to_seq: u64,
        max_bytes: u64,
        priority: PullPriority,
        reason: PullReason,
    ) {
        if matches!(priority, PullPriority::IdleBackfill | PullPriority::Audit)
            && self.live_tx.capacity() < LIVE_QUEUE_DEPTH / 2
        {
            tracing::debug!(
                target: "minos_daemon::ingest_sync",
                request_id,
                session_id,
                ?priority,
                ?reason,
                "deferring low-priority pull because live upload queue is busy",
            );
            return;
        }
        match build_pull_response(
            &self.store,
            self.host_id,
            request_id,
            session_id,
            from_seq,
            to_seq,
            max_bytes,
        )
        .await
        {
            Ok(response) => {
                if let Err(error) = self
                    .backfill_out
                    .try_send(ClientFrame::HostIngestPullResponse { response })
                {
                    tracing::warn!(
                        target: "minos_daemon::ingest_sync",
                        error = %error,
                        "failed to enqueue pulled ingest chunk",
                    );
                }
            }
            Err(error) => tracing::warn!(
                target: "minos_daemon::ingest_sync",
                error = %error,
                "failed to build pulled ingest chunk",
            ),
        }
    }

    fn is_connected(&self) -> bool {
        *self.link_rx.borrow() == RelayLinkState::Connected
    }

    async fn mark_dirty_chunk(&self, chunk: &IngestChunk) {
        if let Err(error) = mark_dirty_chunk(&self.store, chunk).await {
            tracing::warn!(
                target: "minos_daemon::ingest_sync",
                error = %error,
                session_id = %chunk.session_id(),
                seq = chunk.seq,
                "failed to persist dirty ingest range",
            );
        }
    }
}

async fn live_upload_loop(handle: IngestSyncHandle, mut rx: mpsc::Receiver<IngestChunk>) {
    let mut buf = Vec::with_capacity(LIVE_BATCH_MAX);
    while let Some(first) = rx.recv().await {
        buf.push(first);
        let deadline = tokio::time::Instant::now() + LIVE_BATCH_WINDOW;
        while buf.len() < LIVE_BATCH_MAX {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(chunk)) => buf.push(chunk),
                Ok(None) | Err(_) => break,
            }
        }

        let chunks = std::mem::take(&mut buf);
        if !handle.is_connected() {
            for chunk in &chunks {
                handle.mark_dirty_chunk(chunk).await;
            }
            continue;
        }

        let batch = HostIngestLiveBatch {
            batch_id: Uuid::new_v4().to_string(),
            host_id: handle.host_id,
            chunks: chunks
                .iter()
                .map(|chunk| chunk.to_wire(handle.host_id))
                .collect(),
        };
        if let Err(error) = handle
            .live_out
            .try_send(ClientFrame::HostIngestLiveBatch { batch })
        {
            tracing::warn!(
                target: "minos_daemon::ingest_sync",
                error = %error,
                count = chunks.len(),
                "live ingest queue full; marking range dirty",
            );
            for chunk in &chunks {
                handle.mark_dirty_chunk(chunk).await;
            }
        }
    }
}

async fn mark_dirty_chunk(store: &LocalStore, chunk: &IngestChunk) -> Result<()> {
    sqlx::query(
        "INSERT INTO ingest_sync_state( \
            session_id, backend_acked_seq, dirty_from_seq, dirty_to_seq, \
            dirty_bytes, dirty_events, updated_at \
         ) VALUES (?, 0, ?, ?, ?, 1, ?) \
         ON CONFLICT(session_id) DO UPDATE SET \
            dirty_from_seq = CASE \
                WHEN dirty_from_seq IS NULL OR dirty_from_seq > excluded.dirty_from_seq \
                THEN excluded.dirty_from_seq ELSE dirty_from_seq END, \
            dirty_to_seq = CASE \
                WHEN dirty_to_seq IS NULL OR dirty_to_seq < excluded.dirty_to_seq \
                THEN excluded.dirty_to_seq ELSE dirty_to_seq END, \
            dirty_bytes = dirty_bytes + excluded.dirty_bytes, \
            dirty_events = dirty_events + 1, \
            updated_at = excluded.updated_at",
    )
    .bind(&chunk.ingest.session_id)
    .bind(chunk.seq as i64)
    .bind(chunk.seq as i64)
    .bind(chunk.byte_len as i64)
    .bind(chunk.ts_ms())
    .execute(store.pool())
    .await?;
    Ok(())
}

async fn mark_backend_acked(
    store: &LocalStore,
    session_id: &str,
    accepted_to_seq: u64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO ingest_sync_state( \
            session_id, backend_acked_seq, dirty_from_seq, dirty_to_seq, \
            dirty_bytes, dirty_events, updated_at \
         ) VALUES (?, ?, NULL, NULL, 0, 0, ?) \
         ON CONFLICT(session_id) DO UPDATE SET \
            backend_acked_seq = CASE \
                WHEN backend_acked_seq < excluded.backend_acked_seq \
                THEN excluded.backend_acked_seq ELSE backend_acked_seq END, \
            dirty_from_seq = CASE \
                WHEN dirty_to_seq IS NULL THEN NULL \
                WHEN dirty_to_seq <= excluded.backend_acked_seq THEN NULL \
                WHEN dirty_from_seq <= excluded.backend_acked_seq THEN excluded.backend_acked_seq + 1 \
                ELSE dirty_from_seq END, \
            dirty_to_seq = CASE \
                WHEN dirty_to_seq IS NULL THEN NULL \
                WHEN dirty_to_seq <= excluded.backend_acked_seq THEN NULL \
                ELSE dirty_to_seq END, \
            dirty_events = CASE \
                WHEN dirty_to_seq IS NULL OR dirty_to_seq <= excluded.backend_acked_seq THEN 0 \
                ELSE dirty_events END, \
            dirty_bytes = CASE \
                WHEN dirty_to_seq IS NULL OR dirty_to_seq <= excluded.backend_acked_seq THEN 0 \
                ELSE dirty_bytes END, \
            updated_at = excluded.updated_at",
    )
    .bind(session_id)
    .bind(accepted_to_seq as i64)
    .bind(now)
    .execute(store.pool())
    .await?;
    Ok(())
}

async fn build_manifest(store: &LocalStore, host_id: DeviceId) -> Result<Option<HostGapManifest>> {
    let rows = sqlx::query_as::<_, ManifestSessionRow>(
        "SELECT \
            t.session_id AS session_id, t.last_seq AS local_to_seq, t.status AS status, \
            COALESCE(s.backend_acked_seq, 0) AS backend_acked_seq, \
            s.dirty_from_seq AS dirty_from_seq, s.dirty_to_seq AS dirty_to_seq \
         FROM sessions t \
         LEFT JOIN ingest_sync_state s ON s.session_id = t.session_id \
         WHERE t.last_seq > COALESCE(s.backend_acked_seq, 0) \
            OR s.dirty_to_seq IS NOT NULL \
         ORDER BY t.last_activity_at DESC \
         LIMIT 500",
    )
    .fetch_all(store.pool())
    .await?;

    let mut sessions = Vec::new();
    for row in rows {
        let backend_acked_seq = row.backend_acked_seq.max(0) as u64;
        let local_to_seq = row.local_to_seq.max(0) as u64;
        if local_to_seq <= backend_acked_seq {
            continue;
        }
        let local_from_seq = backend_acked_seq + 1;
        let range_from = row
            .dirty_from_seq
            .map(|seq| seq.max(1) as u64)
            .unwrap_or(local_from_seq)
            .min(local_from_seq);
        let range_to = row
            .dirty_to_seq
            .map(|seq| seq.max(0) as u64)
            .unwrap_or(local_to_seq)
            .max(local_to_seq);
        let summary = range_summary(store, &row.session_id, local_from_seq, local_to_seq).await?;
        sessions.push(SessionGapManifest {
            session_id: row.session_id,
            backend_acked_seq,
            local_from_seq,
            local_to_seq,
            missing_ranges: vec![SeqRange {
                from: range_from,
                to: range_to,
            }],
            bytes: summary.bytes,
            event_count: summary.event_count,
            first_ts_ms: summary.first_ts_ms,
            last_ts_ms: summary.last_ts_ms,
            running: row.status != "closed",
        });
    }

    if sessions.is_empty() {
        return Ok(None);
    }
    Ok(Some(HostGapManifest {
        manifest_id: Uuid::new_v4().to_string(),
        host_id,
        sessions,
    }))
}

async fn range_summary(
    store: &LocalStore,
    session_id: &str,
    from_seq: u64,
    to_seq: u64,
) -> Result<RangeSummary> {
    let row = sqlx::query_as::<_, RangeSummary>(
        "SELECT \
            COALESCE(SUM(CASE \
                WHEN body_inline IS NOT NULL THEN length(body_inline) \
                ELSE COALESCE(artifact_size_bytes, 0) END), 0) AS bytes, \
            COUNT(*) AS event_count, \
            COALESCE(MIN(ts_ms), 0) AS first_ts_ms, \
            COALESCE(MAX(ts_ms), 0) AS last_ts_ms \
         FROM events WHERE session_id = ? AND seq BETWEEN ? AND ?",
    )
    .bind(session_id)
    .bind(from_seq as i64)
    .bind(to_seq as i64)
    .fetch_one(store.pool())
    .await?;
    Ok(row)
}

async fn build_pull_response(
    store: &LocalStore,
    host_id: DeviceId,
    request_id: String,
    session_id: String,
    from_seq: u64,
    to_seq: u64,
    max_bytes: u64,
) -> Result<HostIngestPullResponse> {
    let thread = store
        .get_session(&session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found for pull: {session_id}"))?;
    let rows = store.read_events(&session_id, from_seq, to_seq).await?;
    let mut chunks = Vec::new();
    let mut bytes = 0_u64;
    let mut last_seq = from_seq.saturating_sub(1);
    for row in rows {
        let chunk = wire_chunk_from_event_row(host_id, &thread, &row)?;
        let would_exceed =
            !chunks.is_empty() && max_bytes > 0 && bytes.saturating_add(chunk.byte_len) > max_bytes;
        if would_exceed {
            break;
        }
        bytes = bytes.saturating_add(chunk.byte_len);
        last_seq = chunk.seq;
        chunks.push(chunk);
    }
    let has_more = last_seq < to_seq;
    Ok(HostIngestPullResponse {
        request_id,
        session_id,
        from_seq,
        to_seq: last_seq,
        chunks,
        has_more,
    })
}

#[derive(sqlx::FromRow)]
struct ManifestSessionRow {
    session_id: String,
    local_to_seq: i64,
    status: String,
    backend_acked_seq: i64,
    dirty_from_seq: Option<i64>,
    dirty_to_seq: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct RangeSummary {
    bytes: u64,
    event_count: u64,
    first_ts_ms: i64,
    last_ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_protocol::realtime::ClientFrame;
    use minos_ui_protocol::UiEventMessage;
    use tokio::sync::watch;

    async fn seed_store() -> Arc<LocalStore> {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("sync.sqlite");
        let store = Arc::new(LocalStore::open(&db_path).await.unwrap());
        // Keep the tempdir alive for the duration of the process. SQLite has
        // the file open, but artifact paths also live under this directory.
        std::mem::forget(tmp);

        sqlx::query(
            "INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO projects(project_id, name, workspace_slug, workspace_path, created_at, updated_at) \
             VALUES ('p-sync', 'Sync', 'sync', '/w', 0, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations(conversation_id, project_id, title, created_at_ms, updated_at_ms) \
             VALUES ('c-sync', 'p-sync', 'Sync', 0, 300)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES ('thr-sync', 'c-sync', '/w', 'codex', 'running', 3, 0, 300)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        let projection_json = serde_json::to_vec(&Vec::<UiEventMessage>::new()).unwrap();
        for seq in 1..=3_i64 {
            sqlx::query(
                "INSERT INTO events( \
                    session_id, seq, body_kind, body_inline, projection_json, ts_ms, source \
                 ) VALUES ('thr-sync', ?, 'inline', ?, ?, ?, 'live')",
            )
            .bind(seq)
            .bind(format!(r#"{{"seq":{seq}}}"#).into_bytes())
            .bind(&projection_json)
            .bind(seq * 100)
            .execute(store.pool())
            .await
            .unwrap();
        }
        store
    }

    fn test_handle(
        store: Arc<LocalStore>,
    ) -> (
        IngestSyncHandle,
        mpsc::Receiver<ClientFrame>,
        mpsc::Receiver<ClientFrame>,
        mpsc::Receiver<ClientFrame>,
    ) {
        let host_id = DeviceId::new();
        let (control_tx, control_rx) = mpsc::channel(8);
        let (live_tx, live_rx) = mpsc::channel(8);
        let (backfill_tx, backfill_rx) = mpsc::channel(8);
        let (_link_tx, link_rx) = watch::channel(RelayLinkState::Connected);
        (
            IngestSyncHandle::spawn(host_id, store, control_tx, live_tx, backfill_tx, link_rx),
            control_rx,
            live_rx,
            backfill_rx,
        )
    }

    #[tokio::test]
    async fn send_manifest_reports_local_gap_metadata() {
        let store = seed_store().await;
        let (sync, mut control_rx, _live_rx, _backfill_rx) = test_handle(store);

        sync.send_manifest().await;

        let frame = control_rx.recv().await.unwrap();
        let ClientFrame::HostGapManifest { manifest } = frame else {
            panic!("expected HostGapManifest");
        };
        assert_eq!(manifest.sessions.len(), 1);
        let session = &manifest.sessions[0];
        assert_eq!(session.session_id, "thr-sync");
        assert_eq!(session.local_from_seq, 1);
        assert_eq!(session.local_to_seq, 3);
        assert_eq!(session.event_count, 3);
        assert!(session.running);
    }

    #[tokio::test]
    async fn handle_pull_range_reads_sqlite_and_emits_response() {
        let store = seed_store().await;
        let (sync, _control_rx, _live_rx, mut backfill_rx) = test_handle(store);

        sync.handle_pull_range(
            "pull-1".into(),
            "thr-sync".into(),
            2,
            3,
            1024,
            PullPriority::ClientOpenedHistory,
            PullReason::ClientOpenedHistory,
        )
        .await;

        let frame = backfill_rx.recv().await.unwrap();
        let ClientFrame::HostIngestPullResponse { response } = frame else {
            panic!("expected HostIngestPullResponse");
        };
        assert_eq!(response.request_id, "pull-1");
        assert_eq!(response.session_id, "thr-sync");
        assert_eq!(response.from_seq, 2);
        assert_eq!(response.to_seq, 3);
        assert!(!response.has_more);
        assert_eq!(response.chunks.len(), 2);
        assert_eq!(response.chunks[0].seq, 2);
        assert_eq!(response.chunks[1].seq, 3);
    }
}
