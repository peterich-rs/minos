//! `push_dispatch_log` — event-level push idempotency.
//!
//! PK `(event_id, account_id)`: a successful push is recorded once and never
//! re-sent for the same durable event to the same account.

use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

/// Returns true if a successful push was already recorded for this pair.
pub async fn has_sent<S>(
    store: &S,
    event_id: &str,
    account_id: &str,
) -> Result<bool, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => has_sent_sqlite(pool, event_id, account_id).await,
        StorePoolRef::Postgres(pool) => has_sent_postgres(pool, event_id, account_id).await,
    }
}

/// Record a successful push. Idempotent: ON CONFLICT DO NOTHING.
pub async fn record_sent<S>(
    store: &S,
    event_id: &str,
    account_id: &str,
    sent_at_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            record_sent_sqlite(pool, event_id, account_id, sent_at_ms).await
        }
        StorePoolRef::Postgres(pool) => {
            record_sent_postgres(pool, event_id, account_id, sent_at_ms).await
        }
    }
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn has_sent_sqlite(
    pool: &SqlitePool,
    event_id: &str,
    account_id: &str,
) -> Result<bool, BackendError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM push_dispatch_log WHERE event_id = ?1 AND account_id = ?2",
    )
    .bind(event_id)
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_dispatch_log.has_sent".into(),
        message: e.to_string(),
    })?;
    Ok(count > 0)
}

async fn record_sent_sqlite(
    pool: &SqlitePool,
    event_id: &str,
    account_id: &str,
    sent_at_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO push_dispatch_log (event_id, account_id, sent_at_ms)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(event_id, account_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(account_id)
    .bind(sent_at_ms)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_dispatch_log.record_sent".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn has_sent_postgres(
    pool: &PgPool,
    event_id: &str,
    account_id: &str,
) -> Result<bool, BackendError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM push_dispatch_log WHERE event_id = $1 AND account_id = $2",
    )
    .bind(event_id)
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_dispatch_log.has_sent".into(),
        message: e.to_string(),
    })?;
    Ok(count > 0)
}

async fn record_sent_postgres(
    pool: &PgPool,
    event_id: &str,
    account_id: &str,
    sent_at_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO push_dispatch_log (event_id, account_id, sent_at_ms)
         VALUES ($1, $2, $3)
         ON CONFLICT(event_id, account_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(account_id)
    .bind(sent_at_ms)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_dispatch_log.record_sent".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, memory_pool};

    #[tokio::test]
    async fn record_and_has_sent_round_trip() {
        let pool = memory_pool().await;
        let acc1 = insert_account(&pool, "a1@example.com").await;
        let acc2 = insert_account(&pool, "a2@example.com").await;
        assert!(!has_sent(&pool, "ev-1", &acc1).await.unwrap());
        record_sent(&pool, "ev-1", &acc1, 1_000).await.unwrap();
        assert!(has_sent(&pool, "ev-1", &acc1).await.unwrap());
        // Different account not marked
        assert!(!has_sent(&pool, "ev-1", &acc2).await.unwrap());
        // Idempotent re-insert
        record_sent(&pool, "ev-1", &acc1, 2_000).await.unwrap();
        assert!(has_sent(&pool, "ev-1", &acc1).await.unwrap());
    }
}
