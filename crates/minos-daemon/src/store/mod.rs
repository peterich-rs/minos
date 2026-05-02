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

pub mod event_writer;
pub mod migrations_loader;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct LocalStore {
    pool: SqlitePool,
}

impl LocalStore {
    pub async fn open(db_file: &Path) -> anyhow::Result<Self> {
        let url = format!("sqlite://{}?mode=rwc", db_file.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn list_threads(
        &self,
        before_ts_ms: Option<i64>,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadRow>> {
        let limit = limit.unwrap_or(50).min(500) as i64;
        let rows = match before_ts_ms {
            Some(ts) => {
                sqlx::query_as::<_, ThreadRow>(
                    "SELECT * FROM threads WHERE last_activity_at < ? ORDER BY last_activity_at DESC LIMIT ?",
                )
                .bind(ts)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, ThreadRow>(
                    "SELECT * FROM threads ORDER BY last_activity_at DESC LIMIT ?",
                )
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
            "SELECT thread_id, seq, payload, ts_ms, source FROM events WHERE thread_id = ? AND seq BETWEEN ? AND ? ORDER BY seq ASC",
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
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub thread_id: String,
    pub seq: i64,
    pub payload: Vec<u8>,
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
        let threads = store.list_threads(None, None).await.unwrap();
        assert_eq!(threads.len(), 3);
        let one = store.get_thread("thr-1").await.unwrap();
        assert_eq!(one.unwrap().agent, "codex");
    }
}
