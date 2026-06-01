//! `notification_preferences` table CRUD. One row per account, created
//! on first access with sensible defaults.

use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NotificationPreferencesRow {
    pub account_id: String,
    pub direct_message_enabled: bool,
    pub group_mention_enabled: bool,
    pub approval_required_enabled: bool,
    pub agent_session_ended_enabled: bool,
    pub quiet_hours_start_minute: Option<i16>,
    pub quiet_hours_end_minute: Option<i16>,
    pub quiet_hours_timezone: Option<String>,
    pub updated_at_ms: i64,
}

/// Get preferences for an account. Returns defaults if no row exists.
pub async fn get<S>(
    store: &S,
    account_id: &str,
) -> Result<NotificationPreferencesRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => get_sqlite(pool, account_id).await,
        StorePoolRef::Postgres(pool) => get_postgres(pool, account_id).await,
    }
}

/// Upsert preferences for an account.
pub async fn upsert<S>(
    store: &S,
    account_id: &str,
    direct_message_enabled: bool,
    group_mention_enabled: bool,
    approval_required_enabled: bool,
    agent_session_ended_enabled: bool,
    quiet_hours_start_minute: Option<i16>,
    quiet_hours_end_minute: Option<i16>,
    quiet_hours_timezone: Option<&str>,
    at_ms: i64,
) -> Result<NotificationPreferencesRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            upsert_sqlite(
                pool,
                account_id,
                direct_message_enabled,
                group_mention_enabled,
                approval_required_enabled,
                agent_session_ended_enabled,
                quiet_hours_start_minute,
                quiet_hours_end_minute,
                quiet_hours_timezone,
                at_ms,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            upsert_postgres(
                pool,
                account_id,
                direct_message_enabled,
                group_mention_enabled,
                approval_required_enabled,
                agent_session_ended_enabled,
                quiet_hours_start_minute,
                quiet_hours_end_minute,
                quiet_hours_timezone,
                at_ms,
            )
            .await
        }
    }
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn get_sqlite(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<NotificationPreferencesRow, BackendError> {
    let row = sqlx::query_as::<_, NotificationPreferencesRow>(
        "SELECT * FROM notification_preferences WHERE account_id = ?1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_preferences.get".into(),
        message: e.to_string(),
    })?;

    Ok(row.unwrap_or_else(|| default_preferences(account_id)))
}

async fn upsert_sqlite(
    pool: &SqlitePool,
    account_id: &str,
    direct_message_enabled: bool,
    group_mention_enabled: bool,
    approval_required_enabled: bool,
    agent_session_ended_enabled: bool,
    quiet_hours_start_minute: Option<i16>,
    quiet_hours_end_minute: Option<i16>,
    quiet_hours_timezone: Option<&str>,
    at_ms: i64,
) -> Result<NotificationPreferencesRow, BackendError> {
    sqlx::query_as::<_, NotificationPreferencesRow>(
        "INSERT INTO notification_preferences
             (account_id, direct_message_enabled, group_mention_enabled,
              approval_required_enabled, agent_session_ended_enabled,
              quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_timezone, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(account_id) DO UPDATE SET
             direct_message_enabled = excluded.direct_message_enabled,
             group_mention_enabled = excluded.group_mention_enabled,
             approval_required_enabled = excluded.approval_required_enabled,
             agent_session_ended_enabled = excluded.agent_session_ended_enabled,
             quiet_hours_start_minute = excluded.quiet_hours_start_minute,
             quiet_hours_end_minute = excluded.quiet_hours_end_minute,
             quiet_hours_timezone = excluded.quiet_hours_timezone,
             updated_at_ms = excluded.updated_at_ms
         RETURNING *",
    )
    .bind(account_id)
    .bind(direct_message_enabled)
    .bind(group_mention_enabled)
    .bind(approval_required_enabled)
    .bind(agent_session_ended_enabled)
    .bind(quiet_hours_start_minute)
    .bind(quiet_hours_end_minute)
    .bind(quiet_hours_timezone)
    .bind(at_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_preferences.upsert".into(),
        message: e.to_string(),
    })
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn get_postgres(
    pool: &PgPool,
    account_id: &str,
) -> Result<NotificationPreferencesRow, BackendError> {
    let row = sqlx::query_as::<_, NotificationPreferencesRow>(
        "SELECT * FROM notification_preferences WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_preferences.get".into(),
        message: e.to_string(),
    })?;

    Ok(row.unwrap_or_else(|| default_preferences(account_id)))
}

async fn upsert_postgres(
    pool: &PgPool,
    account_id: &str,
    direct_message_enabled: bool,
    group_mention_enabled: bool,
    approval_required_enabled: bool,
    agent_session_ended_enabled: bool,
    quiet_hours_start_minute: Option<i16>,
    quiet_hours_end_minute: Option<i16>,
    quiet_hours_timezone: Option<&str>,
    at_ms: i64,
) -> Result<NotificationPreferencesRow, BackendError> {
    sqlx::query_as::<_, NotificationPreferencesRow>(
        "INSERT INTO notification_preferences
             (account_id, direct_message_enabled, group_mention_enabled,
              approval_required_enabled, agent_session_ended_enabled,
              quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_timezone, updated_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT(account_id) DO UPDATE SET
             direct_message_enabled = EXCLUDED.direct_message_enabled,
             group_mention_enabled = EXCLUDED.group_mention_enabled,
             approval_required_enabled = EXCLUDED.approval_required_enabled,
             agent_session_ended_enabled = EXCLUDED.agent_session_ended_enabled,
             quiet_hours_start_minute = EXCLUDED.quiet_hours_start_minute,
             quiet_hours_end_minute = EXCLUDED.quiet_hours_end_minute,
             quiet_hours_timezone = EXCLUDED.quiet_hours_timezone,
             updated_at_ms = EXCLUDED.updated_at_ms
         RETURNING *",
    )
    .bind(account_id)
    .bind(direct_message_enabled)
    .bind(group_mention_enabled)
    .bind(approval_required_enabled)
    .bind(agent_session_ended_enabled)
    .bind(quiet_hours_start_minute)
    .bind(quiet_hours_end_minute)
    .bind(quiet_hours_timezone)
    .bind(at_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_preferences.upsert".into(),
        message: e.to_string(),
    })
}

fn default_preferences(account_id: &str) -> NotificationPreferencesRow {
    NotificationPreferencesRow {
        account_id: account_id.to_string(),
        direct_message_enabled: true,
        group_mention_enabled: true,
        approval_required_enabled: true,
        agent_session_ended_enabled: false,
        quiet_hours_start_minute: None,
        quiet_hours_end_minute: None,
        quiet_hours_timezone: None,
        updated_at_ms: 0,
    }
}
