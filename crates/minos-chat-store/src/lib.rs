use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

pub mod mcp_handler;
pub mod mcp_server;
pub mod mcp_socket;
pub mod teamwork_mcp;

const OPEN_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
];

#[derive(Debug, Clone)]
pub struct TeamworkStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamworkDelegationStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamworkDelegation {
    pub delegation_id: String,
    pub conversation_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub status: TeamworkDelegationStatus,
    pub source_agent: Option<AgentName>,
    pub source_session_id: Option<String>,
    pub target_agent: AgentName,
    pub prompt: String,
    pub session_id: Option<String>,
    pub request_message_id: Option<String>,
    pub result_message_id: Option<String>,
    pub result_text: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamworkSourceDeliveryStatus {
    Pending,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamworkSourceDelivery {
    pub delivery_id: String,
    pub conversation_id: String,
    pub delegation_id: String,
    pub source_session_id: String,
    pub body: String,
    pub status: TeamworkSourceDeliveryStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl TeamworkStore {
    pub async fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!(
                    "failed to create teamwork db parent dir {}",
                    parent.display()
                )
            })?;
        }

        for (attempt, delay) in OPEN_RETRY_DELAYS.iter().enumerate() {
            match Self::open_once(db_path).await {
                Ok(store) => return Ok(store),
                Err(error) if is_sqlite_busy_error(&error) => {
                    if attempt == OPEN_RETRY_DELAYS.len() - 1 {
                        return Err(error);
                    }
                    tokio::time::sleep(*delay).await;
                }
                Err(error) => return Err(error),
            }
        }

        Self::open_once(db_path).await
    }

    async fn open_once(db_path: &Path) -> Result<Self> {
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn ensure_conversation(
        &self,
        conversation_id: &str,
        title: &str,
        workspace_root: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO teamwork_conversations( \
                conversation_id, title, workspace_root, created_at_ms, updated_at_ms \
             ) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(conversation_id) DO UPDATE SET \
                title = excluded.title, \
                workspace_root = excluded.workspace_root, \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(conversation_id)
        .bind(title)
        .bind(workspace_root)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_delegation(
        &self,
        conversation_id: &str,
        source_agent: Option<AgentName>,
        source_session_id: Option<String>,
        target_agent: AgentName,
        prompt: String,
        session_id: Option<String>,
    ) -> Result<TeamworkDelegation> {
        let prompt = prompt.trim().to_owned();
        anyhow::ensure!(!prompt.is_empty(), "delegation prompt must not be empty");
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let seq_i64: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(delegation_seq), 0) + 1 FROM teamwork_delegations",
        )
        .fetch_one(&mut *tx)
        .await?;
        let delegation_id = format!("delegation-{seq_i64}");
        let status = TeamworkDelegationStatus::Running;
        sqlx::query(
            "INSERT INTO teamwork_delegations( \
                delegation_seq, delegation_id, conversation_id, created_at_ms, updated_at_ms, \
                status, source_agent, source_session_id, target_agent, prompt, session_id, \
                request_message_id, result_message_id, result_text, error \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(seq_i64)
        .bind(&delegation_id)
        .bind(conversation_id)
        .bind(now)
        .bind(now)
        .bind(status.as_db())
        .bind(source_agent.map(|agent| agent.bin_name().to_owned()))
        .bind(source_session_id.as_deref())
        .bind(target_agent.bin_name())
        .bind(&prompt)
        .bind(session_id.as_deref())
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(TeamworkDelegation {
            delegation_id,
            conversation_id: conversation_id.to_owned(),
            created_at_ms: now,
            updated_at_ms: now,
            status,
            source_agent,
            source_session_id,
            target_agent,
            prompt,
            session_id,
            request_message_id: None,
            result_message_id: None,
            result_text: None,
            error: None,
        })
    }

    pub async fn set_delegation_request_message_id(
        &self,
        conversation_id: &str,
        delegation_id: &str,
        request_message_id: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let result = sqlx::query(
            "UPDATE teamwork_delegations \
             SET request_message_id = ?, updated_at_ms = ? \
             WHERE conversation_id = ? AND delegation_id = ?",
        )
        .bind(request_message_id)
        .bind(now)
        .bind(conversation_id)
        .bind(delegation_id)
        .execute(&self.pool)
        .await?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "delegation not found: {delegation_id}"
        );
        Ok(())
    }

    pub async fn enqueue_source_delivery(
        &self,
        conversation_id: &str,
        delegation_id: &str,
        source_session_id: &str,
        body: &str,
    ) -> Result<TeamworkSourceDelivery> {
        let now = chrono::Utc::now().timestamp_millis();
        let delivery_id = format!("delivery-{}-{}", delegation_id, now);
        let status = TeamworkSourceDeliveryStatus::Pending;
        sqlx::query(
            "INSERT INTO teamwork_source_deliveries( \
                delivery_id, conversation_id, delegation_id, source_session_id, body, \
                status, attempts, last_error, created_at_ms, updated_at_ms \
             ) VALUES (?, ?, ?, ?, ?, ?, 0, NULL, ?, ?)",
        )
        .bind(&delivery_id)
        .bind(conversation_id)
        .bind(delegation_id)
        .bind(source_session_id)
        .bind(body)
        .bind(status.as_db())
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(TeamworkSourceDelivery {
            delivery_id,
            conversation_id: conversation_id.to_owned(),
            delegation_id: delegation_id.to_owned(),
            source_session_id: source_session_id.to_owned(),
            body: body.to_owned(),
            status,
            attempts: 0,
            last_error: None,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub async fn list_pending_source_deliveries_for_thread(
        &self,
        source_session_id: &str,
    ) -> Result<Vec<TeamworkSourceDelivery>> {
        let rows = sqlx::query(
            "SELECT * FROM teamwork_source_deliveries \
             WHERE source_session_id = ? AND status = ? \
             ORDER BY created_at_ms ASC",
        )
        .bind(source_session_id)
        .bind(TeamworkSourceDeliveryStatus::Pending.as_db())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(source_delivery_from_row).collect()
    }

    pub async fn mark_source_delivery(
        &self,
        delivery_id: &str,
        status: TeamworkSourceDeliveryStatus,
        last_error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE teamwork_source_deliveries \
             SET status = ?, last_error = ?, attempts = attempts + 1, updated_at_ms = ? \
             WHERE delivery_id = ?",
        )
        .bind(status.as_db())
        .bind(last_error)
        .bind(now)
        .bind(delivery_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn latest_source_delivery_for_delegation(
        &self,
        conversation_id: &str,
        delegation_id: &str,
    ) -> Result<Option<TeamworkSourceDelivery>> {
        let row = sqlx::query(
            "SELECT * FROM teamwork_source_deliveries \
             WHERE conversation_id = ? AND delegation_id = ? \
             ORDER BY created_at_ms DESC LIMIT 1",
        )
        .bind(conversation_id)
        .bind(delegation_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(source_delivery_from_row).transpose()
    }

    /// Block until the delegation reaches a terminal status or timeout.
    pub async fn wait_delegation(
        &self,
        conversation_id: &str,
        delegation_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<(TeamworkDelegation, bool)> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let Some(delegation) = self.get_delegation(conversation_id, delegation_id).await?
            else {
                anyhow::bail!("delegation not found: {delegation_id}");
            };
            if delegation.status.is_terminal() {
                return Ok((delegation, false));
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok((delegation, true));
            }
            let sleep_for = poll_interval.min(deadline.saturating_duration_since(now));
            tokio::time::sleep(sleep_for).await;
        }
    }

    pub async fn running_delegation_for_thread(
        &self,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<Option<TeamworkDelegation>> {
        self.latest_delegation_for_thread(
            conversation_id,
            session_id,
            Some(TeamworkDelegationStatus::Running),
        )
        .await
    }

    pub async fn ensure_delegate_target_allowed(
        &self,
        conversation_id: &str,
        source_session_id: Option<&str>,
        target_agent: AgentName,
    ) -> Result<()> {
        let Some(source_session_id) = source_session_id else {
            return Ok(());
        };
        let Some(source_delegation) = self
            .latest_delegation_for_thread(conversation_id, source_session_id, None)
            .await?
        else {
            return Ok(());
        };
        let Some(original_agent) = source_delegation.source_agent else {
            anyhow::bail!(
                "delegate_to_agent depth limit reached: delegated thread {source_session_id} cannot delegate again"
            );
        };
        anyhow::ensure!(
            target_agent == original_agent,
            "delegate_to_agent depth limit reached: delegated thread {source_session_id} may only delegate back to {}, not {}",
            original_agent.bin_name(),
            target_agent.bin_name()
        );
        Ok(())
    }

    pub async fn complete_delegation_for_thread(
        &self,
        conversation_id: &str,
        session_id: &str,
        result_message_id: Option<&str>,
        result_text: &str,
    ) -> Result<Option<TeamworkDelegation>> {
        let Some(delegation) = self
            .running_delegation_for_thread(conversation_id, session_id)
            .await?
        else {
            return Ok(None);
        };
        let result_text = result_text.trim();
        anyhow::ensure!(
            !result_text.is_empty(),
            "delegation result text must not be empty"
        );
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE teamwork_delegations \
             SET status = ?, updated_at_ms = ?, result_message_id = ?, result_text = ?, error = NULL \
             WHERE conversation_id = ? AND delegation_id = ? AND status = ?",
        )
        .bind(TeamworkDelegationStatus::Completed.as_db())
        .bind(now)
        .bind(result_message_id)
        .bind(result_text)
        .bind(conversation_id)
        .bind(&delegation.delegation_id)
        .bind(TeamworkDelegationStatus::Running.as_db())
        .execute(&self.pool)
        .await?;
        self.get_delegation(conversation_id, &delegation.delegation_id)
            .await
    }

    pub async fn get_delegation(
        &self,
        conversation_id: &str,
        delegation_id: &str,
    ) -> Result<Option<TeamworkDelegation>> {
        let row = sqlx::query(
            "SELECT * FROM teamwork_delegations \
             WHERE conversation_id = ? AND delegation_id = ?",
        )
        .bind(conversation_id)
        .bind(delegation_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(delegation_from_row).transpose()
    }

    async fn latest_delegation_for_thread(
        &self,
        conversation_id: &str,
        session_id: &str,
        status: Option<TeamworkDelegationStatus>,
    ) -> Result<Option<TeamworkDelegation>> {
        let row = if let Some(status) = status {
            sqlx::query(
                "SELECT * FROM teamwork_delegations \
                 WHERE conversation_id = ? AND session_id = ? AND status = ? \
                 ORDER BY delegation_seq DESC LIMIT 1",
            )
            .bind(conversation_id)
            .bind(session_id)
            .bind(status.as_db())
            .fetch_optional(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT * FROM teamwork_delegations \
                 WHERE conversation_id = ? AND session_id = ? \
                 ORDER BY delegation_seq DESC LIMIT 1",
            )
            .bind(conversation_id)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?
        };
        row.map(delegation_from_row).transpose()
    }

    pub async fn cancel_delegation(
        &self,
        conversation_id: &str,
        delegation_id: &str,
        reason: Option<String>,
    ) -> Result<TeamworkDelegation> {
        let current = self
            .get_delegation(conversation_id, delegation_id)
            .await?
            .with_context(|| format!("delegation not found: {delegation_id}"))?;
        anyhow::ensure!(
            !current.status.is_terminal(),
            "delegation {delegation_id} is already {:?}",
            current.status
        );
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE teamwork_delegations \
             SET status = ?, updated_at_ms = ?, error = ? \
             WHERE conversation_id = ? AND delegation_id = ?",
        )
        .bind(TeamworkDelegationStatus::Cancelled.as_db())
        .bind(now)
        .bind(reason.as_deref())
        .bind(conversation_id)
        .bind(delegation_id)
        .execute(&self.pool)
        .await?;
        self.get_delegation(conversation_id, delegation_id)
            .await?
            .with_context(|| format!("delegation disappeared after cancel: {delegation_id}"))
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS teamwork_conversations ( \
                conversation_id TEXT PRIMARY KEY, \
                title TEXT NOT NULL, \
                workspace_root TEXT NOT NULL, \
                created_at_ms INTEGER NOT NULL, \
                updated_at_ms INTEGER NOT NULL \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS teamwork_delegations ( \
                delegation_seq INTEGER PRIMARY KEY, \
                delegation_id TEXT NOT NULL UNIQUE, \
                conversation_id TEXT NOT NULL REFERENCES teamwork_conversations(conversation_id), \
                created_at_ms INTEGER NOT NULL, \
                updated_at_ms INTEGER NOT NULL, \
                status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'cancelled', 'failed')), \
                source_agent TEXT, \
                source_session_id TEXT, \
                target_agent TEXT NOT NULL, \
                prompt TEXT NOT NULL, \
                session_id TEXT, \
                request_message_id TEXT, \
                result_message_id TEXT, \
                result_text TEXT, \
                error TEXT \
             )",
        )
        .execute(pool)
        .await?;

        // Latest-only additive upgrades for existing DBs that already created the
        // table without newer columns (CREATE IF NOT EXISTS does not alter shape).
        // Ignore "duplicate column" errors from already-upgraded databases.
        for ddl in [
            "ALTER TABLE teamwork_delegations ADD COLUMN source_session_id TEXT",
            "ALTER TABLE teamwork_delegations ADD COLUMN request_message_id TEXT",
            "ALTER TABLE teamwork_delegations ADD COLUMN result_message_id TEXT",
            "ALTER TABLE teamwork_delegations ADD COLUMN result_text TEXT",
        ] {
            let _ = sqlx::query(ddl).execute(pool).await;
        }

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS teamwork_delegations_by_conversation_status \
             ON teamwork_delegations(conversation_id, status, delegation_seq DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS teamwork_source_deliveries ( \
                delivery_id TEXT PRIMARY KEY, \
                conversation_id TEXT NOT NULL, \
                delegation_id TEXT NOT NULL, \
                source_session_id TEXT NOT NULL, \
                body TEXT NOT NULL, \
                status TEXT NOT NULL CHECK(status IN ('pending', 'delivered', 'failed')), \
                attempts INTEGER NOT NULL DEFAULT 0, \
                last_error TEXT, \
                created_at_ms INTEGER NOT NULL, \
                updated_at_ms INTEGER NOT NULL \
             )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS teamwork_source_deliveries_by_source_pending \
             ON teamwork_source_deliveries(source_session_id, status, created_at_ms)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}

impl TeamworkDelegationStatus {
    const fn as_db(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown teamwork delegation status: {other}"),
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

impl TeamworkSourceDeliveryStatus {
    const fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "delivered" => Ok(Self::Delivered),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown teamwork source delivery status: {other}"),
        }
    }
}

pub fn default_db_path() -> Result<PathBuf> {
    Ok(resolve_minos_home()?.join("daemon.sqlite"))
}

fn is_sqlite_busy_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("SQLITE_BUSY")
            || message.contains("SQLITE_LOCKED")
            || message.contains("(code: 5)")
            || message.contains("(code: 6)")
    })
}

fn resolve_minos_home() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MINOS_HOME") {
        return Ok(path.into());
    }
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home).join(".minos"));
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(user_profile).join(".minos"));
    }
    let home_drive = std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty());
    let home_path = std::env::var_os("HOMEPATH").filter(|value| !value.is_empty());
    if let (Some(drive), Some(path)) = (home_drive, home_path) {
        return Ok(PathBuf::from(drive).join(path).join(".minos"));
    }
    anyhow::bail!("unable to resolve MINOS_HOME from environment")
}

fn delegation_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TeamworkDelegation> {
    let status: String = row.try_get("status")?;
    let target_agent: String = row.try_get("target_agent")?;
    Ok(TeamworkDelegation {
        delegation_id: row.try_get("delegation_id")?,
        conversation_id: row.try_get("conversation_id")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
        status: TeamworkDelegationStatus::from_db(&status)?,
        source_agent: parse_agent(row.try_get::<Option<String>, _>("source_agent")?)?,
        source_session_id: row.try_get("source_session_id")?,
        target_agent: parse_agent(Some(target_agent))?
            .context("teamwork_delegations.target_agent is NULL")?,
        prompt: row.try_get("prompt")?,
        session_id: row.try_get("session_id")?,
        request_message_id: row.try_get("request_message_id").unwrap_or(None),
        result_message_id: row.try_get("result_message_id")?,
        result_text: row.try_get("result_text")?,
        error: row.try_get("error")?,
    })
}

fn source_delivery_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TeamworkSourceDelivery> {
    let status: String = row.try_get("status")?;
    Ok(TeamworkSourceDelivery {
        delivery_id: row.try_get("delivery_id")?,
        conversation_id: row.try_get("conversation_id")?,
        delegation_id: row.try_get("delegation_id")?,
        source_session_id: row.try_get("source_session_id")?,
        body: row.try_get("body")?,
        status: TeamworkSourceDeliveryStatus::from_db(&status)?,
        attempts: row.try_get("attempts")?,
        last_error: row.try_get("last_error")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn parse_agent(value: Option<String>) -> Result<Option<AgentName>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value.as_str() {
        "codex" => Ok(Some(AgentName::Codex)),
        "claude" => Ok(Some(AgentName::Claude)),
        "gemini" => Ok(Some(AgentName::Gemini)),
        "opencode" => Ok(Some(AgentName::Opencode)),
        "grok" => Ok(Some(AgentName::Grok)),
        other => anyhow::bail!("unknown agent in teamwork store: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delegation_can_be_created_read_and_cancelled() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TeamworkStore::open(&tmp.path().join("teamwork.sqlite"))
            .await
            .unwrap();
        store
            .ensure_conversation("conversation-main", "main", "/tmp/ws")
            .await
            .unwrap();

        let delegation = store
            .create_delegation(
                "conversation-main",
                Some(AgentName::Codex),
                Some("thread-codex".into()),
                AgentName::Gemini,
                "check the failing test".into(),
                Some("thread-gemini".into()),
            )
            .await
            .unwrap();

        assert_eq!(delegation.delegation_id, "delegation-1");
        assert_eq!(delegation.conversation_id, "conversation-main");
        assert_eq!(delegation.status, TeamworkDelegationStatus::Running);
        assert_eq!(
            delegation.source_session_id.as_deref(),
            Some("thread-codex")
        );
        assert_eq!(
            store
                .get_delegation("conversation-main", &delegation.delegation_id)
                .await
                .unwrap()
                .unwrap()
                .target_agent,
            AgentName::Gemini
        );

        let cancelled = store
            .cancel_delegation(
                "conversation-main",
                &delegation.delegation_id,
                Some("no longer needed".into()),
            )
            .await
            .unwrap();
        assert_eq!(cancelled.status, TeamworkDelegationStatus::Cancelled);
        assert_eq!(cancelled.error.as_deref(), Some("no longer needed"));
    }

    #[tokio::test]
    async fn delegation_can_be_completed_by_target_session() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TeamworkStore::open(&tmp.path().join("teamwork.sqlite"))
            .await
            .unwrap();
        store
            .ensure_conversation("conversation-main", "main", "/tmp/ws")
            .await
            .unwrap();
        store
            .create_delegation(
                "conversation-main",
                Some(AgentName::Opencode),
                Some("thread-opencode".into()),
                AgentName::Codex,
                "say hi".into(),
                Some("thread-codex".into()),
            )
            .await
            .unwrap();

        let completed = store
            .complete_delegation_for_thread(
                "conversation-main",
                "thread-codex",
                Some("message-1"),
                "hi",
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(completed.status, TeamworkDelegationStatus::Completed);
        assert_eq!(completed.result_message_id.as_deref(), Some("message-1"));
        assert_eq!(completed.result_text.as_deref(), Some("hi"));
        assert_eq!(completed.source_agent, Some(AgentName::Opencode));
        assert_eq!(
            completed.source_session_id.as_deref(),
            Some("thread-opencode")
        );
    }

    #[tokio::test]
    async fn migrate_adds_source_session_id_to_legacy_delegations_table() {
        // Simulate a pre-source_session_id schema (matches production daemon.sqlite
        // that only received the request_message_id ALTER).
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("legacy.sqlite");
        {
            let url = format!("sqlite://{}?mode=rwc", path.display());
            let opts = SqliteConnectOptions::from_str(&url).unwrap();
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE teamwork_conversations ( \
                    conversation_id TEXT PRIMARY KEY, \
                    title TEXT NOT NULL, \
                    workspace_root TEXT NOT NULL, \
                    created_at_ms INTEGER NOT NULL, \
                    updated_at_ms INTEGER NOT NULL \
                 )",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "CREATE TABLE teamwork_delegations ( \
                    delegation_seq INTEGER PRIMARY KEY, \
                    delegation_id TEXT NOT NULL UNIQUE, \
                    conversation_id TEXT NOT NULL, \
                    created_at_ms INTEGER NOT NULL, \
                    updated_at_ms INTEGER NOT NULL, \
                    status TEXT NOT NULL, \
                    source_agent TEXT, \
                    target_agent TEXT NOT NULL, \
                    prompt TEXT NOT NULL, \
                    session_id TEXT, \
                    error TEXT, \
                    request_message_id TEXT \
                 )",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }

        let store = TeamworkStore::open(&path).await.unwrap();
        store.ensure_conversation("c1", "c1", "/tmp").await.unwrap();
        let delegation = store
            .create_delegation(
                "c1",
                Some(AgentName::Codex),
                Some("thread-source".into()),
                AgentName::Grok,
                "do the thing".into(),
                Some("thread-target".into()),
            )
            .await
            .expect("INSERT with source_session_id must work after migrate");
        assert_eq!(
            delegation.source_session_id.as_deref(),
            Some("thread-source")
        );
    }

    #[tokio::test]
    async fn delegated_thread_may_only_delegate_back_to_source_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TeamworkStore::open(&tmp.path().join("teamwork.sqlite"))
            .await
            .unwrap();
        store
            .ensure_conversation("conversation-main", "main", "/tmp/ws")
            .await
            .unwrap();
        store
            .create_delegation(
                "conversation-main",
                Some(AgentName::Codex),
                Some("thread-codex".into()),
                AgentName::Opencode,
                "review this".into(),
                Some("thread-opencode".into()),
            )
            .await
            .unwrap();

        store
            .ensure_delegate_target_allowed(
                "conversation-main",
                Some("thread-opencode"),
                AgentName::Codex,
            )
            .await
            .unwrap();

        let error = store
            .ensure_delegate_target_allowed(
                "conversation-main",
                Some("thread-opencode"),
                AgentName::Gemini,
            )
            .await
            .expect_err("third-agent delegation should be blocked");
        assert!(error
            .to_string()
            .contains("may only delegate back to codex"));
    }

    #[tokio::test]
    async fn wait_delegation_times_out_while_running_and_returns_after_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TeamworkStore::open(&tmp.path().join("teamwork.sqlite"))
            .await
            .unwrap();
        store
            .ensure_conversation("conversation-main", "main", "/tmp/ws")
            .await
            .unwrap();
        let delegation = store
            .create_delegation(
                "conversation-main",
                Some(AgentName::Codex),
                Some("thread-codex".into()),
                AgentName::Gemini,
                "work".into(),
                Some("thread-gemini".into()),
            )
            .await
            .unwrap();

        let (running, timed_out) = store
            .wait_delegation(
                "conversation-main",
                &delegation.delegation_id,
                Duration::from_millis(50),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
        assert!(timed_out);
        assert_eq!(running.status, TeamworkDelegationStatus::Running);

        store
            .complete_delegation_for_thread(
                "conversation-main",
                "thread-gemini",
                Some("msg-1"),
                "done",
            )
            .await
            .unwrap();

        let (completed, timed_out) = store
            .wait_delegation(
                "conversation-main",
                &delegation.delegation_id,
                Duration::from_millis(200),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
        assert!(!timed_out);
        assert_eq!(completed.status, TeamworkDelegationStatus::Completed);
        assert_eq!(completed.result_text.as_deref(), Some("done"));
    }
}
