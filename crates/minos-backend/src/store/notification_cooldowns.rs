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

/// Check if a cooldown has expired. Returns `true` if the notification
/// should be sent (cooldown expired or never recorded).
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
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            check_and_update_sqlite(pool, account_id, cooldown_key, cooldown_ms, now_ms).await
        }
        StorePoolRef::Postgres(pool) => {
            check_and_update_postgres(pool, account_id, cooldown_key, cooldown_ms, now_ms).await
        }
    }
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn check_and_update_sqlite(
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
            return Ok(false); // Still in cooldown
        }
    }

    // Upsert the cooldown record
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

    Ok(true)
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn check_and_update_postgres(
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
            return Ok(false); // Still in cooldown
        }
    }

    // Upsert the cooldown record
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

    Ok(true)
}
