//! `notification_cooldowns` table CRUD. Tracks the last time a notification
//! was sent to an account for a given cooldown key, enabling rate-limiting
//! of repeated notifications (e.g. same conversation, same approval).

use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NotificationCooldownRow {
    pub account_id: String,
    pub cooldown_key: String,
    pub last_sent_at_ms: i64,
}

/// Read-only: returns `true` if cooldown has expired or never recorded.
/// Does **not** stamp `last_sent_at_ms` — call [`record_sent`] only after true delivery.
pub async fn is_allowed<S>(
    store: &S,
    account_id: &str,
    cooldown_key: &str,
    cooldown_ms: i64,
    now_ms: i64,
) -> Result<bool, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            is_allowed_sqlite(pool, account_id, cooldown_key, cooldown_ms, now_ms).await
        }
        StorePoolRef::Postgres(pool) => {
            is_allowed_postgres(pool, account_id, cooldown_key, cooldown_ms, now_ms).await
        }
    }
}

/// Stamp cooldown after a successful push send.
pub async fn record_sent<S>(
    store: &S,
    account_id: &str,
    cooldown_key: &str,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => record_sent_sqlite(pool, account_id, cooldown_key, now_ms).await,
        StorePoolRef::Postgres(pool) => {
            record_sent_postgres(pool, account_id, cooldown_key, now_ms).await
        }
    }
}

/// Deprecated combined API: check + stamp. Prefer [`is_allowed`] + [`record_sent`].
pub async fn check_and_update<S>(
    store: &S,
    account_id: &str,
    cooldown_key: &str,
    cooldown_ms: i64,
    now_ms: i64,
) -> Result<bool, BackendError>
where
    S: AsStorePool + ?Sized,
{
    if !is_allowed(store, account_id, cooldown_key, cooldown_ms, now_ms).await? {
        return Ok(false);
    }
    record_sent(store, account_id, cooldown_key, now_ms).await?;
    Ok(true)
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn is_allowed_sqlite(
    pool: &SqlitePool,
    account_id: &str,
    cooldown_key: &str,
    cooldown_ms: i64,
    now_ms: i64,
) -> Result<bool, BackendError> {
    let existing = sqlx::query_as::<_, NotificationCooldownRow>(
        "SELECT * FROM notification_cooldowns WHERE account_id = ?1 AND cooldown_key = ?2",
    )
    .bind(account_id)
    .bind(cooldown_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_cooldowns.check".into(),
        message: e.to_string(),
    })?;

    if let Some(row) = existing {
        if now_ms - row.last_sent_at_ms < cooldown_ms {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn record_sent_sqlite(
    pool: &SqlitePool,
    account_id: &str,
    cooldown_key: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO notification_cooldowns (account_id, cooldown_key, last_sent_at_ms)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(account_id, cooldown_key) DO UPDATE SET last_sent_at_ms = excluded.last_sent_at_ms",
    )
    .bind(account_id)
    .bind(cooldown_key)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_cooldowns.upsert".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn is_allowed_postgres(
    pool: &PgPool,
    account_id: &str,
    cooldown_key: &str,
    cooldown_ms: i64,
    now_ms: i64,
) -> Result<bool, BackendError> {
    let existing = sqlx::query_as::<_, NotificationCooldownRow>(
        "SELECT * FROM notification_cooldowns WHERE account_id = $1 AND cooldown_key = $2",
    )
    .bind(account_id)
    .bind(cooldown_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_cooldowns.check".into(),
        message: e.to_string(),
    })?;

    if let Some(row) = existing {
        if now_ms - row.last_sent_at_ms < cooldown_ms {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn record_sent_postgres(
    pool: &PgPool,
    account_id: &str,
    cooldown_key: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO notification_cooldowns (account_id, cooldown_key, last_sent_at_ms)
         VALUES ($1, $2, $3)
         ON CONFLICT(account_id, cooldown_key) DO UPDATE SET last_sent_at_ms = EXCLUDED.last_sent_at_ms",
    )
    .bind(account_id)
    .bind(cooldown_key)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "notification_cooldowns.upsert".into(),
        message: e.to_string(),
    })?;
    Ok(())
}
