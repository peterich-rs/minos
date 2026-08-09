//! Durable CompletionWatch rows — restart-safe turn projection state.
//!
//! In-memory [`crate::completion_watch::CompletionWatchRegistry`] is a cache
//! hydrated from this table on startup; arm/project/expire keep both in sync.

use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

pub const STATUS_ARMED: &str = "armed";
pub const STATUS_PROJECTED: &str = "projected";
pub const STATUS_EXPIRED: &str = "expired";

#[derive(Debug, Clone)]
pub struct CompletionWatchRow {
    pub watch_key: String,
    pub dispatch_id: String,
    pub origin_message_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub raw_seq_floor: i64,
    pub armed_at_ms: i64,
    pub deadline_at_ms: i64,
    pub status: String,
    pub projected_message_id: Option<String>,
    pub mention_account_id: Option<String>,
    pub mention_minos_id: Option<String>,
}

/// Upsert an armed watch (re-arm replaces the row).
pub async fn upsert_armed<S>(store: &S, row: &CompletionWatchRow) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => upsert_armed_sqlite(pool, row).await,
        StorePoolRef::Postgres(pool) => upsert_armed_postgres(pool, row).await,
    }
}

/// List all currently armed watches (startup hydrate).
pub async fn list_armed<S>(store: &S) -> Result<Vec<CompletionWatchRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => list_by_status_sqlite(pool, STATUS_ARMED).await,
        StorePoolRef::Postgres(pool) => list_by_status_postgres(pool, STATUS_ARMED).await,
    }
}

/// Mark watch projected (success) and optional projected_message_id.
pub async fn mark_projected<S>(
    store: &S,
    watch_key: &str,
    projected_message_id: Option<&str>,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    mark_status(store, watch_key, STATUS_PROJECTED, projected_message_id).await
}

/// Mark watch expired (TTL failure).
pub async fn mark_expired<S>(store: &S, watch_key: &str) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    mark_status(store, watch_key, STATUS_EXPIRED, None).await
}

/// Delete a terminal row (optional GC); prefer status update for audit.
pub async fn delete<S>(store: &S, watch_key: &str) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query("DELETE FROM completion_watches WHERE watch_key = ?1")
                .bind(watch_key)
                .execute(pool)
                .await
                .map_err(store_err("completion_watches::delete"))?;
            Ok(())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query("DELETE FROM completion_watches WHERE watch_key = $1")
                .bind(watch_key)
                .execute(pool)
                .await
                .map_err(store_err("completion_watches::delete"))?;
            Ok(())
        }
    }
}

async fn mark_status<S>(
    store: &S,
    watch_key: &str,
    status: &str,
    projected_message_id: Option<&str>,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "UPDATE completion_watches
                 SET status = ?1, projected_message_id = COALESCE(?2, projected_message_id)
                 WHERE watch_key = ?3",
            )
            .bind(status)
            .bind(projected_message_id)
            .bind(watch_key)
            .execute(pool)
            .await
            .map_err(store_err("completion_watches::mark_status"))?;
            Ok(())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "UPDATE completion_watches
                 SET status = $1, projected_message_id = COALESCE($2, projected_message_id)
                 WHERE watch_key = $3",
            )
            .bind(status)
            .bind(projected_message_id)
            .bind(watch_key)
            .execute(pool)
            .await
            .map_err(store_err("completion_watches::mark_status"))?;
            Ok(())
        }
    }
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn upsert_armed_sqlite(pool: &SqlitePool, row: &CompletionWatchRow) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO completion_watches (
            watch_key, dispatch_id, origin_message_id, conversation_id, session_id,
            agent_id, raw_seq_floor, armed_at_ms, deadline_at_ms, status,
            projected_message_id, mention_account_id, mention_minos_id
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,?11,?12)
         ON CONFLICT(watch_key) DO UPDATE SET
            dispatch_id = excluded.dispatch_id,
            origin_message_id = excluded.origin_message_id,
            conversation_id = excluded.conversation_id,
            session_id = excluded.session_id,
            agent_id = excluded.agent_id,
            raw_seq_floor = excluded.raw_seq_floor,
            armed_at_ms = excluded.armed_at_ms,
            deadline_at_ms = excluded.deadline_at_ms,
            status = excluded.status,
            projected_message_id = NULL,
            mention_account_id = excluded.mention_account_id,
            mention_minos_id = excluded.mention_minos_id",
    )
    .bind(&row.watch_key)
    .bind(&row.dispatch_id)
    .bind(&row.origin_message_id)
    .bind(&row.conversation_id)
    .bind(&row.session_id)
    .bind(&row.agent_id)
    .bind(row.raw_seq_floor)
    .bind(row.armed_at_ms)
    .bind(row.deadline_at_ms)
    .bind(STATUS_ARMED)
    .bind(&row.mention_account_id)
    .bind(&row.mention_minos_id)
    .execute(pool)
    .await
    .map_err(store_err("completion_watches::upsert_armed"))?;
    Ok(())
}

async fn list_by_status_sqlite(
    pool: &SqlitePool,
    status: &str,
) -> Result<Vec<CompletionWatchRow>, BackendError> {
    let rows = sqlx::query_as::<_, CompletionWatchSqlRow>(
        "SELECT watch_key, dispatch_id, origin_message_id, conversation_id, session_id,
                agent_id, raw_seq_floor, armed_at_ms, deadline_at_ms, status,
                projected_message_id, mention_account_id, mention_minos_id
         FROM completion_watches WHERE status = ?1",
    )
    .bind(status)
    .fetch_all(pool)
    .await
    .map_err(store_err("completion_watches::list_armed"))?;
    Ok(rows.into_iter().map(Into::into).collect())
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn upsert_armed_postgres(pool: &PgPool, row: &CompletionWatchRow) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO completion_watches (
            watch_key, dispatch_id, origin_message_id, conversation_id, session_id,
            agent_id, raw_seq_floor, armed_at_ms, deadline_at_ms, status,
            projected_message_id, mention_account_id, mention_minos_id
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,NULL,$11,$12)
         ON CONFLICT(watch_key) DO UPDATE SET
            dispatch_id = EXCLUDED.dispatch_id,
            origin_message_id = EXCLUDED.origin_message_id,
            conversation_id = EXCLUDED.conversation_id,
            session_id = EXCLUDED.session_id,
            agent_id = EXCLUDED.agent_id,
            raw_seq_floor = EXCLUDED.raw_seq_floor,
            armed_at_ms = EXCLUDED.armed_at_ms,
            deadline_at_ms = EXCLUDED.deadline_at_ms,
            status = EXCLUDED.status,
            projected_message_id = NULL,
            mention_account_id = EXCLUDED.mention_account_id,
            mention_minos_id = EXCLUDED.mention_minos_id",
    )
    .bind(&row.watch_key)
    .bind(&row.dispatch_id)
    .bind(&row.origin_message_id)
    .bind(&row.conversation_id)
    .bind(&row.session_id)
    .bind(&row.agent_id)
    .bind(row.raw_seq_floor)
    .bind(row.armed_at_ms)
    .bind(row.deadline_at_ms)
    .bind(STATUS_ARMED)
    .bind(&row.mention_account_id)
    .bind(&row.mention_minos_id)
    .execute(pool)
    .await
    .map_err(store_err("completion_watches::upsert_armed"))?;
    Ok(())
}

async fn list_by_status_postgres(
    pool: &PgPool,
    status: &str,
) -> Result<Vec<CompletionWatchRow>, BackendError> {
    let rows = sqlx::query_as::<_, CompletionWatchSqlRow>(
        "SELECT watch_key, dispatch_id, origin_message_id, conversation_id, session_id,
                agent_id, raw_seq_floor, armed_at_ms, deadline_at_ms, status,
                projected_message_id, mention_account_id, mention_minos_id
         FROM completion_watches WHERE status = $1",
    )
    .bind(status)
    .fetch_all(pool)
    .await
    .map_err(store_err("completion_watches::list_armed"))?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(sqlx::FromRow)]
struct CompletionWatchSqlRow {
    watch_key: String,
    dispatch_id: String,
    origin_message_id: String,
    conversation_id: String,
    session_id: String,
    agent_id: String,
    raw_seq_floor: i64,
    armed_at_ms: i64,
    deadline_at_ms: i64,
    status: String,
    projected_message_id: Option<String>,
    mention_account_id: Option<String>,
    mention_minos_id: Option<String>,
}

impl From<CompletionWatchSqlRow> for CompletionWatchRow {
    fn from(r: CompletionWatchSqlRow) -> Self {
        Self {
            watch_key: r.watch_key,
            dispatch_id: r.dispatch_id,
            origin_message_id: r.origin_message_id,
            conversation_id: r.conversation_id,
            session_id: r.session_id,
            agent_id: r.agent_id,
            raw_seq_floor: r.raw_seq_floor,
            armed_at_ms: r.armed_at_ms,
            deadline_at_ms: r.deadline_at_ms,
            status: r.status,
            projected_message_id: r.projected_message_id,
            mention_account_id: r.mention_account_id,
            mention_minos_id: r.mention_minos_id,
        }
    }
}

fn store_err(op: &'static str) -> impl FnOnce(sqlx::Error) -> BackendError {
    move |e| BackendError::StoreQuery {
        operation: op.into(),
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;

    fn sample(key: &str) -> CompletionWatchRow {
        CompletionWatchRow {
            watch_key: key.into(),
            dispatch_id: "d1".into(),
            origin_message_id: "o1".into(),
            conversation_id: "c1".into(),
            session_id: "s1".into(),
            agent_id: "a1".into(),
            raw_seq_floor: 3,
            armed_at_ms: 100,
            deadline_at_ms: 200,
            status: STATUS_ARMED.into(),
            projected_message_id: None,
            mention_account_id: Some("acc".into()),
            mention_minos_id: None,
        }
    }

    #[tokio::test]
    async fn upsert_list_project_round_trip() {
        let pool = memory_pool().await;
        upsert_armed(&pool, &sample("o1:s1")).await.unwrap();
        let armed = list_armed(&pool).await.unwrap();
        assert_eq!(armed.len(), 1);
        assert_eq!(armed[0].raw_seq_floor, 3);

        // Re-arm updates floor.
        let mut again = sample("o1:s1");
        again.raw_seq_floor = 9;
        upsert_armed(&pool, &again).await.unwrap();
        let armed = list_armed(&pool).await.unwrap();
        assert_eq!(armed.len(), 1);
        assert_eq!(armed[0].raw_seq_floor, 9);

        mark_projected(&pool, "o1:s1", Some("agent-result:x"))
            .await
            .unwrap();
        assert!(list_armed(&pool).await.unwrap().is_empty());
    }
}
