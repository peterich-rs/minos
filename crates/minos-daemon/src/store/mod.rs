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
pub mod bot_delivery_ledger;
pub mod event_writer;
pub mod migrations_loader;

use artifacts::{ArtifactRange, ArtifactStore};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct LocalStore {
    pool: SqlitePool,
    artifact_store: ArtifactStore,
    db_path: std::path::PathBuf,
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
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .foreign_keys(true)
            // Windows CI (and concurrent EventWriter + migration traffic) can
            // briefly hit SQLITE_BUSY (517) without a busy timeout.
            .busy_timeout(std::time::Duration::from_secs(5))
            .pragma("busy_timeout", "5000");
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self {
            pool,
            artifact_store: ArtifactStore::new(artifact_root),
            db_path: db_file.to_path_buf(),
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn artifacts(&self) -> &ArtifactStore {
        &self.artifact_store
    }

    pub async fn list_sessions(
        &self,
        before_ts_ms: Option<i64>,
        limit: Option<u32>,
        agent: Option<&str>,
    ) -> anyhow::Result<Vec<SessionRow>> {
        let limit = limit.unwrap_or(50).min(500) as i64;
        let rows = match before_ts_ms {
            Some(ts) => {
                sqlx::query_as::<_, SessionRow>(
                    "SELECT * FROM sessions \
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
                sqlx::query_as::<_, SessionRow>(
                    "SELECT * FROM sessions \
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

    pub async fn get_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRow>> {
        Ok(
            sqlx::query_as::<_, SessionRow>("SELECT * FROM sessions WHERE session_id = ?")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn read_events(
        &self,
        session_id: &str,
        from_seq: u64,
        to_seq: u64,
    ) -> anyhow::Result<Vec<EventRow>> {
        Ok(sqlx::query_as::<_, EventRow>(
            "SELECT session_id, seq, body_kind, body_inline, artifact_id, artifact_size_bytes, \
                    artifact_sha256, artifact_media_type, projection_json, ts_ms, source \
             FROM events WHERE session_id = ? AND seq BETWEEN ? AND ? ORDER BY seq ASC",
        )
        .bind(session_id)
        .bind(from_seq as i64)
        .bind(to_seq as i64)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Recover orphan sessions after unclean process death.
    ///
    /// - **Mid-flight** (`running` / `starting` / `resuming`) → `suspended` +
    ///   `needs_continue=1` so open path can inject CONTINUE.
    /// - **Idle** is left as **`idle`**. The agent process is gone, but the
    ///   session is still "between turns" / ready to reattach — it must not
    ///   surface as user-visible **Paused** after Desktop/daemon restart.
    /// - Already `closed` / `suspended` rows are untouched.
    ///
    /// `resume_session` reattaches when the row is idle but no live process exists.
    pub async fn mark_orphans_suspended(&self) -> anyhow::Result<u64> {
        let r = sqlx::query(
            "UPDATE sessions SET \
                needs_continue = 1, \
                status = 'suspended', \
                last_pause_reason = 'daemon_restart', \
                last_close_reason = NULL, \
                ended_at = NULL \
             WHERE status IN ('running', 'starting', 'resuming')",
        )
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Persist daemon-stop outcome for one session (sync path; survives process exit).
    ///
    /// - `needs_continue == true` (was mid-flight): `suspended` + flag for CONTINUE.
    /// - `needs_continue == false` (was idle / already paused without mid-flight):
    ///   keep **`idle`** so restart does not rebrand a finished turn as Paused.
    pub async fn suspend_thread_for_daemon_restart(
        &self,
        session_id: &str,
        needs_continue: bool,
        ts_ms: i64,
    ) -> anyhow::Result<u64> {
        let result = if needs_continue {
            sqlx::query(
                "UPDATE sessions SET status = 'suspended', last_pause_reason = 'daemon_restart', \
                                    last_close_reason = NULL, ended_at = NULL, \
                                    needs_continue = 1, last_activity_at = ? \
                 WHERE session_id = ? AND status != 'closed'",
            )
            .bind(ts_ms)
            .bind(session_id)
            .execute(&self.pool)
            .await?
        } else {
            // Between turns: durable ready state is idle (reattach on next use).
            // Force idle even if a concurrent bridge briefly wrote suspended —
            // shutdown is the authority for DaemonRestart durable status.
            sqlx::query(
                "UPDATE sessions SET status = 'idle', last_pause_reason = NULL, \
                                    last_close_reason = NULL, ended_at = NULL, \
                                    needs_continue = 0, last_activity_at = ? \
                 WHERE session_id = ? AND status != 'closed'",
            )
            .bind(ts_ms)
            .bind(session_id)
            .execute(&self.pool)
            .await?
        };
        Ok(result.rows_affected())
    }

    /// Atomically clear `needs_continue` and return whether it was set.
    /// Used so user-send and auto-continue cannot both inject a continue turn.
    pub async fn take_needs_continue(&self, session_id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query(
            "UPDATE sessions SET needs_continue = 0 \
             WHERE session_id = ? AND needs_continue != 0",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_needs_continue(
        &self,
        session_id: &str,
        needs_continue: bool,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE sessions SET needs_continue = ? WHERE session_id = ?")
            .bind(i64::from(needs_continue))
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Idempotent workspace upsert. The `sessions.workspace_root` FK requires
    /// the parent row to exist before any `INSERT INTO sessions` succeeds.
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

    /// Persist a freshly-spawned session. Idempotent on `session_id` so callers
    /// don't have to round-trip a SELECT before calling — `INSERT OR IGNORE`
    /// makes a duplicate `start_agent` (e.g. after a UI retry) a benign
    /// no-op rather than a constraint violation. The `events.session_id` FK
    /// is the load-bearing reason this exists: without a parent sessions row
    /// every `EventWriter::write_live` for the session fails with SQLite
    /// error 787.
    pub async fn insert_session(
        &self,
        session_id: &str,
        conversation_id: &str,
        workspace_root: &str,
        agent: &str,
        bot_id: Option<&str>,
        provider_session_id: Option<&str>,
        parent_session_id: Option<&str>,
        status: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO sessions( \
                session_id, conversation_id, workspace_root, parent_session_id, agent, bot_id, \
                provider_session_id, status, last_seq, started_at, last_activity_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(session_id)
        .bind(conversation_id)
        .bind(workspace_root)
        .bind(parent_session_id)
        .bind(agent)
        .bind(bot_id)
        .bind(provider_session_id)
        .bind(status)
        .bind(ts_ms)
        .bind(ts_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_session_provider_session_id(
        &self,
        session_id: &str,
        provider_session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE sessions SET provider_session_id = ? WHERE session_id = ?")
            .bind(provider_session_id)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Stamp a session closed: `status='closed'`, `ended_at=ts_ms`,
    /// `last_close_reason=reason`. No-op (zero rows updated) if the row is
    /// missing — callers treat that as success because there's nothing left
    /// to persist about a session we never recorded.
    pub async fn close_session_row(
        &self,
        session_id: &str,
        reason: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE sessions SET status = 'closed', last_close_reason = ?, \
                                ended_at = ?, last_activity_at = ? \
             WHERE session_id = ?",
        )
        .bind(reason)
        .bind(ts_ms)
        .bind(ts_ms)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_session_status(
        &self,
        session_id: &str,
        status: &str,
        pause_reason: Option<&str>,
        close_reason: Option<&str>,
        ended_at: Option<i64>,
        ts_ms: i64,
    ) -> anyhow::Result<u64> {
        let result = sqlx::query(
            "UPDATE sessions SET status = ?, last_pause_reason = ?, \
                                last_close_reason = ?, ended_at = ?, \
                                last_activity_at = ? \
             WHERE session_id = ?",
        )
        .bind(status)
        .bind(pause_reason)
        .bind(close_reason)
        .bind(ended_at)
        .bind(ts_ms)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_session(&self, session_id: &str) -> anyhow::Result<u64> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM events WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query("DELETE FROM sessions WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.artifact_store
            .delete_session_artifacts(session_id)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn read_artifact_range(
        &self,
        session_id: &str,
        artifact_id: &str,
        offset: u64,
        limit: u32,
    ) -> anyhow::Result<ArtifactRange> {
        self.artifact_store
            .read_range(session_id, artifact_id, offset, limit)
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

    /// Find a project by exact workspace path (Hub collab bind to Desktop project).
    pub async fn find_project_by_workspace_path(
        &self,
        workspace_path: &str,
    ) -> anyhow::Result<Option<ProjectRow>> {
        let path = workspace_path.trim();
        if path.is_empty() {
            return Ok(None);
        }
        Ok(sqlx::query_as::<_, ProjectRow>(
            "SELECT * FROM projects WHERE workspace_path = ? ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Ensure a project row exists (idempotent). Used when Hub passes a project_id
    /// that Desktop has not yet created locally, or when binding by workspace path.
    pub async fn ensure_project(
        &self,
        project_id: &str,
        name: &str,
        workspace_slug: &str,
        workspace_path: Option<&str>,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        if self.get_project(project_id).await?.is_some() {
            return Ok(());
        }
        match self
            .create_project(project_id, name, workspace_slug, workspace_path, ts_ms)
            .await
        {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .chain()
                    .any(|c| c.to_string().contains("UNIQUE constraint failed")) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Ensure a conversation row exists under `project_id` with the **same** id
    /// as the Hub conversation (no synthetic conversation-{slug} rewrite).
    pub async fn ensure_conversation(
        &self,
        conversation_id: &str,
        project_id: &str,
        title: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        if let Some(existing) = self.get_conversation(conversation_id).await? {
            // Keep existing project binding; refresh title if empty / placeholder.
            if existing.title.trim().is_empty() || existing.title == "Direct agent sessions" {
                let _ = self
                    .update_conversation_fields(conversation_id, Some(title), None, None, ts_ms)
                    .await?;
            }
            return Ok(());
        }
        match self
            .create_conversation(conversation_id, project_id, title, ts_ms)
            .await
        {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .chain()
                    .any(|c| c.to_string().contains("UNIQUE constraint failed")) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
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

    /// Delete a project. Conversation/message/session rows cascade through the
    /// schema; no legacy reassignment path is kept.
    pub async fn delete_project(&self, project_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM projects WHERE project_id = ?")
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn count_conversations_by_project(&self, project_id: &str) -> anyhow::Result<u32> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE project_id = ?")
                .bind(project_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(u32::try_from(count.max(0)).unwrap_or(u32::MAX))
    }

    // ── Conversation CRUD ──────────────────────────────────────────────

    /// Optional product metadata at create time (git snapshot, tags).
    pub async fn create_conversation(
        &self,
        conversation_id: &str,
        project_id: &str,
        title: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        self.create_conversation_with_meta(
            conversation_id,
            project_id,
            title,
            ts_ms,
            &ConversationCreateMeta::default(),
        )
        .await
    }

    pub async fn create_conversation_with_meta(
        &self,
        conversation_id: &str,
        project_id: &str,
        title: &str,
        ts_ms: i64,
        meta: &ConversationCreateMeta,
    ) -> anyhow::Result<()> {
        let progress = meta
            .progress
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("todo");
        sqlx::query(
            "INSERT INTO conversations( \
                conversation_id, project_id, title, created_at_ms, updated_at_ms, \
                priority, progress, branch, worktree_path, git_mode, git_dirty, git_head \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(project_id)
        .bind(title)
        .bind(ts_ms)
        .bind(ts_ms)
        .bind(meta.priority.as_deref())
        .bind(progress)
        .bind(meta.branch.as_deref())
        .bind(meta.worktree_path.as_deref())
        .bind(meta.git_mode.as_deref())
        .bind(meta.git_dirty.map(i64::from))
        .bind(meta.git_head.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// All non-empty conversation worktree paths (for startup orphan prune).
    pub async fn list_registered_worktree_paths(&self) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT worktree_path FROM conversations \
             WHERE worktree_path IS NOT NULL AND TRIM(worktree_path) != ''",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(p,)| p).collect())
    }

    /// Persist live git binding fields for a conversation work unit.
    pub async fn update_conversation_git_fields(
        &self,
        conversation_id: &str,
        branch: Option<&str>,
        worktree_path: Option<&str>,
        git_mode: Option<&str>,
        git_dirty: Option<bool>,
        git_head: Option<&str>,
        ts_ms: i64,
    ) -> anyhow::Result<bool> {
        // Do not bump updated_at_ms — that column is last-message activity only.
        let _ = ts_ms;
        let result = sqlx::query(
            "UPDATE conversations SET \
                branch = ?, \
                worktree_path = ?, \
                git_mode = COALESCE(?, git_mode), \
                git_dirty = ?, \
                git_head = ? \
             WHERE conversation_id = ?",
        )
        .bind(branch)
        .bind(worktree_path)
        .bind(git_mode)
        .bind(git_dirty.map(i64::from))
        .bind(git_head)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Option<ConversationRow>> {
        Ok(sqlx::query_as::<_, ConversationRow>(&format!(
            "SELECT {} FROM conversations c WHERE c.conversation_id = ?",
            CONVERSATION_SELECT_COLS
        ))
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_conversation_fields(
        &self,
        conversation_id: &str,
        title: Option<&str>,
        priority: Option<Option<&str>>,
        progress: Option<&str>,
        ts_ms: i64,
    ) -> anyhow::Result<bool> {
        // priority: None = leave; Some(None) = clear; Some(Some(v)) = set
        let mut sets = Vec::new();
        if title.is_some() {
            sets.push("title = ?");
        }
        if priority.is_some() {
            sets.push("priority = ?");
        }
        if progress.is_some() {
            sets.push("progress = ?");
        }
        if sets.is_empty() {
            return Ok(false);
        }
        // title / priority / progress must not move last-message activity.
        let _ = ts_ms;
        let sql = format!(
            "UPDATE conversations SET {} WHERE conversation_id = ?",
            sets.join(", ")
        );
        let mut query = sqlx::query(&sql);
        if let Some(t) = title {
            query = query.bind(t);
        }
        if let Some(p) = priority {
            query = query.bind(p);
        }
        if let Some(pr) = progress {
            query = query.bind(pr);
        }
        query = query.bind(conversation_id);
        let result = query.execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    /// Bump progress from `todo` → `in_progress` once work actually starts.
    pub async fn promote_conversation_in_progress_if_todo(
        &self,
        conversation_id: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        let _ = ts_ms;
        sqlx::query(
            "UPDATE conversations \
             SET progress = 'in_progress' \
             WHERE conversation_id = ? AND progress = 'todo'",
        )
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_conversations_by_project(
        &self,
        project_id: &str,
        before_updated_at_ms: Option<i64>,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<ConversationRow>> {
        let limit = limit.unwrap_or(50).min(500) as i64;
        let rows = match before_updated_at_ms {
            Some(ts) => {
                sqlx::query_as::<_, ConversationRow>(&format!(
                    "SELECT * FROM ( \
                        SELECT {CONVERSATION_SELECT_COLS} \
                        FROM conversations c \
                        WHERE c.project_id = ? \
                     ) WHERE updated_at_ms < ? \
                     ORDER BY updated_at_ms DESC, conversation_id LIMIT ?"
                ))
                .bind(project_id)
                .bind(ts)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, ConversationRow>(&format!(
                    "SELECT * FROM ( \
                        SELECT {CONVERSATION_SELECT_COLS} \
                        FROM conversations c \
                        WHERE c.project_id = ? \
                     ) \
                     ORDER BY updated_at_ms DESC, conversation_id LIMIT ?"
                ))
                .bind(project_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    /// Roster bot_ids allowed in each conversation (membership SSOT).
    /// Distinct from sessions: a member may have zero sessions yet.
    ///
    /// Values are `bot_id` strings (not runtime labels). UI layers that need
    /// runtime badges resolve via [`Self::list_conversation_roster`] / bot_identities.
    pub async fn list_agents_for_conversations(
        &self,
        conversation_ids: &[String],
    ) -> anyhow::Result<HashMap<String, Vec<String>>> {
        if conversation_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = std::iter::repeat_n("?", conversation_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT conversation_id, bot_id, brief, joined_at_ms FROM conversation_agent_members \
             WHERE conversation_id IN ({placeholders}) \
             ORDER BY conversation_id, joined_at_ms ASC, bot_id"
        );
        let mut query = sqlx::query(&sql);
        for id in conversation_ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(&self.pool).await?;
        let mut by_conversation = HashMap::<String, Vec<String>>::new();
        for row in rows {
            by_conversation
                .entry(row.try_get("conversation_id")?)
                .or_default()
                .push(row.try_get("bot_id")?);
        }
        Ok(by_conversation)
    }

    /// Full roster rows (bot_id + resolved identity + brief + join time).
    pub async fn list_conversation_roster(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Vec<ConversationAgentMemberRow>> {
        let rows = sqlx::query(
            "SELECT m.bot_id, m.brief, m.joined_at_ms, \
                    b.runtime_agent, b.display_name \
             FROM conversation_agent_members m \
             LEFT JOIN bot_identities b ON b.bot_id = m.bot_id \
             WHERE m.conversation_id = ? \
             ORDER BY m.joined_at_ms ASC, m.bot_id",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let brief: String = row.try_get("brief")?;
            let runtime: Option<String> = row.try_get("runtime_agent")?;
            let display: Option<String> = row.try_get("display_name")?;
            out.push(ConversationAgentMemberRow {
                bot_id: row.try_get("bot_id")?,
                brief: if brief.is_empty() { None } else { Some(brief) },
                joined_at_ms: row.try_get("joined_at_ms")?,
                runtime_agent: runtime.filter(|s| !s.is_empty()),
                display_name: display.filter(|s| !s.is_empty()),
            });
        }
        Ok(out)
    }

    /// Membership check by bot identity id.
    pub async fn is_conversation_agent_member(
        &self,
        conversation_id: &str,
        bot_id: &str,
    ) -> anyhow::Result<bool> {
        let trimmed = bot_id.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM conversation_agent_members \
             WHERE conversation_id = ? AND bot_id = ? LIMIT 1",
        )
        .bind(conversation_id)
        .bind(trimmed)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Offline start helper: true when any roster bot for this conversation has
    /// the given runtime family (join roster + bot_identities).
    pub async fn is_member_by_runtime(
        &self,
        conversation_id: &str,
        runtime_agent: &str,
    ) -> anyhow::Result<bool> {
        let runtime = runtime_agent.trim().to_ascii_lowercase();
        if runtime.is_empty() {
            return Ok(false);
        }
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM conversation_agent_members m \
             INNER JOIN bot_identities b ON b.bot_id = m.bot_id \
             WHERE m.conversation_id = ? AND lower(b.runtime_agent) = ? \
             LIMIT 1",
        )
        .bind(conversation_id)
        .bind(runtime)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Replace the full agent roster for a conversation (deduped by bot_id, order preserved).
    pub async fn set_conversation_agent_members(
        &self,
        conversation_id: &str,
        members: &[ConversationAgentMemberInput],
        joined_at_ms: i64,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM conversation_agent_members WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;
        let mut seen = std::collections::HashSet::new();
        for member in members {
            let trimmed = member.bot_id.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
                continue;
            }
            let brief = normalize_roster_brief(member.brief.as_deref());
            sqlx::query(
                "INSERT INTO conversation_agent_members \
                 (conversation_id, bot_id, joined_at_ms, brief) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(conversation_id)
            .bind(trimmed)
            .bind(joined_at_ms)
            .bind(brief)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Idempotent add of one bot to the conversation roster.
    /// When the row already exists, `brief` is updated when non-empty.
    pub async fn add_conversation_agent_member(
        &self,
        conversation_id: &str,
        bot_id: &str,
        joined_at_ms: i64,
        brief: Option<&str>,
    ) -> anyhow::Result<()> {
        let trimmed = bot_id.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let brief = normalize_roster_brief(brief);
        sqlx::query(
            "INSERT INTO conversation_agent_members \
             (conversation_id, bot_id, joined_at_ms, brief) VALUES (?, ?, ?, ?) \
             ON CONFLICT(conversation_id, bot_id) DO UPDATE SET \
               brief = CASE \
                 WHEN excluded.brief != '' THEN excluded.brief \
                 ELSE conversation_agent_members.brief \
               END",
        )
        .bind(conversation_id)
        .bind(trimmed)
        .bind(joined_at_ms)
        .bind(brief)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove one bot from the conversation roster.
    /// Returns `true` when a membership row was deleted.
    pub async fn remove_conversation_agent_member(
        &self,
        conversation_id: &str,
        bot_id: &str,
    ) -> anyhow::Result<bool> {
        let trimmed = bot_id.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let result = sqlx::query(
            "DELETE FROM conversation_agent_members \
             WHERE conversation_id = ? AND bot_id = ?",
        )
        .bind(conversation_id)
        .bind(trimmed)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remove all roster members whose bot identity has the given runtime.
    /// Returns bot_ids that were removed.
    pub async fn remove_conversation_members_by_runtime(
        &self,
        conversation_id: &str,
        runtime_agent: &str,
    ) -> anyhow::Result<Vec<String>> {
        let runtime = runtime_agent.trim().to_ascii_lowercase();
        if runtime.is_empty() {
            return Ok(Vec::new());
        }
        let bot_ids: Vec<(String,)> = sqlx::query_as(
            "SELECT m.bot_id FROM conversation_agent_members m \
             INNER JOIN bot_identities b ON b.bot_id = m.bot_id \
             WHERE m.conversation_id = ? AND lower(b.runtime_agent) = ?",
        )
        .bind(conversation_id)
        .bind(&runtime)
        .fetch_all(&self.pool)
        .await?;
        let mut removed = Vec::new();
        for (bot_id,) in bot_ids {
            if self
                .remove_conversation_agent_member(conversation_id, &bot_id)
                .await?
            {
                removed.push(bot_id);
            }
        }
        Ok(removed)
    }

    pub async fn list_sessions_by_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Vec<SessionRow>> {
        Ok(sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM sessions \
             WHERE conversation_id = ? \
             ORDER BY last_activity_at DESC, session_id",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn insert_session_in_conversation(
        &self,
        session_id: &str,
        conversation_id: &str,
        workspace_root: &str,
        agent: &str,
        bot_id: Option<&str>,
        provider_session_id: Option<&str>,
        parent_session_id: Option<&str>,
        status: &str,
        ts_ms: i64,
        count_as_agent_session: bool,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO sessions( \
                session_id, conversation_id, workspace_root, parent_session_id, agent, bot_id, \
                provider_session_id, status, last_seq, started_at, last_activity_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(session_id)
        .bind(conversation_id)
        .bind(workspace_root)
        .bind(parent_session_id)
        .bind(agent)
        .bind(bot_id)
        .bind(provider_session_id)
        .bind(status)
        .bind(ts_ms)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;
        if count_as_agent_session && inserted.rows_affected() > 0 {
            // Session membership is not a chat-message event — do not bump
            // last-message activity (`updated_at_ms`).
            sqlx::query(
                "UPDATE conversations \
                 SET agent_session_count = agent_session_count + 1 \
                 WHERE conversation_id = ?",
            )
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Insert or update a durable conversation timeline row.
    ///
    /// # Ordering contract
    /// - **`message_seq`** (SQLite rowid / primary key) is the only sort key for the
    ///   conversation timeline. Clients must display `ORDER BY message_seq ASC`.
    /// - On **first insert** of a new `message_id`, `message_seq` is assigned and never
    ///   reassigned. Concurrent multi-agent finishes are ordered by durable insert order
    ///   (finish/write order), not by wall-clock start of the agent turn.
    /// - On **upsert** of an existing `message_id`, body/metadata update in place and
    ///   `message_seq` is preserved so streaming rewrites do not reorder history.
    /// - `created_at_ms` is display-only; do not sort by it.
    /// - `reply_to_message_id` / `mentions_json` express causality (e.g. delegation
    ///   result → request) without changing sort order.
    /// - List filters hide rows whose `session_id` is a subagent (`parent_session_id`
    ///   set); those belong in session transcript, not the group timeline.
    pub async fn upsert_conversation_message(
        &self,
        conversation_id: &str,
        message_id: &str,
        session_id: Option<&str>,
        sender_role: &str,
        bot_id: Option<&str>,
        body: &str,
        ts_ms: i64,
        reply_to_message_id: Option<&str>,
        delegation_id: Option<&str>,
        mentions_json: &str,
    ) -> anyhow::Result<i64> {
        let preview = body.chars().take(120).collect::<String>();
        let mut tx = self.pool.begin().await?;
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT message_seq FROM chat_messages WHERE message_id = ?")
                .bind(message_id)
                .fetch_optional(&mut *tx)
                .await?;

        let message_seq = if let Some(seq) = existing {
            sqlx::query(
                "UPDATE chat_messages \
                 SET session_id = ?, sender_role = ?, bot_id = ?, body = ?, \
                     reply_to_message_id = ?, delegation_id = ?, mentions_json = ? \
                 WHERE message_id = ?",
            )
            .bind(session_id)
            .bind(sender_role)
            .bind(bot_id)
            .bind(body)
            .bind(reply_to_message_id)
            .bind(delegation_id)
            .bind(mentions_json)
            .bind(message_id)
            .execute(&mut *tx)
            .await?;
            seq
        } else {
            let result = sqlx::query(
                "INSERT INTO chat_messages( \
                    message_id, conversation_id, session_id, created_at_ms, sender_role, bot_id, body, \
                    reply_to_message_id, delegation_id, mentions_json \
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(message_id)
            .bind(conversation_id)
            .bind(session_id)
            .bind(ts_ms)
            .bind(sender_role)
            .bind(bot_id)
            .bind(body)
            .bind(reply_to_message_id)
            .bind(delegation_id)
            .bind(mentions_json)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE conversations \
                 SET message_count = message_count + 1 \
                 WHERE conversation_id = ?",
            )
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;
            result.last_insert_rowid()
        };

        // Sole writer of conversations.updated_at_ms outside create: last-message
        // activity (preview + epoch). Title/git/session paths must not touch it.
        sqlx::query(
            "UPDATE conversations \
             SET last_message_preview = ?, updated_at_ms = ? \
             WHERE conversation_id = ?",
        )
        .bind(preview)
        .bind(ts_ms)
        .bind(conversation_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(message_seq)
    }

    pub async fn list_conversation_messages(
        &self,
        conversation_id: &str,
        before_seq: Option<i64>,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<ChatMessageRow>> {
        let limit = limit.unwrap_or(100).min(500) as i64;
        let rows = match before_seq {
            Some(seq) => {
                sqlx::query_as::<_, ChatMessageRow>(
                    "SELECT m.* FROM chat_messages m \
                     LEFT JOIN sessions t ON t.session_id = m.session_id \
                     WHERE m.conversation_id = ? \
                       AND m.message_seq < ? \
                       AND (m.session_id IS NULL OR t.session_id IS NULL OR t.parent_session_id IS NULL) \
                     ORDER BY m.message_seq DESC LIMIT ?",
                )
                .bind(conversation_id)
                .bind(seq)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, ChatMessageRow>(
                    "SELECT m.* FROM chat_messages m \
                     LEFT JOIN sessions t ON t.session_id = m.session_id \
                     WHERE m.conversation_id = ? \
                       AND (m.session_id IS NULL OR t.session_id IS NULL OR t.parent_session_id IS NULL) \
                     ORDER BY m.message_seq DESC LIMIT ?",
                )
                .bind(conversation_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    /// Load raw reaction rows for a batch of message ids (any order).
    pub async fn list_reactions_for_messages(
        &self,
        message_ids: &[String],
    ) -> anyhow::Result<Vec<ChatMessageReactionRow>> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        // sqlx SQLite has no array bind; build a small IN list of placeholders.
        let placeholders = vec!["?"; message_ids.len()].join(", ");
        let sql = format!(
            "SELECT reaction_id, message_id, emoji, actor_id, actor_kind, display_name, created_at_ms \
             FROM chat_message_reactions \
             WHERE message_id IN ({placeholders}) \
             ORDER BY message_id ASC, emoji ASC, created_at_ms ASC"
        );
        let mut query = sqlx::query_as::<_, ChatMessageReactionRow>(&sql);
        for id in message_ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Look up conversation_id for a chat message (for reaction RPC validation).
    pub async fn get_message_conversation_id(
        &self,
        message_id: &str,
    ) -> anyhow::Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT conversation_id FROM chat_messages WHERE message_id = ?")
                .bind(message_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    /// Load body + conversation for reaction mention gates.
    pub async fn get_message_body_for_reaction(
        &self,
        message_id: &str,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT conversation_id, body, COALESCE(mentions_json, '[]') \
             FROM chat_messages WHERE message_id = ?",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Idempotent toggle for the host local user: remove if present, else insert.
    /// SELECT + DELETE/INSERT run in one write transaction so concurrent double-clicks
    /// serialize instead of racing on the UNIQUE(message_id, emoji, actor_id) constraint.
    /// Returns `(conversation_id, added)` after the mutation.
    pub async fn toggle_local_message_reaction(
        &self,
        message_id: &str,
        emoji: &str,
        reaction_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<(String, bool)> {
        self.toggle_message_reaction(
            message_id,
            emoji,
            reaction_id,
            minos_protocol::LOCAL_REACTION_ACTOR_ID,
            minos_protocol::LOCAL_REACTION_ACTOR_KIND,
            minos_protocol::LOCAL_REACTION_DISPLAY_NAME,
            now_ms,
        )
        .await
    }

    /// Toggle reaction for an arbitrary actor (user/local or agent).
    /// Returns `(conversation_id, added)`.
    pub async fn toggle_message_reaction(
        &self,
        message_id: &str,
        emoji: &str,
        reaction_id: &str,
        actor_id: &str,
        actor_kind: &str,
        display_name: &str,
        now_ms: i64,
    ) -> anyhow::Result<(String, bool)> {
        let mut tx = self.pool.begin().await?;

        // Validate message exists and take a write lock on the parent row so
        // concurrent toggles for this message_id queue (SQLite reserved lock).
        let conversation_id: Option<String> = sqlx::query_scalar(
            "UPDATE chat_messages SET message_id = message_id \
             WHERE message_id = ? RETURNING conversation_id",
        )
        .bind(message_id)
        .fetch_optional(&mut *tx)
        .await?;
        let conversation_id =
            conversation_id.ok_or_else(|| anyhow::anyhow!("message not found: {message_id}"))?;

        let existing: Option<String> = sqlx::query_scalar(
            "SELECT reaction_id FROM chat_message_reactions \
             WHERE message_id = ? AND emoji = ? AND actor_id = ?",
        )
        .bind(message_id)
        .bind(emoji)
        .bind(actor_id)
        .fetch_optional(&mut *tx)
        .await?;

        let added = if let Some(existing_id) = existing {
            sqlx::query("DELETE FROM chat_message_reactions WHERE reaction_id = ?")
                .bind(&existing_id)
                .execute(&mut *tx)
                .await?;
            false
        } else {
            sqlx::query(
                "INSERT INTO chat_message_reactions \
                 (reaction_id, message_id, emoji, actor_id, actor_kind, display_name, created_at_ms) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(reaction_id)
            .bind(message_id)
            .bind(emoji)
            .bind(actor_id)
            .bind(actor_kind)
            .bind(display_name)
            .bind(now_ms)
            .execute(&mut *tx)
            .await?;
            true
        };

        tx.commit().await?;
        Ok((conversation_id, added))
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
pub struct SessionRow {
    pub session_id: String,
    pub conversation_id: String,
    pub workspace_root: String,
    /// Runtime agent label for CLI launch (execution detail).
    pub agent: String,
    /// Bot identity link when known.
    pub bot_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub status: String,
    pub last_pause_reason: Option<String>,
    pub last_close_reason: Option<String>,
    pub last_seq: i64,
    pub started_at: i64,
    pub last_activity_at: i64,
    pub ended_at: Option<i64>,
    pub needs_continue: bool,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for SessionRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        let needs_continue_i: i64 = row.try_get("needs_continue").unwrap_or(0);
        Ok(Self {
            session_id: row.try_get("session_id")?,
            conversation_id: row.try_get("conversation_id")?,
            workspace_root: row.try_get("workspace_root")?,
            agent: row.try_get("agent")?,
            bot_id: row.try_get("bot_id")?,
            provider_session_id: row.try_get("provider_session_id")?,
            parent_session_id: row.try_get("parent_session_id")?,
            status: row.try_get("status")?,
            last_pause_reason: row.try_get("last_pause_reason")?,
            last_close_reason: row.try_get("last_close_reason")?,
            last_seq: row.try_get("last_seq")?,
            started_at: row.try_get("started_at")?,
            last_activity_at: row.try_get("last_activity_at")?,
            ended_at: row.try_get("ended_at")?,
            needs_continue: needs_continue_i != 0,
        })
    }
}

/// One roster membership write (create / set members) — keyed by bot_id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationAgentMemberInput {
    pub bot_id: String,
    pub brief: Option<String>,
}

/// Durable roster row with optional peer-facing brief and resolved identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationAgentMemberRow {
    pub bot_id: String,
    pub brief: Option<String>,
    pub joined_at_ms: i64,
    /// From bot_identities when present.
    pub runtime_agent: Option<String>,
    /// From bot_identities when present.
    pub display_name: Option<String>,
}

/// Cap and trim roster briefs stored in SQLite.
pub fn normalize_roster_brief(raw: Option<&str>) -> String {
    let trimmed = raw.map(str::trim).unwrap_or("").to_owned();
    if trimmed.is_empty() {
        return String::new();
    }
    const MAX: usize = 500;
    if trimmed.chars().count() <= MAX {
        trimmed
    } else {
        trimmed.chars().take(MAX).collect()
    }
}

/// Shared SELECT list for conversation rows (list + get).
/// Includes live thread aggregates for board / attention chips.
///
/// **`updated_at_ms` (list last activity):** derived as
/// `MAX(top-level chat_messages.created_at_ms)` (exclude subagent sessions),
/// falling back to `c.created_at_ms` when empty. Physical `conversations.updated_at_ms`
/// is only written on durable message upsert (not title/git/session count) so it
/// stays aligned; list/get still derive so subagent filter stays correct.
const CONVERSATION_SELECT_COLS: &str = "\
    c.conversation_id, c.project_id, c.title, \
    (SELECT m.body FROM chat_messages m \
     LEFT JOIN sessions t ON t.session_id = m.session_id \
     WHERE m.conversation_id = c.conversation_id \
       AND (m.session_id IS NULL OR t.session_id IS NULL OR t.parent_session_id IS NULL) \
     ORDER BY m.message_seq DESC LIMIT 1) AS last_message_preview, \
    (SELECT COUNT(*) FROM chat_messages m \
     LEFT JOIN sessions t ON t.session_id = m.session_id \
     WHERE m.conversation_id = c.conversation_id \
       AND (m.session_id IS NULL OR t.session_id IS NULL OR t.parent_session_id IS NULL)) AS message_count, \
    c.agent_session_count, c.created_at_ms, \
    COALESCE((SELECT MAX(m.created_at_ms) FROM chat_messages m \
              LEFT JOIN sessions t ON t.session_id = m.session_id \
              WHERE m.conversation_id = c.conversation_id \
                AND (m.session_id IS NULL OR t.session_id IS NULL OR t.parent_session_id IS NULL)), c.created_at_ms) AS updated_at_ms, \
    c.priority, \
    COALESCE(c.progress, 'todo') AS progress, \
    c.branch, \
    c.worktree_path, \
    c.git_mode, \
    c.git_dirty, \
    c.git_head, \
    (SELECT COUNT(*) FROM sessions th \
     WHERE th.conversation_id = c.conversation_id \
       AND th.status IN ('starting', 'running', 'resuming')) AS running_count, \
    (SELECT COUNT(*) FROM sessions th \
     WHERE th.conversation_id = c.conversation_id \
       AND th.status = 'suspended' \
       AND (th.needs_continue != 0 \
            OR COALESCE(th.last_pause_reason, '') NOT IN ('daemon_restart', ''))) \
       AS needs_attention_count";

#[derive(Debug, Clone, Default)]
pub struct ConversationCreateMeta {
    pub priority: Option<String>,
    pub progress: Option<String>,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub git_mode: Option<String>,
    pub git_dirty: Option<bool>,
    pub git_head: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConversationRow {
    pub conversation_id: String,
    pub project_id: String,
    pub title: String,
    pub last_message_preview: Option<String>,
    pub message_count: i64,
    pub agent_session_count: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: Option<String>,
    pub progress: String,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub git_mode: Option<String>,
    pub git_dirty: Option<i64>,
    pub git_head: Option<String>,
    pub running_count: i64,
    pub needs_attention_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChatMessageRow {
    pub message_seq: i64,
    pub message_id: String,
    pub conversation_id: String,
    pub session_id: Option<String>,
    pub created_at_ms: i64,
    pub sender_role: String,
    pub bot_id: Option<String>,
    pub body: String,
    pub reply_to_message_id: Option<String>,
    pub delegation_id: Option<String>,
    pub mentions_json: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChatMessageReactionRow {
    pub reaction_id: String,
    pub message_id: String,
    pub emoji: String,
    pub actor_id: String,
    pub actor_kind: String,
    pub display_name: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub session_id: String,
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

// ── Host bot identities (replaces agent_profiles) ─────────────────────────

/// Source values for [`BotIdentityRow::source`].
pub const BOT_SOURCE_USER_CONFIGURED: &str = "user_configured";
pub const BOT_SOURCE_HOST_RUNTIME_SEED: &str = "host_runtime_seed";

/// Stable bot_id prefix for offline runtime seeds (`local-rt-{runtime}`).
pub fn local_runtime_bot_id(runtime: &str) -> String {
    format!("local-rt-{}", runtime.trim().to_ascii_lowercase())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BotIdentityRow {
    pub bot_id: String,
    pub display_name: String,
    pub description: String,
    pub runtime_agent: String,
    pub model: String,
    pub reasoning_effort: String,
    pub system_prompt: String,
    pub env_json: String,
    pub source: String,
    pub owner_account_id: Option<String>,
    pub synced_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Wire-compat view of a bot identity as a historical AgentProfile row.
/// `id` == `bot_id`; `name` == `display_name`; `instructions` == `system_prompt`.
#[derive(Debug, Clone)]
pub struct AgentProfileRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub runtime_agent: String,
    pub model: String,
    pub reasoning_effort: String,
    pub instructions: String,
    pub env_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl From<BotIdentityRow> for AgentProfileRow {
    fn from(row: BotIdentityRow) -> Self {
        Self {
            id: row.bot_id,
            name: row.display_name,
            description: row.description,
            runtime_agent: row.runtime_agent,
            model: row.model,
            reasoning_effort: row.reasoning_effort,
            instructions: row.system_prompt,
            env_json: row.env_json,
            created_at_ms: row.created_at_ms,
            updated_at_ms: row.updated_at_ms,
        }
    }
}

impl LocalStore {
    const BOT_SELECT_COLS: &'static str = "bot_id, display_name, description, runtime_agent, \
        model, reasoning_effort, system_prompt, env_json, source, owner_account_id, \
        synced_at_ms, created_at_ms, updated_at_ms";

    pub async fn list_bot_identities(&self) -> anyhow::Result<Vec<BotIdentityRow>> {
        let sql = format!(
            "SELECT {} FROM bot_identities ORDER BY updated_at_ms DESC",
            Self::BOT_SELECT_COLS
        );
        let rows = sqlx::query_as::<_, BotIdentityRow>(&sql)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn get_bot_identity(&self, bot_id: &str) -> anyhow::Result<Option<BotIdentityRow>> {
        let sql = format!(
            "SELECT {} FROM bot_identities WHERE bot_id = ?",
            Self::BOT_SELECT_COLS
        );
        let row = sqlx::query_as::<_, BotIdentityRow>(&sql)
            .bind(bot_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Insert or replace a bot identity row (cloud sync / local upsert).
    pub async fn upsert_bot_identity(
        &self,
        bot_id: &str,
        display_name: &str,
        description: &str,
        runtime_agent: &str,
        model: &str,
        reasoning_effort: &str,
        system_prompt: &str,
        env_json: &str,
        source: &str,
        owner_account_id: Option<&str>,
        synced_at_ms: Option<i64>,
        now_ms: i64,
    ) -> anyhow::Result<BotIdentityRow> {
        sqlx::query(
            "INSERT INTO bot_identities ( \
                bot_id, display_name, description, runtime_agent, model, reasoning_effort, \
                system_prompt, env_json, source, owner_account_id, synced_at_ms, \
                created_at_ms, updated_at_ms \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(bot_id) DO UPDATE SET \
                display_name = excluded.display_name, \
                description = excluded.description, \
                runtime_agent = excluded.runtime_agent, \
                model = excluded.model, \
                reasoning_effort = excluded.reasoning_effort, \
                system_prompt = excluded.system_prompt, \
                env_json = excluded.env_json, \
                source = excluded.source, \
                owner_account_id = excluded.owner_account_id, \
                synced_at_ms = excluded.synced_at_ms, \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(bot_id)
        .bind(display_name)
        .bind(description)
        .bind(runtime_agent)
        .bind(model)
        .bind(reasoning_effort)
        .bind(system_prompt)
        .bind(env_json)
        .bind(source)
        .bind(owner_account_id)
        .bind(synced_at_ms)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        self.get_bot_identity(bot_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("bot identity missing after upsert"))
    }

    pub async fn create_bot_identity(
        &self,
        bot_id: &str,
        display_name: &str,
        description: &str,
        runtime_agent: &str,
        model: &str,
        reasoning_effort: &str,
        system_prompt: &str,
        now_ms: i64,
    ) -> anyhow::Result<BotIdentityRow> {
        self.upsert_bot_identity(
            bot_id,
            display_name,
            description,
            runtime_agent,
            model,
            reasoning_effort,
            system_prompt,
            "[]",
            BOT_SOURCE_USER_CONFIGURED,
            None,
            None,
            now_ms,
        )
        .await
    }

    pub async fn update_bot_identity(
        &self,
        bot_id: &str,
        display_name: &str,
        description: &str,
        system_prompt: &str,
        now_ms: i64,
    ) -> anyhow::Result<Option<BotIdentityRow>> {
        let n = sqlx::query(
            "UPDATE bot_identities SET display_name = ?, description = ?, system_prompt = ?, \
             updated_at_ms = ? WHERE bot_id = ?",
        )
        .bind(display_name)
        .bind(description)
        .bind(system_prompt)
        .bind(now_ms)
        .bind(bot_id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if n == 0 {
            return Ok(None);
        }
        self.get_bot_identity(bot_id).await
    }

    pub async fn delete_bot_identity(&self, bot_id: &str) -> anyhow::Result<bool> {
        let n = sqlx::query("DELETE FROM bot_identities WHERE bot_id = ?")
            .bind(bot_id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    /// Ensure a stable host-runtime seed bot exists for `runtime` (`local-rt-{runtime}`).
    /// Upserts if missing; returns bot_id.
    pub async fn ensure_local_runtime_bot(
        &self,
        runtime: &str,
        now_ms: i64,
    ) -> anyhow::Result<String> {
        let runtime = runtime.trim().to_ascii_lowercase();
        if runtime.is_empty() {
            anyhow::bail!("runtime is empty");
        }
        let bot_id = local_runtime_bot_id(&runtime);
        if self.get_bot_identity(&bot_id).await?.is_some() {
            return Ok(bot_id);
        }
        self.upsert_bot_identity(
            &bot_id,
            &runtime,
            "",
            &runtime,
            "",
            "",
            "",
            "[]",
            BOT_SOURCE_HOST_RUNTIME_SEED,
            None,
            None,
            now_ms,
        )
        .await?;
        Ok(bot_id)
    }

    // ── Thin agent_profile wire wrappers (id ↔ bot_id) ────────────────────

    pub async fn list_agent_profiles(&self) -> anyhow::Result<Vec<AgentProfileRow>> {
        Ok(self
            .list_bot_identities()
            .await?
            .into_iter()
            .map(AgentProfileRow::from)
            .collect())
    }

    pub async fn create_agent_profile(
        &self,
        id: &str,
        name: &str,
        description: &str,
        runtime_agent: &str,
        model: &str,
        reasoning_effort: &str,
        instructions: &str,
        now_ms: i64,
    ) -> anyhow::Result<AgentProfileRow> {
        let row = self
            .create_bot_identity(
                id,
                name,
                description,
                runtime_agent,
                model,
                reasoning_effort,
                instructions,
                now_ms,
            )
            .await?;
        Ok(AgentProfileRow::from(row))
    }

    pub async fn get_agent_profile(&self, id: &str) -> anyhow::Result<Option<AgentProfileRow>> {
        Ok(self.get_bot_identity(id).await?.map(AgentProfileRow::from))
    }

    pub async fn update_agent_profile(
        &self,
        id: &str,
        name: &str,
        description: &str,
        instructions: &str,
        now_ms: i64,
    ) -> anyhow::Result<Option<AgentProfileRow>> {
        Ok(self
            .update_bot_identity(id, name, description, instructions, now_ms)
            .await?
            .map(AgentProfileRow::from))
    }

    pub async fn delete_agent_profile(&self, id: &str) -> anyhow::Result<bool> {
        self.delete_bot_identity(id).await
    }

    /// Newest non-empty bot `description` for a runtime (peer role-brief fallback).
    ///
    /// Prefer identity-scoped briefs when available; conversation roster `brief`
    /// overrides. Used when listing roster / building session-start briefings.
    pub async fn latest_profile_description_for_runtime(
        &self,
        runtime_agent: &str,
    ) -> anyhow::Result<Option<String>> {
        let runtime = runtime_agent.trim().to_ascii_lowercase();
        if runtime.is_empty() {
            return Ok(None);
        }
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT description FROM bot_identities \
             WHERE lower(runtime_agent) = ? AND trim(description) != '' \
             ORDER BY updated_at_ms DESC LIMIT 1",
        )
        .bind(runtime)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|(d,)| normalize_roster_brief(Some(d.as_str())))
            .filter(|s| !s.is_empty()))
    }

    /// Newest non-empty description for a specific bot (roster brief fallback).
    pub async fn bot_description(&self, bot_id: &str) -> anyhow::Result<Option<String>> {
        let row = self.get_bot_identity(bot_id).await?;
        Ok(row
            .map(|r| normalize_roster_brief(Some(r.description.as_str())))
            .filter(|s| !s.is_empty()))
    }

    /// Fill empty roster briefs from the bot identity description (then runtime fallback).
    pub async fn enrich_roster_with_profile_briefs(
        &self,
        members: Vec<ConversationAgentMemberRow>,
    ) -> anyhow::Result<Vec<ConversationAgentMemberRow>> {
        let mut out = Vec::with_capacity(members.len());
        for mut m in members {
            let has_brief = m
                .brief
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            if !has_brief {
                if let Some(desc) = self.bot_description(&m.bot_id).await? {
                    m.brief = Some(desc);
                } else if let Some(runtime) = m.runtime_agent.clone() {
                    if let Some(desc) = self
                        .latest_profile_description_for_runtime(&runtime)
                        .await?
                    {
                        m.brief = Some(desc);
                    }
                }
            } else if let Some(b) = m.brief.take() {
                let n = normalize_roster_brief(Some(b.as_str()));
                m.brief = if n.is_empty() { None } else { Some(n) };
            }
            out.push(m);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_conversation(store: &LocalStore) {
        store
            .create_project("p", "Project", "project", Some("/w"), 0)
            .await
            .unwrap();
        store
            .create_conversation("c", "p", "Conversation", 0)
            .await
            .unwrap();
    }

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
    async fn conversation_metadata_create_and_update() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("meta.sqlite"))
            .await
            .unwrap();
        store
            .create_project("p", "Project", "project", Some("/w"), 0)
            .await
            .unwrap();
        store
            .create_conversation_with_meta(
                "c1",
                "p",
                "JWT work",
                10,
                &ConversationCreateMeta {
                    priority: Some("high".into()),
                    progress: Some("todo".into()),
                    branch: Some("feature/jwt".into()),
                    worktree_path: Some("/tmp/wt/jwt".into()),
                    git_mode: Some("worktree".into()),
                    git_dirty: Some(false),
                    git_head: Some("abc1234".into()),
                },
            )
            .await
            .unwrap();

        let row = store.get_conversation("c1").await.unwrap().unwrap();
        assert_eq!(row.title, "JWT work");
        assert_eq!(row.priority.as_deref(), Some("high"));
        assert_eq!(row.progress, "todo");
        assert_eq!(row.branch.as_deref(), Some("feature/jwt"));
        assert_eq!(row.worktree_path.as_deref(), Some("/tmp/wt/jwt"));
        assert_eq!(row.git_mode.as_deref(), Some("worktree"));
        assert_eq!(row.git_dirty, Some(0));
        assert_eq!(row.git_head.as_deref(), Some("abc1234"));

        store
            .update_conversation_fields(
                "c1",
                Some("JWT auth refactor"),
                Some(Some("medium")),
                Some("in_progress"),
                20,
            )
            .await
            .unwrap();
        let row = store.get_conversation("c1").await.unwrap().unwrap();
        assert_eq!(row.title, "JWT auth refactor");
        assert_eq!(row.priority.as_deref(), Some("medium"));
        assert_eq!(row.progress, "in_progress");

        store
            .promote_conversation_in_progress_if_todo("c1", 30)
            .await
            .unwrap();
        // already in_progress — unchanged
        let row = store.get_conversation("c1").await.unwrap().unwrap();
        assert_eq!(row.progress, "in_progress");

        store
            .create_conversation("c2", "p", "Other", 40)
            .await
            .unwrap();
        store
            .promote_conversation_in_progress_if_todo("c2", 50)
            .await
            .unwrap();
        let row = store.get_conversation("c2").await.unwrap().unwrap();
        assert_eq!(row.progress, "in_progress");
        assert!(row.branch.is_none());
    }

    #[tokio::test]
    async fn conversation_agent_membership_is_roster_not_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("members.sqlite"))
            .await
            .unwrap();
        store
            .create_project("p", "Project", "project", Some("/w"), 0)
            .await
            .unwrap();
        store
            .create_conversation("c1", "p", "Team", 10)
            .await
            .unwrap();

        let codex_bot = store.ensure_local_runtime_bot("codex", 10).await.unwrap();
        let claude_bot = store.ensure_local_runtime_bot("claude", 10).await.unwrap();
        assert_eq!(codex_bot, "local-rt-codex");
        assert_eq!(claude_bot, "local-rt-claude");

        store
            .set_conversation_agent_members(
                "c1",
                &[
                    ConversationAgentMemberInput {
                        bot_id: codex_bot.clone(),
                        brief: Some("implements features".into()),
                    },
                    ConversationAgentMemberInput {
                        bot_id: claude_bot.clone(),
                        brief: Some("reviews PRs".into()),
                    },
                    ConversationAgentMemberInput {
                        bot_id: codex_bot.clone(),
                        brief: None,
                    },
                ],
                11,
            )
            .await
            .unwrap();
        let roster = store.list_conversation_roster("c1").await.unwrap();
        assert_eq!(roster.len(), 2);
        // Same joined_at_ms → secondary sort by bot_id.
        assert_eq!(roster[0].bot_id, claude_bot);
        assert_eq!(roster[0].brief.as_deref(), Some("reviews PRs"));
        assert_eq!(roster[0].runtime_agent.as_deref(), Some("claude"));
        assert_eq!(roster[1].bot_id, codex_bot);
        assert_eq!(roster[1].brief.as_deref(), Some("implements features"));
        let map = store
            .list_agents_for_conversations(&["c1".into()])
            .await
            .unwrap();
        assert_eq!(
            map.get("c1").map(Vec::as_slice),
            Some(&[claude_bot.clone(), codex_bot.clone()][..])
        );
        assert!(store
            .is_conversation_agent_member("c1", &codex_bot)
            .await
            .unwrap());
        assert!(!store
            .is_conversation_agent_member("c1", "local-rt-grok")
            .await
            .unwrap());
        assert!(store.is_member_by_runtime("c1", "codex").await.unwrap());
        assert!(!store.is_member_by_runtime("c1", "grok").await.unwrap());

        let grok_bot = store.ensure_local_runtime_bot("grok", 12).await.unwrap();
        store
            .add_conversation_agent_member("c1", &grok_bot, 12, Some("experiments"))
            .await
            .unwrap();
        assert!(store
            .is_conversation_agent_member("c1", &grok_bot)
            .await
            .unwrap());

        assert!(store
            .remove_conversation_agent_member("c1", &grok_bot)
            .await
            .unwrap());
        assert!(!store
            .is_conversation_agent_member("c1", &grok_bot)
            .await
            .unwrap());
        assert!(!store
            .remove_conversation_agent_member("c1", &grok_bot)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn roster_brief_falls_back_to_bot_identity_description() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("profile-brief.sqlite"))
            .await
            .unwrap();
        store
            .create_project("p", "Project", "project", Some("/w"), 0)
            .await
            .unwrap();
        store
            .create_conversation("c1", "p", "Team", 10)
            .await
            .unwrap();
        store
            .create_agent_profile(
                "bot-codex",
                "Feature Codex",
                "implements features in the worktree",
                "codex",
                "gpt-5",
                "",
                "",
                200,
            )
            .await
            .unwrap();
        store
            .create_agent_profile(
                "bot-claude",
                "Reviewer",
                "from profile — should not win over roster brief",
                "claude",
                "opus",
                "",
                "",
                200,
            )
            .await
            .unwrap();
        store
            .set_conversation_agent_members(
                "c1",
                &[
                    ConversationAgentMemberInput {
                        bot_id: "bot-codex".into(),
                        brief: None,
                    },
                    ConversationAgentMemberInput {
                        bot_id: "bot-claude".into(),
                        brief: Some("conversation override".into()),
                    },
                ],
                11,
            )
            .await
            .unwrap();

        let raw = store.list_conversation_roster("c1").await.unwrap();
        let enriched = store.enrich_roster_with_profile_briefs(raw).await.unwrap();
        let by_bot: std::collections::HashMap<_, _> =
            enriched.into_iter().map(|m| (m.bot_id, m.brief)).collect();
        assert_eq!(
            by_bot.get("bot-codex").and_then(|b| b.as_deref()),
            Some("implements features in the worktree")
        );
        assert_eq!(
            by_bot.get("bot-claude").and_then(|b| b.as_deref()),
            Some("conversation override")
        );
    }

    #[tokio::test]
    async fn mark_orphans_suspended_flips_mid_flight_only_keeps_idle() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w',0,0)")
            .execute(store.pool())
            .await
            .unwrap();
        seed_conversation(&store).await;
        for (i, status) in [
            "running",
            "idle",
            "closed",
            "suspended",
            "starting",
            "resuming",
        ]
        .iter()
        .enumerate()
        {
            sqlx::query("INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, 'c', '/w', 'codex', ?, 0, ?, ?)")
                .bind(format!("t{i}"))
                .bind(*status)
                .bind(i as i64)
                .bind(i as i64)
                .execute(store.pool())
                .await
                .unwrap();
        }
        let n = store.mark_orphans_suspended().await.unwrap();
        // running, starting, resuming — idle stays idle
        assert_eq!(n, 3);
        let running = store.get_session("t0").await.unwrap().unwrap();
        assert_eq!(running.status, "suspended");
        assert!(running.needs_continue);
        let idle = store.get_session("t1").await.unwrap().unwrap();
        assert_eq!(idle.status, "idle");
        assert!(!idle.needs_continue);
        let closed = store.get_session("t2").await.unwrap().unwrap();
        assert_eq!(closed.status, "closed");
        let already = store.get_session("t3").await.unwrap().unwrap();
        assert_eq!(already.status, "suspended");
        assert!(!already.needs_continue);
        let starting = store.get_session("t4").await.unwrap().unwrap();
        assert!(starting.needs_continue);
        let resuming = store.get_session("t5").await.unwrap().unwrap();
        assert!(resuming.needs_continue);
    }

    #[tokio::test]
    async fn take_needs_continue_is_one_shot() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&tmp.path().join("t.sqlite"))
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces(root, first_seen_at, last_seen_at) VALUES ('/w',0,0)")
            .execute(store.pool())
            .await
            .unwrap();
        seed_conversation(&store).await;
        sqlx::query(
            "INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, \
             last_seq, started_at, last_activity_at, needs_continue) \
             VALUES ('t-nc', 'c', '/w', 'codex', 'suspended', 0, 0, 0, 1)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        assert!(store.take_needs_continue("t-nc").await.unwrap());
        assert!(!store.take_needs_continue("t-nc").await.unwrap());
        assert!(
            !store
                .get_session("t-nc")
                .await
                .unwrap()
                .unwrap()
                .needs_continue
        );
    }

    #[tokio::test]
    async fn list_and_get_sessions() {
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
        seed_conversation(&store).await;
        for i in 0..3 {
            sqlx::query("INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, 'c', '/w', 'codex', 'idle', 0, ?, ?)")
                .bind(format!("thr-{i}"))
                .bind(i as i64)
                .bind(i as i64)
                .execute(store.pool())
                .await
                .unwrap();
        }
        let sessions = store.list_sessions(None, None, None).await.unwrap();
        assert_eq!(sessions.len(), 3);
        let one = store.get_session("thr-1").await.unwrap();
        assert_eq!(one.unwrap().agent, "codex");
    }

    #[tokio::test]
    async fn subagent_thread_keeps_parent_without_counting_as_agent_session() {
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
        seed_conversation(&store).await;

        store
            .insert_session_in_conversation(
                "parent",
                "c",
                "/w",
                "codex",
                Some("local-rt-codex"),
                None,
                None,
                "idle",
                10,
                true,
            )
            .await
            .unwrap();
        store
            .insert_session_in_conversation(
                "sub",
                "c",
                "/w",
                "codex",
                Some("local-rt-codex"),
                None,
                Some("parent"),
                "idle",
                11,
                false,
            )
            .await
            .unwrap();

        let sub = store.get_session("sub").await.unwrap().unwrap();
        assert_eq!(sub.parent_session_id.as_deref(), Some("parent"));
        let count: (i64,) = sqlx::query_as(
            "SELECT agent_session_count FROM conversations WHERE conversation_id = 'c'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn list_conversation_messages_hides_subagent_thread_messages() {
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
        seed_conversation(&store).await;
        store
            .insert_session_in_conversation(
                "parent",
                "c",
                "/w",
                "codex",
                Some("local-rt-codex"),
                None,
                None,
                "idle",
                10,
                true,
            )
            .await
            .unwrap();
        store
            .insert_session_in_conversation(
                "sub",
                "c",
                "/w",
                "codex",
                Some("local-rt-codex"),
                None,
                Some("parent"),
                "idle",
                11,
                false,
            )
            .await
            .unwrap();
        store
            .upsert_conversation_message(
                "c", "user", None, "user", None, "prompt", 12, None, None, "[]",
            )
            .await
            .unwrap();
        store
            .upsert_conversation_message(
                "c",
                "parent-result",
                Some("parent"),
                "agent",
                Some("local-rt-codex"),
                "parent result",
                13,
                None,
                None,
                "[]",
            )
            .await
            .unwrap();
        store
            .upsert_conversation_message(
                "c",
                "sub-result",
                Some("sub"),
                "agent",
                Some("local-rt-codex"),
                "sub result",
                14,
                None,
                None,
                "[]",
            )
            .await
            .unwrap();

        let rows = store
            .list_conversation_messages("c", None, Some(10))
            .await
            .unwrap();

        assert_eq!(
            rows.iter().map(|row| row.body.as_str()).collect::<Vec<_>>(),
            vec!["parent result", "prompt"]
        );

        let conversations = store
            .list_conversations_by_project("p", None, Some(10))
            .await
            .unwrap();
        assert_eq!(conversations.len(), 1);
        assert_eq!(
            conversations[0].last_message_preview.as_deref(),
            Some("parent result")
        );
        assert_eq!(conversations[0].message_count, 2);
    }

    /// Timeline order is durable insert order (`message_seq`), not wall clock alone.
    #[tokio::test]
    async fn conversation_messages_ordered_by_message_seq_and_upsert_preserves_seq() {
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
        seed_conversation(&store).await;
        store
            .insert_session_in_conversation(
                "t-a",
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
        store
            .insert_session_in_conversation(
                "t-b",
                "c",
                "/w",
                "claude",
                Some("local-rt-claude"),
                None,
                None,
                "idle",
                2,
                true,
            )
            .await
            .unwrap();

        let seq_user = store
            .upsert_conversation_message(
                "c",
                "user-1",
                None,
                "user",
                None,
                "please help",
                100,
                None,
                None,
                "[]",
            )
            .await
            .unwrap();
        let seq_delegate = store
            .upsert_conversation_message(
                "c",
                "mcp-delegation:c:1",
                Some("t-a"),
                "agent",
                Some("local-rt-codex"),
                "@claude#tb do X",
                200,
                None,
                Some("del-1"),
                "[]",
            )
            .await
            .unwrap();
        // Target finishes before source (finish-order insert).
        let seq_target = store
            .upsert_conversation_message(
                "c",
                "agent-result:c:t-b:k1",
                Some("t-b"),
                "agent",
                Some("local-rt-claude"),
                "@codex#ta done",
                300,
                Some("mcp-delegation:c:1"),
                Some("del-1"),
                "[]",
            )
            .await
            .unwrap();
        let seq_source = store
            .upsert_conversation_message(
                "c",
                "agent-result:c:t-a:k2",
                Some("t-a"),
                "agent",
                Some("local-rt-codex"),
                "here is the answer",
                400,
                None,
                None,
                "[]",
            )
            .await
            .unwrap();

        assert!(seq_user < seq_delegate);
        assert!(seq_delegate < seq_target);
        assert!(seq_target < seq_source);

        // Upsert same message_id keeps message_seq, updates body + reply metadata.
        let seq_again = store
            .upsert_conversation_message(
                "c",
                "agent-result:c:t-b:k1",
                Some("t-b"),
                "agent",
                Some("local-rt-claude"),
                "@codex#ta done (revised)",
                999,
                Some("mcp-delegation:c:1"),
                Some("del-1"),
                "[]",
            )
            .await
            .unwrap();
        assert_eq!(seq_again, seq_target);

        // List is newest-first (DESC); clients reverse to ASC for display.
        let desc = store
            .list_conversation_messages("c", None, Some(50))
            .await
            .unwrap();
        let ids_desc: Vec<&str> = desc.iter().map(|r| r.message_id.as_str()).collect();
        assert_eq!(
            ids_desc,
            vec![
                "agent-result:c:t-a:k2",
                "agent-result:c:t-b:k1",
                "mcp-delegation:c:1",
                "user-1",
            ]
        );
        assert_eq!(desc[1].body, "@codex#ta done (revised)");
        assert_eq!(
            desc[1].reply_to_message_id.as_deref(),
            Some("mcp-delegation:c:1")
        );

        let mut asc = desc;
        asc.reverse();
        let seqs: Vec<i64> = asc.iter().map(|r| r.message_seq).collect();
        assert!(seqs.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(asc[0].message_id, "user-1");
        assert_eq!(
            asc[2].reply_to_message_id.as_deref(),
            Some("mcp-delegation:c:1")
        );
    }

    #[tokio::test]
    async fn update_session_status_persists_runtime_state() {
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
        seed_conversation(&store).await;
        sqlx::query("INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES ('thr-status', 'c', '/w', 'codex', 'idle', 0, 1, 1)")
            .execute(store.pool())
            .await
            .unwrap();

        let updated = store
            .update_session_status("thr-status", "running", None, None, None, 42)
            .await
            .unwrap();
        assert_eq!(updated, 1);

        let row = store.get_session("thr-status").await.unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert_eq!(row.last_activity_at, 42);
        assert!(row.last_pause_reason.is_none());
        assert!(row.last_close_reason.is_none());
        assert!(row.ended_at.is_none());
    }

    #[tokio::test]
    async fn delete_session_removes_thread_and_events() {
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
        seed_conversation(&store).await;
        sqlx::query("INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES ('thr-delete', 'c', '/w', 'codex', 'idle', 1, 0, 0)")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO events(session_id, seq, body_kind, body_inline, projection_json, ts_ms, source) VALUES ('thr-delete', 1, 'inline', ?, ?, 0, 'live')",
        )
        .bind(br#"{"method":"item/started"}"#.as_slice())
        .bind(b"[]".as_slice())
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ingest_sync_state( \
                session_id, backend_acked_seq, dirty_from_seq, dirty_to_seq, \
                dirty_bytes, dirty_events, updated_at \
             ) VALUES ('thr-delete', 0, 1, 1, 1, 1, 0)",
        )
        .execute(store.pool())
        .await
        .unwrap();

        let deleted = store.delete_session("thr-delete").await.unwrap();

        assert_eq!(deleted, 1);
        assert!(store.get_session("thr-delete").await.unwrap().is_none());
        let events: (i64,) = sqlx::query_as("SELECT count(*) FROM events WHERE session_id = ?")
            .bind("thr-delete")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(events.0, 0);
        let sync_rows: (i64,) =
            sqlx::query_as("SELECT count(*) FROM ingest_sync_state WHERE session_id = ?")
                .bind("thr-delete")
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(sync_rows.0, 0);
    }

    #[tokio::test]
    async fn list_sessions_filters_agent() {
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
        seed_conversation(&store).await;
        for (session_id, agent, ts) in [("thr-a", "codex", 10), ("thr-b", "claude", 20)] {
            sqlx::query("INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) VALUES (?, 'c', '/w', ?, 'idle', 0, ?, ?)")
                .bind(session_id)
                .bind(agent)
                .bind(ts)
                .bind(ts)
                .execute(store.pool())
                .await
                .unwrap();
        }

        let sessions = store
            .list_sessions(None, None, Some("claude"))
            .await
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "thr-b");
    }

    #[tokio::test]
    async fn toggle_local_message_reaction_add_remove_and_list() {
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
        seed_conversation(&store).await;
        store
            .upsert_conversation_message(
                "c",
                "msg-react",
                None,
                "user",
                None,
                "hello",
                10,
                None,
                None,
                "[]",
            )
            .await
            .unwrap();

        let (cid, added) = store
            .toggle_local_message_reaction("msg-react", "👍", "rx-1", 20)
            .await
            .unwrap();
        assert_eq!(cid, "c");
        assert!(added);

        let rows = store
            .list_reactions_for_messages(&["msg-react".into()])
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].emoji, "👍");
        assert_eq!(rows[0].actor_id, minos_protocol::LOCAL_REACTION_ACTOR_ID);

        let (cid2, added2) = store
            .toggle_local_message_reaction("msg-react", "👍", "rx-2", 30)
            .await
            .unwrap();
        assert_eq!(cid2, "c");
        assert!(!added2);
        let rows2 = store
            .list_reactions_for_messages(&["msg-react".into()])
            .await
            .unwrap();
        assert!(rows2.is_empty());
    }

    #[tokio::test]
    async fn toggle_local_message_reaction_concurrent_double_add_is_idempotent() {
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
        seed_conversation(&store).await;
        store
            .upsert_conversation_message(
                "c", "msg-race", None, "user", None, "hello", 10, None, None, "[]",
            )
            .await
            .unwrap();

        let store_a = store.clone();
        let store_b = store.clone();
        let (a, b) = tokio::join!(
            store_a.toggle_local_message_reaction("msg-race", "🎉", "rx-a", 20),
            store_b.toggle_local_message_reaction("msg-race", "🎉", "rx-b", 21),
        );
        let (cid_a, added_a) = a.unwrap();
        let (cid_b, added_b) = b.unwrap();
        assert_eq!(cid_a, "c");
        assert_eq!(cid_b, "c");
        // One add + one remove (second toggle sees the row) — never unique-violation.
        assert_ne!(added_a, added_b);
        let rows = store
            .list_reactions_for_messages(&["msg-race".into()])
            .await
            .unwrap();
        assert!(rows.len() <= 1);
    }
}
