//! Durable push work queue — claim/retry/backoff for push delivery.
//!
//! Success ledger remains [`super::push_dispatch_log`]. Queue rows track work
//! state so process restarts and provider failures retry instead of log-only
//! fire-and-forget.

use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_CLAIMED: &str = "claimed";
pub const STATUS_SENT: &str = "sent";
pub const STATUS_SKIPPED: &str = "skipped";
pub const STATUS_DEAD: &str = "dead";

/// Max delivery attempts before `dead`.
pub const MAX_ATTEMPTS: i32 = 12;

/// Claimed rows older than this are reclaimed as pending (worker crash).
pub const STALE_CLAIMED_MS: i64 = 60_000;

#[derive(Debug, Clone)]
pub struct PushDispatchRow {
    pub queue_id: String,
    pub event_id: String,
    pub account_id: String,
    pub topic: String,
    pub topic_seq: i64,
    pub payload_json: String,
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at_ms: i64,
    pub last_error: Option<String>,
    pub provider_message_id: Option<String>,
    pub created_at_ms: i64,
    pub claimed_by: Option<String>,
    pub claimed_at_ms: Option<i64>,
}

/// Enqueue push work for one (event, account). Idempotent on UNIQUE pair.
/// Returns `true` if a new row was inserted.
pub async fn enqueue<S>(
    store: &S,
    event_id: &str,
    account_id: &str,
    topic: &str,
    topic_seq: i64,
    payload_json: &str,
    now_ms: i64,
) -> Result<bool, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            enqueue_sqlite(
                pool,
                event_id,
                account_id,
                topic,
                topic_seq,
                payload_json,
                now_ms,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            enqueue_postgres(
                pool,
                event_id,
                account_id,
                topic,
                topic_seq,
                payload_json,
                now_ms,
            )
            .await
        }
    }
}

/// Claim due pending (and stale claimed) rows for processing.
pub async fn claim_due<S>(
    store: &S,
    now_ms: i64,
    limit: i64,
    worker_id: &str,
) -> Result<Vec<PushDispatchRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => claim_due_sqlite(pool, now_ms, limit, worker_id).await,
        StorePoolRef::Postgres(pool) => claim_due_postgres(pool, now_ms, limit, worker_id).await,
    }
}

pub async fn mark_sent<S>(
    store: &S,
    queue_id: &str,
    provider_message_id: Option<&str>,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    mark_terminal(store, queue_id, STATUS_SENT, None, provider_message_id, now_ms).await
}

pub async fn mark_skipped<S>(
    store: &S,
    queue_id: &str,
    reason: &str,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    mark_terminal(store, queue_id, STATUS_SKIPPED, Some(reason), None, now_ms).await
}

pub async fn mark_dead<S>(
    store: &S,
    queue_id: &str,
    last_error: &str,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    mark_terminal(store, queue_id, STATUS_DEAD, Some(last_error), None, now_ms).await
}

/// Requeue as pending with backoff after a transient failure.
pub async fn requeue_pending<S>(
    store: &S,
    queue_id: &str,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: &str,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            requeue_sqlite(pool, queue_id, attempts, next_attempt_at_ms, last_error).await
        }
        StorePoolRef::Postgres(pool) => {
            requeue_postgres(pool, queue_id, attempts, next_attempt_at_ms, last_error).await
        }
    }
}

/// Exponential backoff for next attempt (ms from now). Cap 5 minutes.
#[must_use]
pub fn backoff_delay_ms(attempts: i32) -> i64 {
    let exp = attempts.saturating_sub(1).clamp(0, 10) as u32;
    let base = 500_i64.saturating_mul(1_i64 << exp.min(10));
    base.min(300_000)
}

async fn mark_terminal<S>(
    store: &S,
    queue_id: &str,
    status: &str,
    last_error: Option<&str>,
    provider_message_id: Option<&str>,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            mark_terminal_sqlite(pool, queue_id, status, last_error, provider_message_id, now_ms)
                .await
        }
        StorePoolRef::Postgres(pool) => {
            mark_terminal_postgres(
                pool,
                queue_id,
                status,
                last_error,
                provider_message_id,
                now_ms,
            )
            .await
        }
    }
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn enqueue_sqlite(
    pool: &SqlitePool,
    event_id: &str,
    account_id: &str,
    topic: &str,
    topic_seq: i64,
    payload_json: &str,
    now_ms: i64,
) -> Result<bool, BackendError> {
    let queue_id = Uuid::new_v4().to_string();
    let result = sqlx::query(
        "INSERT INTO push_dispatch_queue (
            queue_id, event_id, account_id, topic, topic_seq, payload_json,
            status, attempts, next_attempt_at_ms, last_error, provider_message_id,
            created_at_ms, claimed_by, claimed_at_ms
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,0,?8,NULL,NULL,?8,NULL,NULL)
         ON CONFLICT(event_id, account_id) DO NOTHING",
    )
    .bind(&queue_id)
    .bind(event_id)
    .bind(account_id)
    .bind(topic)
    .bind(topic_seq)
    .bind(payload_json)
    .bind(STATUS_PENDING)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::enqueue"))?;
    Ok(result.rows_affected() > 0)
}

async fn claim_due_sqlite(
    pool: &SqlitePool,
    now_ms: i64,
    limit: i64,
    worker_id: &str,
) -> Result<Vec<PushDispatchRow>, BackendError> {
    let stale_before = now_ms - STALE_CLAIMED_MS;
    sqlx::query(
        "UPDATE push_dispatch_queue
         SET status = ?1, claimed_by = NULL, claimed_at_ms = NULL
         WHERE status = ?2 AND claimed_at_ms IS NOT NULL AND claimed_at_ms < ?3",
    )
    .bind(STATUS_PENDING)
    .bind(STATUS_CLAIMED)
    .bind(stale_before)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::reclaim_stale"))?;

    let candidates: Vec<(String,)> = sqlx::query_as(
        "SELECT queue_id FROM push_dispatch_queue
         WHERE status = ?1 AND next_attempt_at_ms <= ?2
         ORDER BY next_attempt_at_ms ASC
         LIMIT ?3",
    )
    .bind(STATUS_PENDING)
    .bind(now_ms)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(store_err("push_dispatch_queue::claim_due.select"))?;

    let mut out = Vec::with_capacity(candidates.len());
    for (queue_id,) in candidates {
        let result = sqlx::query(
            "UPDATE push_dispatch_queue
             SET status = ?1, attempts = attempts + 1,
                 claimed_by = ?2, claimed_at_ms = ?3
             WHERE queue_id = ?4 AND status = ?5",
        )
        .bind(STATUS_CLAIMED)
        .bind(worker_id)
        .bind(now_ms)
        .bind(&queue_id)
        .bind(STATUS_PENDING)
        .execute(pool)
        .await
        .map_err(store_err("push_dispatch_queue::claim_due.update"))?;
        if result.rows_affected() == 0 {
            continue;
        }
        if let Some(row) = get_by_id_sqlite(pool, &queue_id).await? {
            out.push(row);
        }
    }
    Ok(out)
}

async fn get_by_id_sqlite(
    pool: &SqlitePool,
    queue_id: &str,
) -> Result<Option<PushDispatchRow>, BackendError> {
    let row = sqlx::query_as::<_, PushDispatchSqlRow>(
        "SELECT queue_id, event_id, account_id, topic, topic_seq, payload_json,
                status, attempts, next_attempt_at_ms, last_error, provider_message_id,
                created_at_ms, claimed_by, claimed_at_ms
         FROM push_dispatch_queue WHERE queue_id = ?1",
    )
    .bind(queue_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("push_dispatch_queue::get_by_id"))?;
    Ok(row.map(Into::into))
}

async fn mark_terminal_sqlite(
    pool: &SqlitePool,
    queue_id: &str,
    status: &str,
    last_error: Option<&str>,
    provider_message_id: Option<&str>,
    _now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "UPDATE push_dispatch_queue
         SET status = ?1, last_error = ?2, provider_message_id = COALESCE(?3, provider_message_id),
             claimed_by = NULL, claimed_at_ms = NULL
         WHERE queue_id = ?4",
    )
    .bind(status)
    .bind(last_error)
    .bind(provider_message_id)
    .bind(queue_id)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::mark_terminal"))?;
    Ok(())
}

async fn requeue_sqlite(
    pool: &SqlitePool,
    queue_id: &str,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: &str,
) -> Result<(), BackendError> {
    sqlx::query(
        "UPDATE push_dispatch_queue
         SET status = ?1, attempts = ?2, next_attempt_at_ms = ?3,
             last_error = ?4, claimed_by = NULL, claimed_at_ms = NULL
         WHERE queue_id = ?5",
    )
    .bind(STATUS_PENDING)
    .bind(attempts)
    .bind(next_attempt_at_ms)
    .bind(last_error)
    .bind(queue_id)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::requeue"))?;
    Ok(())
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn enqueue_postgres(
    pool: &PgPool,
    event_id: &str,
    account_id: &str,
    topic: &str,
    topic_seq: i64,
    payload_json: &str,
    now_ms: i64,
) -> Result<bool, BackendError> {
    let queue_id = Uuid::new_v4().to_string();
    let result = sqlx::query(
        "INSERT INTO push_dispatch_queue (
            queue_id, event_id, account_id, topic, topic_seq, payload_json,
            status, attempts, next_attempt_at_ms, last_error, provider_message_id,
            created_at_ms, claimed_by, claimed_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,0,$8,NULL,NULL,$8,NULL,NULL)
         ON CONFLICT(event_id, account_id) DO NOTHING",
    )
    .bind(&queue_id)
    .bind(event_id)
    .bind(account_id)
    .bind(topic)
    .bind(topic_seq)
    .bind(payload_json)
    .bind(STATUS_PENDING)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::enqueue"))?;
    Ok(result.rows_affected() > 0)
}

async fn claim_due_postgres(
    pool: &PgPool,
    now_ms: i64,
    limit: i64,
    worker_id: &str,
) -> Result<Vec<PushDispatchRow>, BackendError> {
    let stale_before = now_ms - STALE_CLAIMED_MS;
    sqlx::query(
        "UPDATE push_dispatch_queue
         SET status = $1, claimed_by = NULL, claimed_at_ms = NULL
         WHERE status = $2 AND claimed_at_ms IS NOT NULL AND claimed_at_ms < $3",
    )
    .bind(STATUS_PENDING)
    .bind(STATUS_CLAIMED)
    .bind(stale_before)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::reclaim_stale"))?;

    let rows = sqlx::query_as::<_, PushDispatchSqlRowPg>(
        "UPDATE push_dispatch_queue
         SET status = $1, attempts = attempts + 1,
             claimed_by = $2, claimed_at_ms = $3
         WHERE queue_id IN (
           SELECT queue_id FROM push_dispatch_queue
           WHERE status = $4 AND next_attempt_at_ms <= $3
           ORDER BY next_attempt_at_ms ASC
           LIMIT $5
           FOR UPDATE SKIP LOCKED
         )
         RETURNING queue_id, event_id, account_id, topic, topic_seq, payload_json,
                   status, attempts, next_attempt_at_ms, last_error, provider_message_id,
                   created_at_ms, claimed_by, claimed_at_ms",
    )
    .bind(STATUS_CLAIMED)
    .bind(worker_id)
    .bind(now_ms)
    .bind(STATUS_PENDING)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(store_err("push_dispatch_queue::claim_due"))?;
    Ok(rows.into_iter().map(Into::into).collect())
}

async fn mark_terminal_postgres(
    pool: &PgPool,
    queue_id: &str,
    status: &str,
    last_error: Option<&str>,
    provider_message_id: Option<&str>,
    _now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "UPDATE push_dispatch_queue
         SET status = $1, last_error = $2, provider_message_id = COALESCE($3, provider_message_id),
             claimed_by = NULL, claimed_at_ms = NULL
         WHERE queue_id = $4",
    )
    .bind(status)
    .bind(last_error)
    .bind(provider_message_id)
    .bind(queue_id)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::mark_terminal"))?;
    Ok(())
}

async fn requeue_postgres(
    pool: &PgPool,
    queue_id: &str,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: &str,
) -> Result<(), BackendError> {
    sqlx::query(
        "UPDATE push_dispatch_queue
         SET status = $1, attempts = $2, next_attempt_at_ms = $3,
             last_error = $4, claimed_by = NULL, claimed_at_ms = NULL
         WHERE queue_id = $5",
    )
    .bind(STATUS_PENDING)
    .bind(attempts)
    .bind(next_attempt_at_ms)
    .bind(last_error)
    .bind(queue_id)
    .execute(pool)
    .await
    .map_err(store_err("push_dispatch_queue::requeue"))?;
    Ok(())
}

// ── Row types ──────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct PushDispatchSqlRow {
    queue_id: String,
    event_id: String,
    account_id: String,
    topic: String,
    topic_seq: i64,
    payload_json: String,
    status: String,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: Option<String>,
    provider_message_id: Option<String>,
    created_at_ms: i64,
    claimed_by: Option<String>,
    claimed_at_ms: Option<i64>,
}

impl From<PushDispatchSqlRow> for PushDispatchRow {
    fn from(r: PushDispatchSqlRow) -> Self {
        Self {
            queue_id: r.queue_id,
            event_id: r.event_id,
            account_id: r.account_id,
            topic: r.topic,
            topic_seq: r.topic_seq,
            payload_json: r.payload_json,
            status: r.status,
            attempts: r.attempts,
            next_attempt_at_ms: r.next_attempt_at_ms,
            last_error: r.last_error,
            provider_message_id: r.provider_message_id,
            created_at_ms: r.created_at_ms,
            claimed_by: r.claimed_by,
            claimed_at_ms: r.claimed_at_ms,
        }
    }
}

#[derive(sqlx::FromRow)]
struct PushDispatchSqlRowPg {
    queue_id: String,
    event_id: String,
    account_id: String,
    topic: String,
    topic_seq: i64,
    payload_json: String,
    status: String,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: Option<String>,
    provider_message_id: Option<String>,
    created_at_ms: i64,
    claimed_by: Option<String>,
    claimed_at_ms: Option<i64>,
}

impl From<PushDispatchSqlRowPg> for PushDispatchRow {
    fn from(r: PushDispatchSqlRowPg) -> Self {
        Self {
            queue_id: r.queue_id,
            event_id: r.event_id,
            account_id: r.account_id,
            topic: r.topic,
            topic_seq: r.topic_seq,
            payload_json: r.payload_json,
            status: r.status,
            attempts: r.attempts,
            next_attempt_at_ms: r.next_attempt_at_ms,
            last_error: r.last_error,
            provider_message_id: r.provider_message_id,
            created_at_ms: r.created_at_ms,
            claimed_by: r.claimed_by,
            claimed_at_ms: r.claimed_at_ms,
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
    use crate::store::test_support::{insert_account, memory_pool};

    #[tokio::test]
    async fn enqueue_claim_sent_round_trip() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "push-q@example.com").await;
        let now = 1_000_i64;
        assert!(enqueue(
            &pool,
            "ev-1",
            &account_id,
            "account:x",
            1,
            r#"{"kind":"test"}"#,
            now,
        )
        .await
        .unwrap());
        // Idempotent
        assert!(!enqueue(
            &pool,
            "ev-1",
            &account_id,
            "account:x",
            1,
            r#"{"kind":"test"}"#,
            now,
        )
        .await
        .unwrap());

        let claimed = claim_due(&pool, now, 10, "worker-1").await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].event_id, "ev-1");
        assert_eq!(claimed[0].attempts, 1);

        mark_sent(&pool, &claimed[0].queue_id, Some("prov-1"), now + 1)
            .await
            .unwrap();
        let again = claim_due(&pool, now + 10, 10, "worker-1").await.unwrap();
        assert!(again.is_empty());
    }

    #[tokio::test]
    async fn requeue_and_backoff_claim() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "push-retry@example.com").await;
        let now = 1_000_i64;
        enqueue(
            &pool,
            "ev-r",
            &account_id,
            "account:x",
            1,
            "{}",
            now,
        )
        .await
        .unwrap();
        let claimed = claim_due(&pool, now, 10, "w").await.unwrap();
        let row = &claimed[0];
        let next = now + backoff_delay_ms(row.attempts);
        requeue_pending(&pool, &row.queue_id, row.attempts, next, "rate_limited")
            .await
            .unwrap();
        assert!(claim_due(&pool, now, 10, "w").await.unwrap().is_empty());
        let later = claim_due(&pool, next, 10, "w").await.unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].attempts, 2);
    }
}
