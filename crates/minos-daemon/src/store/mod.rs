// SQLite always stores integers as i64; the SQL row encodings use i64 even
// where the Rust-side semantics use u64 (sequence numbers, ms timestamps that
// are always positive). Permitting these casts here keeps the SQL-bind
// surface readable without scattering `as_signed` / `try_from` everywhere.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

pub mod artifacts;
pub mod event_writer;
pub mod migrations_loader;

use artifacts::{ArtifactRange, ArtifactStore};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct LocalStore {
    pool: SqlitePool,
    artifact_store: ArtifactStore,
}

impl LocalStore {
    pub async fn open(db_file: &Path) -> anyhow::Result<Self> {
        let artifact_root = db_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("artifacts");
        let url = format!("sqlite://{}?mode=rwc", db_file.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self {
            pool,
            artifact_store: ArtifactStore::new(artifact_root),
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn artifacts(&self) -> &ArtifactStore {
        &self.artifact_store
    }

    pub async fn list_threads(
        &self,
        before_ts_ms: Option<i64>,
        limit: Option<u32>,
        agent: Option<&str>,
    ) -> anyhow::Result<Vec<ThreadRow>> {
        let limit = limit.unwrap_or(50).min(500) as i64;
        let rows = match before_ts_ms {
            Some(ts) => {
                sqlx::query_as::<_, ThreadRow>(
                    "SELECT * FROM threads \
                     WHERE last_activity_at < ? AND (? IS NULL OR agent = ?) \
                     ORDER BY last_activity_at DESC LIMIT ?",
                )
                .bind(ts)
                .bind(agent)
                .bind(agent)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, ThreadRow>(
                    "SELECT * FROM threads \
                     WHERE (? IS NULL OR agent = ?) \
                     ORDER BY last_activity_at DESC LIMIT ?",
                )
                .bind(agent)
                .bind(agent)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    pub async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<ThreadRow>> {
        Ok(
            sqlx::query_as::<_, ThreadRow>("SELECT * FROM threads WHERE thread_id = ?")
                .bind(thread_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn read_events(
        &self,
        thread_id: &str,
        from_seq: u64,
        to_seq: u64,
    ) -> anyhow::Result<Vec<EventRow>> {
        Ok(sqlx::query_as::<_, EventRow>(
            "SELECT thread_id, seq, body_kind, body_inline, artifact_id, artifact_size_bytes, \
                    artifact_sha256, artifact_media_type, projection_json, ts_ms, source \
             FROM events WHERE thread_id = ? AND seq BETWEEN ? AND ? ORDER BY seq ASC",
        )
        .bind(thread_id)
        .bind(from_seq as i64)
        .bind(to_seq as i64)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Flip every thread whose status is neither `closed` nor `suspended`
    /// to `suspended { daemon_restart }`. Returns the number of rows
    /// affected so callers can log the recovery footprint.
    pub async fn mark_orphans_suspended(&self) -> anyhow::Result<u64> {
        let r = sqlx::query(
            "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart' \
             WHERE status NOT IN ('closed', 'suspended')",
        )
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Idempotent workspace upsert. The `threads.workspace_root` FK requires
    /// the parent row to exist before any `INSERT INTO threads` succeeds.
    /// `INSERT OR IGNORE` keeps `first_seen_at` from the original create and
    /// doesn't bump `last_seen_at` — `update_workspace_seen` does that.
    pub async fn upsert_workspace(&self, root: &str, ts_ms: i64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO workspaces(root, first_seen_at, last_seen_at) \
             VALUES (?, ?, ?)",
        )
        .bind(root)
        .bind(ts_ms)
        .bind(ts_ms)
        .execute(&self.pool)
        .await?;
        sqlx::query("UPDATE workspaces SET last_seen_at = ? WHERE root = ?")
            .bind(ts_ms)
            .bind(root)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Persist a freshly-spawned thread. Idempotent on `thread_id` so callers
    /// don't have to round-trip a SELECT before calling — `INSERT OR IGNORE`
    /// makes a duplicate `start_agent` (e.g. after a UI retry) a benign
    /// no-op rather than a constraint violation. The `events.thread_id` FK
    /// is the load-bearing reason this exists: without a parent threads row
    /// every `EventWriter::write_live` for the thread fails with SQLite
    /// error 787.
    pub async fn insert_thread(
        &self,
        thread_id: &str,
        workspace_root: &str,
        agent: &str,
        codex_session_id: Option<&str>,
        status: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO threads( \
                thread_id, workspace_root, agent, codex_session_id, status, \
                last_seq, started_at, last_activity_at \
             ) VALUES (?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(thread_id)
        .bind(workspace_root)
        .bind(agent)
        .bind(codex_session_id)
        .bind(status)
        .bind(ts_ms)
        .bind(ts_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_thread_provider_session_id(
        &self,
        thread_id: &str,
        provider_session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE threads SET codex_session_id = ? WHERE thread_id = ?")
            .bind(provider_session_id)
            .bind(thread_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Stamp a thread closed: `status='closed'`, `ended_at=ts_ms`,
    /// `last_close_reason=reason`. No-op (zero rows updated) if the row is
    /// missing — callers treat that as success because there's nothing left
    /// to persist about a thread we never recorded.
    pub async fn close_thread_row(
        &self,
        thread_id: &str,
        reason: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE threads SET status = 'closed', last_close_reason = ?, \
                                ended_at = ?, last_activity_at = ? \
             WHERE thread_id = ?",
        )
        .bind(reason)
        .bind(ts_ms)
        .bind(ts_ms)
        .bind(thread_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_thread_status(
        &self,
        thread_id: &str,
        status: &str,
        pause_reason: Option<&str>,
        close_reason: Option<&str>,
        ended_at: Option<i64>,
        ts_ms: i64,
    ) -> anyhow::Result<u64> {
        let result = sqlx::query(
            "UPDATE threads SET status = ?, last_pause_reason = ?, \
                                last_close_reason = ?, ended_at = ?, \
                                last_activity_at = ? \
             WHERE thread_id = ?",
        )
        .bind(status)
        .bind(pause_reason)
        .bind(close_reason)
        .bind(ended_at)
        .bind(ts_ms)
        .bind(thread_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_thread(&self, thread_id: &str) -> anyhow::Result<u64> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM events WHERE thread_id = ?")
            .bind(thread_id)
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query("DELETE FROM threads WHERE thread_id = ?")
            .bind(thread_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.artifact_store
            .delete_thread_artifacts(thread_id)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn read_artifact_range(
        &self,
        thread_id: &str,
        artifact_id: &str,
        offset: u64,
        limit: u32,
    ) -> anyhow::Result<ArtifactRange> {
        self.artifact_store
            .read_range(thread_id, artifact_id, offset, limit)
            .await
    }

    // ── Project CRUD ──────────────────────────────────────────────────

    /// Create a new project. Returns the inserted row.
    pub async fn create_project(
        &self,
        project_id: &str,
        name: &str,
        workspace_slug: &str,
        workspace_path: Option<&str>,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO projects(project_id, name, workspace_slug, workspace_path, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(project_id)
        .bind(name)
        .bind(workspace_slug)
        .bind(workspace_path)
        .bind(ts_ms)
        .bind(ts_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List all projects ordered by most recently updated.
    pub async fn list_projects(&self) -> anyhow::Result<Vec<ProjectRow>> {
        Ok(
            sqlx::query_as::<_, ProjectRow>("SELECT * FROM projects ORDER BY updated_at DESC")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Get a single project by id.
    pub async fn get_project(&self, project_id: &str) -> anyhow::Result<Option<ProjectRow>> {
        Ok(
            sqlx::query_as::<_, ProjectRow>("SELECT * FROM projects WHERE project_id = ?")
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Update a project's name and bump `updated_at`.
    pub async fn update_project_name(
        &self,
        project_id: &str,
        name: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE projects SET name = ?, updated_at = ? WHERE project_id = ?")
            .bind(name)
            .bind(ts_ms)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete a project. Threads referencing it will have their project_id
    /// set to NULL (SQLite FK ON DELETE SET NULL behavior is not configured,
    /// so we do it manually).
    pub async fn delete_project(&self, project_id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE threads SET project_id = NULL WHERE project_id = ?")
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM projects WHERE project_id = ?")
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List threads belonging to a specific project.
    pub async fn list_threads_by_project(
        &self,
        project_id: &str,
        before_ts_ms: Option<i64>,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadRow>> {
        let limit = limit.unwrap_or(50).min(500) as i64;
        let rows = match before_ts_ms {
            Some(ts) => {
                sqlx::query_as::<_, ThreadRow>(
                    "SELECT * FROM threads \
                     WHERE project_id = ? AND last_activity_at < ? \
                     ORDER BY last_activity_at DESC LIMIT ?",
                )
                .bind(project_id)
                .bind(ts)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, ThreadRow>(
                    "SELECT * FROM threads \
                     WHERE project_id = ? \
                     ORDER BY last_activity_at DESC LIMIT ?",
                )
                .bind(project_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    /// Assign a thread to a project.
    pub async fn assign_thread_to_project(
        &self,
        thread_id: &str,
        project_id: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE threads SET project_id = ? WHERE thread_id = ?")
            .bind(project_id)
            .bind(thread_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Touch a project's `updated_at` timestamp.
    pub async fn touch_project(&self, project_id: &str, ts_ms: i64) -> anyhow::Result<()> {
        sqlx::query("UPDATE projects SET updated_at = ? WHERE project_id = ?")
            .bind(ts_ms)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub project_id: String,
    pub name: String,
    pub workspace_slug: String,
    pub workspace_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for ProjectRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            project_id: row.try_get("project_id")?,
            name: row.try_get("name")?,
            workspace_slug: row.try_get("workspace_slug")?,
            workspace_path: row.try_get("workspace_path")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub thread_id: String,
    pub workspace_root: String,
    pub agent: String,
    pub codex_session_id: Option<String>,
    pub status: String,
    pub last_pause_reason: Option<String>,
    pub last_close_reason: Option<String>,
    pub last_seq: i64,
    pub started_at: i64,
    pub last_activity_at: i64,
    pub ended_at: Option<i64>,
    pub project_id: Option<String>,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for ThreadRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            thread_id: row.try_get("thread_id")?,
            workspace_root: row.try_get("workspace_root")?,
            agent: row.try_get("agent")?,
            codex_session_id: row.try_get("codex_session_id")?,
            status: row.try_get("status")?,
            last_pause_reason: row.try_get("last_pause_reason")?,
            last_close_reason: row.try_get("last_close_reason")?,
            last_seq: row.try_get("last_seq")?,
            started_at: row.try_get("started_at")?,
            last_activity_at: row.try_get("last_activity_at")?,
            ended_at: row.try_get("ended_at")?,
            project_id: row.try_get("project_id")?,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub thread_id: String,
    pub seq: i64,
    pub body_kind: String,
    pub body_inline: Option<Vec<u8>>,
    pub artifact_id: Option<String>,
    pub artifact_size_bytes: Option<i64>,
    pub artifact_sha256: Option<String>,
    pub artifact_media_type: Option<String>,
    pub projection_json: Vec<u8>,
    pub ts_ms: i64,
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("test.sqlite");
        let store = LocalStore::open(&p).await.unwrap();
        let row: (i64,) = sqlx::query_as("SELECT count(*) FROM events")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn mark_orphans_suspended_flips_running_idle() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w',0,0)")
            .execute(store.pool())
            .await
            .unwrap();
        for (i, status) in ["running", "idle", "closed", "suspended"]
            .iter()
            .enumerate()
        {
            sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, '/w', 'codex', ?, 0, ?, ?)")
                .bind(format!("t{i}"))
                .bind(*status)
                .bind(i as i64)
                .bind(i as i64)
                .execute(store.pool())
                .await
                .unwrap();
        }
        let n = store.mark_orphans_suspended().await.unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn list_and_get_threads() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        for i in 0..3 {
            sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, '/w', 'codex', 'idle', 0, ?, ?)")
                .bind(format!("thr-{i}"))
                .bind(i as i64)
                .bind(i as i64)
                .execute(store.pool())
                .await
                .unwrap();
        }
        let threads = store.list_threads(None, None, None).await.unwrap();
        assert_eq!(threads.len(), 3);
        let one = store.get_thread("thr-1").await.unwrap();
        assert_eq!(one.unwrap().agent, "codex");
    }

    #[tokio::test]
    async fn update_thread_status_persists_runtime_state() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES ('thr-status', '/w', 'codex', 'idle', 0, 1, 1)")
            .execute(store.pool())
            .await
            .unwrap();

        let updated = store
            .update_thread_status("thr-status", "running", None, None, None, 42)
            .await
            .unwrap();
        assert_eq!(updated, 1);

        let row = store.get_thread("thr-status").await.unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert_eq!(row.last_activity_at, 42);
        assert!(row.last_pause_reason.is_none());
        assert!(row.last_close_reason.is_none());
        assert!(row.ended_at.is_none());
    }

    #[tokio::test]
    async fn delete_thread_removes_thread_and_events() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES ('thr-delete', '/w', 'codex', 'idle', 1, 0, 0)")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO events(thread_id, seq, body_kind, body_inline, projection_json, ts_ms, source) VALUES ('thr-delete', 1, 'inline', ?, ?, 0, 'live')",
        )
        .bind(br#"{"method":"item/started"}"#.as_slice())
        .bind(b"[]".as_slice())
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ingest_sync_state( \
                thread_id, backend_acked_seq, dirty_from_seq, dirty_to_seq, \
                dirty_bytes, dirty_events, updated_at \
             ) VALUES ('thr-delete', 0, 1, 1, 1, 1, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();

        let deleted = store.delete_thread("thr-delete").await.unwrap();

        assert_eq!(deleted, 1);
        assert!(store.get_thread("thr-delete").await.unwrap().is_none());
        let events: (i64,) = sqlx::query_as("SELECT count(*) FROM events WHERE thread_id = ?")
            .bind("thr-delete")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(events.0, 0);
        let sync_rows: (i64,) =
            sqlx::query_as("SELECT count(*) FROM ingest_sync_state WHERE thread_id = ?")
                .bind("thr-delete")
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(sync_rows.0, 0);
    }

    #[tokio::test]
    async fn list_threads_filters_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w', 0, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        for (thread_id, agent, ts) in [("thr-a", "codex", 10), ("thr-b", "claude", 20)] {
            sqlx::query("INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, '/w', ?, 'idle', 0, ?, ?)")
                .bind(thread_id)
                .bind(agent)
                .bind(ts)
                .bind(ts)
                .execute(store.pool())
                .await
                .unwrap();
        }

        let threads = store
            .list_threads(None, None, Some("claude"))
            .await
            .unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, "thr-b");
    }
}
