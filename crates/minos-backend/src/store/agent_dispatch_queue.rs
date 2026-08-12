//! Persistent AgentDispatchQueue — bot **mailbox** physical table.
//!
//! Domain name: `bot_message_deliveries`.
//! Live collab writes enqueue rows in the same transaction as the origin
//! conversation message + social durable/outbox. A worker then leases them to a
//! Host for execution, with backoff and terminal failure.
//!
//! `dispatch_id` == delivery_id. UNIQUE(origin_message_id, agent_id) is the
//! logical mailbox key (one delivery per message×bot).

use sqlx::{PgPool, SqlitePool};

use crate::app::tx::DbTx;
use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_INFLIGHT: &str = "inflight";
pub const STATUS_SUCCEEDED: &str = "succeeded";
pub const STATUS_FAILED_TERMINAL: &str = "failed_terminal";

/// Max forward attempts before `failed_terminal` + user-visible error bubble.
pub const MAX_ATTEMPTS: i32 = 12;

/// Inflight rows older than this are reclaimed as pending (worker crash).
pub const STALE_INFLIGHT_MS: i64 = 60_000;

#[derive(Debug, Clone)]
pub struct AgentDispatchRow {
    /// Delivery id (mailbox row id).
    pub dispatch_id: String,
    pub origin_message_id: String,
    pub conversation_id: String,
    pub account_id: String,
    /// Global bot id (`agents.agent_id`).
    pub agent_id: String,
    pub session_id: Option<String>,
    pub forwarded_text: String,
    pub mention_sender: bool,
    pub sender_minos_id: Option<String>,
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at_ms: i64,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Host installation currently holding the lease (mailbox consumer).
    pub lease_owner_host_id: Option<String>,
    pub lease_expires_at_ms: Option<i64>,
    /// Bot-to-bot automation hop count from root human message.
    pub automation_hop: i32,
}

impl AgentDispatchRow {
    /// Alias: domain name `delivery_id`.
    #[must_use]
    pub fn delivery_id(&self) -> &str {
        &self.dispatch_id
    }
}

/// Insert a pending dispatch. Idempotent on `(origin_message_id, agent_id)`.
/// Multi-@ fan-out enqueues one row per agent for the same origin.
/// Returns `true` if a new row was inserted.
pub async fn enqueue<S>(store: &S, row: &AgentDispatchRow) -> Result<bool, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => enqueue_sqlite(pool, row).await,
        StorePoolRef::Postgres(pool) => enqueue_postgres(pool, row).await,
    }
}

/// Same as [`enqueue`] on an open write transaction (message + bot deliveries co-commit).
pub async fn enqueue_in_tx(
    tx: &mut DbTx<'_>,
    row: &AgentDispatchRow,
) -> Result<bool, BackendError> {
    match tx {
        DbTx::Sqlite(tx) => {
            let mention = i64::from(row.mention_sender);
            let result = sqlx::query(
                "INSERT INTO bot_message_deliveries (
                    dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                    session_id, forwarded_text, mention_sender, sender_minos_id, status,
                    attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                    lease_owner_host_id, lease_expires_at_ms, automation_hop
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
                 ON CONFLICT(origin_message_id, agent_id) DO NOTHING",
            )
            .bind(&row.dispatch_id)
            .bind(&row.origin_message_id)
            .bind(&row.conversation_id)
            .bind(&row.account_id)
            .bind(&row.agent_id)
            .bind(&row.session_id)
            .bind(&row.forwarded_text)
            .bind(mention)
            .bind(&row.sender_minos_id)
            .bind(&row.status)
            .bind(row.attempts)
            .bind(row.next_attempt_at_ms)
            .bind(&row.last_error)
            .bind(row.created_at_ms)
            .bind(row.updated_at_ms)
            .bind(&row.lease_owner_host_id)
            .bind(row.lease_expires_at_ms)
            .bind(row.automation_hop)
            .execute(&mut **tx)
            .await
            .map_err(store_err("agent_dispatch_queue::enqueue_in_tx"))?;
            Ok(result.rows_affected() > 0)
        }
        DbTx::Postgres(tx) => {
            let result = sqlx::query(
                "INSERT INTO bot_message_deliveries (
                    dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                    session_id, forwarded_text, mention_sender, sender_minos_id, status,
                    attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                    lease_owner_host_id, lease_expires_at_ms, automation_hop
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
                 ON CONFLICT(origin_message_id, agent_id) DO NOTHING",
            )
            .bind(&row.dispatch_id)
            .bind(&row.origin_message_id)
            .bind(&row.conversation_id)
            .bind(&row.account_id)
            .bind(&row.agent_id)
            .bind(&row.session_id)
            .bind(&row.forwarded_text)
            .bind(row.mention_sender)
            .bind(&row.sender_minos_id)
            .bind(&row.status)
            .bind(row.attempts)
            .bind(row.next_attempt_at_ms)
            .bind(&row.last_error)
            .bind(row.created_at_ms)
            .bind(row.updated_at_ms)
            .bind(&row.lease_owner_host_id)
            .bind(row.lease_expires_at_ms)
            .bind(row.automation_hop)
            .execute(&mut **tx)
            .await
            .map_err(store_err("agent_dispatch_queue::enqueue_in_tx"))?;
            Ok(result.rows_affected() > 0)
        }
    }
}

/// Claim due pending (and stale inflight) rows for processing.
pub async fn claim_due<S>(
    store: &S,
    now_ms: i64,
    limit: i64,
) -> Result<Vec<AgentDispatchRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => claim_due_sqlite(pool, now_ms, limit).await,
        StorePoolRef::Postgres(pool) => claim_due_postgres(pool, now_ms, limit).await,
    }
}

pub async fn mark_succeeded<S>(
    store: &S,
    dispatch_id: &str,
    session_id: &str,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            mark_status_sqlite(
                pool,
                dispatch_id,
                STATUS_SUCCEEDED,
                Some(session_id),
                None,
                now_ms,
                None,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            mark_status_postgres(
                pool,
                dispatch_id,
                STATUS_SUCCEEDED,
                Some(session_id),
                None,
                now_ms,
                None,
            )
            .await
        }
    }
}

pub async fn mark_failed_terminal<S>(
    store: &S,
    dispatch_id: &str,
    last_error: &str,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            mark_status_sqlite(
                pool,
                dispatch_id,
                STATUS_FAILED_TERMINAL,
                None,
                Some(last_error),
                now_ms,
                None,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            mark_status_postgres(
                pool,
                dispatch_id,
                STATUS_FAILED_TERMINAL,
                None,
                Some(last_error),
                now_ms,
                None,
            )
            .await
        }
    }
}

/// Requeue as pending with backoff after a transient failure.
pub async fn requeue_pending<S>(
    store: &S,
    dispatch_id: &str,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: &str,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            requeue_sqlite(
                pool,
                dispatch_id,
                attempts,
                next_attempt_at_ms,
                last_error,
                now_ms,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            requeue_postgres(
                pool,
                dispatch_id,
                attempts,
                next_attempt_at_ms,
                last_error,
                now_ms,
            )
            .await
        }
    }
}

pub async fn get_by_origin<S>(
    store: &S,
    origin_message_id: &str,
) -> Result<Option<AgentDispatchRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => get_by_origin_sqlite(pool, origin_message_id).await,
        StorePoolRef::Postgres(pool) => get_by_origin_postgres(pool, origin_message_id).await,
    }
}

/// Lookup a mailbox row by delivery/dispatch id.
pub async fn get_by_id<S>(
    store: &S,
    delivery_id: &str,
) -> Result<Option<AgentDispatchRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => get_by_id_sqlite(pool, delivery_id).await,
        StorePoolRef::Postgres(pool) => get_by_id_postgres(pool, delivery_id).await,
    }
}

/// List all inbox rows for an origin, ordered by enqueue time (appearance order
/// for multi-@ fan-out when plans are enqueued in extract order).
pub async fn list_by_origin<S>(
    store: &S,
    origin_message_id: &str,
) -> Result<Vec<AgentDispatchRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let rows = sqlx::query_as::<_, AgentDispatchSqlRow>(
                "SELECT dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                        session_id, forwarded_text, mention_sender, sender_minos_id, status,
                        attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                        lease_owner_host_id, lease_expires_at_ms, automation_hop
                 FROM bot_message_deliveries
                 WHERE origin_message_id = ?1
                 ORDER BY created_at_ms ASC, dispatch_id ASC",
            )
            .bind(origin_message_id)
            .fetch_all(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::list_by_origin"))?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
        StorePoolRef::Postgres(pool) => {
            let rows = sqlx::query_as::<_, AgentDispatchSqlRowPg>(
                "SELECT dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                        session_id, forwarded_text, mention_sender, sender_minos_id, status,
                        attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                        lease_owner_host_id, lease_expires_at_ms, automation_hop
                 FROM bot_message_deliveries
                 WHERE origin_message_id = $1
                 ORDER BY created_at_ms ASC, dispatch_id ASC",
            )
            .bind(origin_message_id)
            .fetch_all(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::list_by_origin"))?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
    }
}

/// Count dispatch rows for an origin (multi-@ fan-out size).
pub async fn count_by_origin<S>(store: &S, origin_message_id: &str) -> Result<i64, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM bot_message_deliveries WHERE origin_message_id = ?1",
            )
            .bind(origin_message_id)
            .fetch_one(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::count_by_origin"))?;
            Ok(n)
        }
        StorePoolRef::Postgres(pool) => {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM bot_message_deliveries WHERE origin_message_id = $1",
            )
            .bind(origin_message_id)
            .fetch_one(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::count_by_origin"))?;
            Ok(n)
        }
    }
}

/// Host-online edge: make pending (and reclaimable inflight) rows for these
/// accounts immediately due so the worker can drain without waiting backoff.
///
/// Sets `status=pending`, `next_attempt_at_ms=now_ms` for matching rows.
/// Returns number of rows touched.
pub async fn force_due_for_accounts<S>(
    store: &S,
    account_ids: &[String],
    now_ms: i64,
) -> Result<u32, BackendError>
where
    S: AsStorePool + ?Sized,
{
    if account_ids.is_empty() {
        return Ok(0);
    }
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => force_due_sqlite(pool, account_ids, now_ms).await,
        StorePoolRef::Postgres(pool) => force_due_postgres(pool, account_ids, now_ms).await,
    }
}

/// Exponential backoff for next attempt (ms from now). Cap 5 minutes.
#[must_use]
pub fn backoff_delay_ms(attempts: i32) -> i64 {
    let exp = attempts.saturating_sub(1).clamp(0, 10) as u32;
    let base = 500_i64.saturating_mul(1_i64 << exp.min(10));
    base.min(300_000)
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn enqueue_sqlite(pool: &SqlitePool, row: &AgentDispatchRow) -> Result<bool, BackendError> {
    let mention = i64::from(row.mention_sender);
    let result = sqlx::query(
        "INSERT INTO bot_message_deliveries (
            dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
            session_id, forwarded_text, mention_sender, sender_minos_id, status,
            attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
            lease_owner_host_id, lease_expires_at_ms, automation_hop
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
         ON CONFLICT(origin_message_id, agent_id) DO NOTHING",
    )
    .bind(&row.dispatch_id)
    .bind(&row.origin_message_id)
    .bind(&row.conversation_id)
    .bind(&row.account_id)
    .bind(&row.agent_id)
    .bind(&row.session_id)
    .bind(&row.forwarded_text)
    .bind(mention)
    .bind(&row.sender_minos_id)
    .bind(&row.status)
    .bind(row.attempts)
    .bind(row.next_attempt_at_ms)
    .bind(&row.last_error)
    .bind(row.created_at_ms)
    .bind(row.updated_at_ms)
    .bind(&row.lease_owner_host_id)
    .bind(row.lease_expires_at_ms)
    .bind(row.automation_hop)
    .execute(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::enqueue"))?;
    Ok(result.rows_affected() > 0)
}

async fn claim_due_sqlite(
    pool: &SqlitePool,
    now_ms: i64,
    limit: i64,
) -> Result<Vec<AgentDispatchRow>, BackendError> {
    let stale_before = now_ms - STALE_INFLIGHT_MS;
    // Reclaim stale/expired leases: honor lease_expires_at_ms when set.
    sqlx::query(
        "UPDATE bot_message_deliveries
         SET status = ?1,
             lease_owner_host_id = NULL,
             lease_expires_at_ms = NULL,
             updated_at_ms = ?2
         WHERE status = ?3
           AND (
             (lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms <= ?2)
             OR (lease_expires_at_ms IS NULL AND updated_at_ms < ?4)
           )",
    )
    .bind(STATUS_PENDING)
    .bind(now_ms)
    .bind(STATUS_INFLIGHT)
    .bind(stale_before)
    .execute(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::reclaim_stale"))?;

    let candidates: Vec<(String,)> = sqlx::query_as(
        "SELECT dispatch_id FROM bot_message_deliveries
         WHERE status = ?1 AND next_attempt_at_ms <= ?2
         ORDER BY next_attempt_at_ms ASC
         LIMIT ?3",
    )
    .bind(STATUS_PENDING)
    .bind(now_ms)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::claim_due.select"))?;

    let mut out = Vec::with_capacity(candidates.len());
    for (dispatch_id,) in candidates {
        let result = sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = ?1, attempts = attempts + 1, updated_at_ms = ?2
             WHERE dispatch_id = ?3 AND status = ?4",
        )
        .bind(STATUS_INFLIGHT)
        .bind(now_ms)
        .bind(&dispatch_id)
        .bind(STATUS_PENDING)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::claim_due.update"))?;
        if result.rows_affected() == 0 {
            continue;
        }
        if let Some(row) = get_by_id_sqlite(pool, &dispatch_id).await? {
            out.push(row);
        }
    }
    Ok(out)
}

async fn get_by_id_sqlite(
    pool: &SqlitePool,
    dispatch_id: &str,
) -> Result<Option<AgentDispatchRow>, BackendError> {
    let row = sqlx::query_as::<_, AgentDispatchSqlRow>(
        "SELECT dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                session_id, forwarded_text, mention_sender, sender_minos_id, status,
                attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                lease_owner_host_id, lease_expires_at_ms, automation_hop
         FROM bot_message_deliveries WHERE dispatch_id = ?1",
    )
    .bind(dispatch_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::get_by_id"))?;
    Ok(row.map(Into::into))
}

async fn get_by_origin_sqlite(
    pool: &SqlitePool,
    origin_message_id: &str,
) -> Result<Option<AgentDispatchRow>, BackendError> {
    let row = sqlx::query_as::<_, AgentDispatchSqlRow>(
        "SELECT dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                session_id, forwarded_text, mention_sender, sender_minos_id, status,
                attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                lease_owner_host_id, lease_expires_at_ms, automation_hop
         FROM bot_message_deliveries WHERE origin_message_id = ?1",
    )
    .bind(origin_message_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::get_by_origin"))?;
    Ok(row.map(Into::into))
}

async fn mark_status_sqlite(
    pool: &SqlitePool,
    dispatch_id: &str,
    status: &str,
    session_id: Option<&str>,
    last_error: Option<&str>,
    now_ms: i64,
    next_attempt_at_ms: Option<i64>,
) -> Result<(), BackendError> {
    if let Some(sid) = session_id {
        sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = ?1, session_id = ?2, last_error = ?3, updated_at_ms = ?4
             WHERE dispatch_id = ?5",
        )
        .bind(status)
        .bind(sid)
        .bind(last_error)
        .bind(now_ms)
        .bind(dispatch_id)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::mark_status"))?;
    } else if let Some(next) = next_attempt_at_ms {
        sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = ?1, last_error = ?2, next_attempt_at_ms = ?3, updated_at_ms = ?4
             WHERE dispatch_id = ?5",
        )
        .bind(status)
        .bind(last_error)
        .bind(next)
        .bind(now_ms)
        .bind(dispatch_id)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::mark_status"))?;
    } else {
        sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = ?1, last_error = ?2, updated_at_ms = ?3
             WHERE dispatch_id = ?4",
        )
        .bind(status)
        .bind(last_error)
        .bind(now_ms)
        .bind(dispatch_id)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::mark_status"))?;
    }
    Ok(())
}

async fn requeue_sqlite(
    pool: &SqlitePool,
    dispatch_id: &str,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "UPDATE bot_message_deliveries
         SET status = ?1, attempts = ?2, next_attempt_at_ms = ?3,
             last_error = ?4, updated_at_ms = ?5
         WHERE dispatch_id = ?6",
    )
    .bind(STATUS_PENDING)
    .bind(attempts)
    .bind(next_attempt_at_ms)
    .bind(last_error)
    .bind(now_ms)
    .bind(dispatch_id)
    .execute(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::requeue"))?;
    Ok(())
}

async fn force_due_sqlite(
    pool: &SqlitePool,
    account_ids: &[String],
    now_ms: i64,
) -> Result<u32, BackendError> {
    // pending always; inflight only when lease is expired/stale (do not steal live work).
    let stale_before = now_ms - STALE_INFLIGHT_MS;
    let mut total = 0u32;
    for account_id in account_ids {
        let result = sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = ?1,
                 next_attempt_at_ms = ?2,
                 lease_owner_host_id = NULL,
                 lease_expires_at_ms = NULL,
                 updated_at_ms = ?3
             WHERE account_id = ?4
               AND (
                    status = ?5
                    OR (
                        status = ?6
                        AND (
                            (lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms < ?2)
                            OR (lease_expires_at_ms IS NULL AND updated_at_ms < ?7)
                        )
                    )
               )",
        )
        .bind(STATUS_PENDING)
        .bind(now_ms)
        .bind(now_ms)
        .bind(account_id)
        .bind(STATUS_PENDING)
        .bind(STATUS_INFLIGHT)
        .bind(stale_before)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::force_due"))?;
        total = total.saturating_add(result.rows_affected() as u32);
    }
    Ok(total)
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn enqueue_postgres(pool: &PgPool, row: &AgentDispatchRow) -> Result<bool, BackendError> {
    let result = sqlx::query(
        "INSERT INTO bot_message_deliveries (
            dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
            session_id, forwarded_text, mention_sender, sender_minos_id, status,
            attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
            lease_owner_host_id, lease_expires_at_ms, automation_hop
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
         ON CONFLICT(origin_message_id, agent_id) DO NOTHING",
    )
    .bind(&row.dispatch_id)
    .bind(&row.origin_message_id)
    .bind(&row.conversation_id)
    .bind(&row.account_id)
    .bind(&row.agent_id)
    .bind(&row.session_id)
    .bind(&row.forwarded_text)
    .bind(row.mention_sender)
    .bind(&row.sender_minos_id)
    .bind(&row.status)
    .bind(row.attempts)
    .bind(row.next_attempt_at_ms)
    .bind(&row.last_error)
    .bind(row.created_at_ms)
    .bind(row.updated_at_ms)
    .bind(&row.lease_owner_host_id)
    .bind(row.lease_expires_at_ms)
    .bind(row.automation_hop)
    .execute(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::enqueue"))?;
    Ok(result.rows_affected() > 0)
}

async fn claim_due_postgres(
    pool: &PgPool,
    now_ms: i64,
    limit: i64,
) -> Result<Vec<AgentDispatchRow>, BackendError> {
    let stale_before = now_ms - STALE_INFLIGHT_MS;
    // Reclaim expired leases (prefer lease_expires_at_ms) or stale inflight.
    sqlx::query(
        "UPDATE bot_message_deliveries
         SET status = $1,
             lease_owner_host_id = NULL,
             lease_expires_at_ms = NULL,
             updated_at_ms = $2
         WHERE status = $3
           AND (
             (lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms <= $2)
             OR (lease_expires_at_ms IS NULL AND updated_at_ms < $4)
           )",
    )
    .bind(STATUS_PENDING)
    .bind(now_ms)
    .bind(STATUS_INFLIGHT)
    .bind(stale_before)
    .execute(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::reclaim_stale"))?;

    // Atomic claim with SKIP LOCKED for multi-instance.
    let rows = sqlx::query_as::<_, AgentDispatchSqlRowPg>(
        "UPDATE bot_message_deliveries
         SET status = $1, attempts = attempts + 1, updated_at_ms = $2
         WHERE dispatch_id IN (
           SELECT dispatch_id FROM bot_message_deliveries
           WHERE status = $3 AND next_attempt_at_ms <= $2
           ORDER BY next_attempt_at_ms ASC
           LIMIT $4
           FOR UPDATE SKIP LOCKED
         )
         RETURNING dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                   session_id, forwarded_text, mention_sender, sender_minos_id, status,
                   attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                   lease_owner_host_id, lease_expires_at_ms, automation_hop",
    )
    .bind(STATUS_INFLIGHT)
    .bind(now_ms)
    .bind(STATUS_PENDING)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::claim_due"))?;
    Ok(rows.into_iter().map(Into::into).collect())
}

async fn get_by_id_postgres(
    pool: &PgPool,
    delivery_id: &str,
) -> Result<Option<AgentDispatchRow>, BackendError> {
    let row = sqlx::query_as::<_, AgentDispatchSqlRowPg>(
        "SELECT dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                session_id, forwarded_text, mention_sender, sender_minos_id, status,
                attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                lease_owner_host_id, lease_expires_at_ms, automation_hop
         FROM bot_message_deliveries WHERE dispatch_id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::get_by_id"))?;
    Ok(row.map(Into::into))
}

async fn get_by_origin_postgres(
    pool: &PgPool,
    origin_message_id: &str,
) -> Result<Option<AgentDispatchRow>, BackendError> {
    let row = sqlx::query_as::<_, AgentDispatchSqlRowPg>(
        "SELECT dispatch_id, origin_message_id, conversation_id, account_id, agent_id,
                session_id, forwarded_text, mention_sender, sender_minos_id, status,
                attempts, next_attempt_at_ms, last_error, created_at_ms, updated_at_ms,
                lease_owner_host_id, lease_expires_at_ms, automation_hop
         FROM bot_message_deliveries WHERE origin_message_id = $1",
    )
    .bind(origin_message_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::get_by_origin"))?;
    Ok(row.map(Into::into))
}

async fn mark_status_postgres(
    pool: &PgPool,
    dispatch_id: &str,
    status: &str,
    session_id: Option<&str>,
    last_error: Option<&str>,
    now_ms: i64,
    next_attempt_at_ms: Option<i64>,
) -> Result<(), BackendError> {
    if let Some(sid) = session_id {
        sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = $1, session_id = $2, last_error = $3, updated_at_ms = $4
             WHERE dispatch_id = $5",
        )
        .bind(status)
        .bind(sid)
        .bind(last_error)
        .bind(now_ms)
        .bind(dispatch_id)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::mark_status"))?;
    } else if let Some(next) = next_attempt_at_ms {
        sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = $1, last_error = $2, next_attempt_at_ms = $3, updated_at_ms = $4
             WHERE dispatch_id = $5",
        )
        .bind(status)
        .bind(last_error)
        .bind(next)
        .bind(now_ms)
        .bind(dispatch_id)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::mark_status"))?;
    } else {
        sqlx::query(
            "UPDATE bot_message_deliveries
             SET status = $1, last_error = $2, updated_at_ms = $3
             WHERE dispatch_id = $4",
        )
        .bind(status)
        .bind(last_error)
        .bind(now_ms)
        .bind(dispatch_id)
        .execute(pool)
        .await
        .map_err(store_err("agent_dispatch_queue::mark_status"))?;
    }
    Ok(())
}

async fn requeue_postgres(
    pool: &PgPool,
    dispatch_id: &str,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "UPDATE bot_message_deliveries
         SET status = $1, attempts = $2, next_attempt_at_ms = $3,
             last_error = $4, updated_at_ms = $5
         WHERE dispatch_id = $6",
    )
    .bind(STATUS_PENDING)
    .bind(attempts)
    .bind(next_attempt_at_ms)
    .bind(last_error)
    .bind(now_ms)
    .bind(dispatch_id)
    .execute(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::requeue"))?;
    Ok(())
}

async fn force_due_postgres(
    pool: &PgPool,
    account_ids: &[String],
    now_ms: i64,
) -> Result<u32, BackendError> {
    // pending always; inflight only when lease is expired/stale (do not steal live work).
    let stale_before = now_ms - STALE_INFLIGHT_MS;
    let result = sqlx::query(
        "UPDATE bot_message_deliveries
         SET status = $1,
             next_attempt_at_ms = $2,
             lease_owner_host_id = NULL,
             lease_expires_at_ms = NULL,
             updated_at_ms = $3
         WHERE account_id = ANY($4)
           AND (
                status = $5
                OR (
                    status = $6
                    AND (
                        (lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms < $2)
                        OR (lease_expires_at_ms IS NULL AND updated_at_ms < $7)
                    )
                )
           )",
    )
    .bind(STATUS_PENDING)
    .bind(now_ms)
    .bind(now_ms)
    .bind(account_ids)
    .bind(STATUS_PENDING)
    .bind(STATUS_INFLIGHT)
    .bind(stale_before)
    .execute(pool)
    .await
    .map_err(store_err("agent_dispatch_queue::force_due"))?;
    Ok(result.rows_affected() as u32)
}

// ── row mapping ────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct AgentDispatchSqlRow {
    dispatch_id: String,
    origin_message_id: String,
    conversation_id: String,
    account_id: String,
    agent_id: String,
    session_id: Option<String>,
    forwarded_text: String,
    mention_sender: i64,
    sender_minos_id: Option<String>,
    status: String,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
    lease_owner_host_id: Option<String>,
    lease_expires_at_ms: Option<i64>,
    automation_hop: i32,
}

impl From<AgentDispatchSqlRow> for AgentDispatchRow {
    fn from(r: AgentDispatchSqlRow) -> Self {
        Self {
            dispatch_id: r.dispatch_id,
            origin_message_id: r.origin_message_id,
            conversation_id: r.conversation_id,
            account_id: r.account_id,
            agent_id: r.agent_id,
            session_id: r.session_id,
            forwarded_text: r.forwarded_text,
            mention_sender: r.mention_sender != 0,
            sender_minos_id: r.sender_minos_id,
            status: r.status,
            attempts: r.attempts,
            next_attempt_at_ms: r.next_attempt_at_ms,
            last_error: r.last_error,
            created_at_ms: r.created_at_ms,
            updated_at_ms: r.updated_at_ms,
            lease_owner_host_id: r.lease_owner_host_id,
            lease_expires_at_ms: r.lease_expires_at_ms,
            automation_hop: r.automation_hop,
        }
    }
}

#[derive(sqlx::FromRow)]
struct AgentDispatchSqlRowPg {
    dispatch_id: String,
    origin_message_id: String,
    conversation_id: String,
    account_id: String,
    agent_id: String,
    session_id: Option<String>,
    forwarded_text: String,
    mention_sender: bool,
    sender_minos_id: Option<String>,
    status: String,
    attempts: i32,
    next_attempt_at_ms: i64,
    last_error: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
    lease_owner_host_id: Option<String>,
    lease_expires_at_ms: Option<i64>,
    automation_hop: i32,
}

impl From<AgentDispatchSqlRowPg> for AgentDispatchRow {
    fn from(r: AgentDispatchSqlRowPg) -> Self {
        Self {
            dispatch_id: r.dispatch_id,
            origin_message_id: r.origin_message_id,
            conversation_id: r.conversation_id,
            account_id: r.account_id,
            agent_id: r.agent_id,
            session_id: r.session_id,
            forwarded_text: r.forwarded_text,
            mention_sender: r.mention_sender,
            sender_minos_id: r.sender_minos_id,
            status: r.status,
            attempts: r.attempts,
            next_attempt_at_ms: r.next_attempt_at_ms,
            last_error: r.last_error,
            created_at_ms: r.created_at_ms,
            updated_at_ms: r.updated_at_ms,
            lease_owner_host_id: r.lease_owner_host_id,
            lease_expires_at_ms: r.lease_expires_at_ms,
            automation_hop: r.automation_hop,
        }
    }
}

/// Default lease TTL for a host mailbox claim (ms).
pub const DEFAULT_LEASE_TTL_MS: i64 = 120_000;

/// Persist canonical session id while delivery remains inflight/leased.
pub async fn set_session_id<S>(
    store: &S,
    delivery_id: &str,
    session_id: &str,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "UPDATE bot_message_deliveries
                 SET session_id = ?1, updated_at_ms = ?2
                 WHERE dispatch_id = ?3",
            )
            .bind(session_id)
            .bind(now_ms)
            .bind(delivery_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::set_session_id"))?;
            Ok(())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "UPDATE bot_message_deliveries
                 SET session_id = $1, updated_at_ms = $2
                 WHERE dispatch_id = $3",
            )
            .bind(session_id)
            .bind(now_ms)
            .bind(delivery_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::set_session_id"))?;
            Ok(())
        }
    }
}

/// Assign lease ownership while row is inflight (mailbox consumer host).
pub async fn set_lease<S>(
    store: &S,
    delivery_id: &str,
    host_installation_id: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_owner_host_id = ?1,
                     lease_expires_at_ms = ?2,
                     updated_at_ms = ?3
                 WHERE dispatch_id = ?4",
            )
            .bind(host_installation_id)
            .bind(lease_expires_at_ms)
            .bind(now_ms)
            .bind(delivery_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::set_lease"))?;
            Ok(())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_owner_host_id = $1,
                     lease_expires_at_ms = $2,
                     updated_at_ms = $3
                 WHERE dispatch_id = $4",
            )
            .bind(host_installation_id)
            .bind(lease_expires_at_ms)
            .bind(now_ms)
            .bind(delivery_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::set_lease"))?;
            Ok(())
        }
    }
}

/// Extend an active mailbox lease when the owning host is still working the delivery.
pub async fn renew_lease<S>(
    store: &S,
    delivery_id: &str,
    host_installation_id: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<bool, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let result = sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_expires_at_ms = ?1,
                     updated_at_ms = ?2
                 WHERE dispatch_id = ?3
                   AND status = ?4
                   AND lease_owner_host_id = ?5",
            )
            .bind(lease_expires_at_ms)
            .bind(now_ms)
            .bind(delivery_id)
            .bind(STATUS_INFLIGHT)
            .bind(host_installation_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::renew_lease"))?;
            Ok(result.rows_affected() == 1)
        }
        StorePoolRef::Postgres(pool) => {
            let result = sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_expires_at_ms = $1,
                     updated_at_ms = $2
                 WHERE dispatch_id = $3
                   AND status = $4
                   AND lease_owner_host_id = $5",
            )
            .bind(lease_expires_at_ms)
            .bind(now_ms)
            .bind(delivery_id)
            .bind(STATUS_INFLIGHT)
            .bind(host_installation_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::renew_lease"))?;
            Ok(result.rows_affected() == 1)
        }
    }
}

/// Renew every inflight lease owned by `host_installation_id` (host keepalive / Ping).
pub async fn renew_leases_for_host<S>(
    store: &S,
    host_installation_id: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<u64, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let result = sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_expires_at_ms = ?1,
                     updated_at_ms = ?2
                 WHERE status = ?3
                   AND lease_owner_host_id = ?4",
            )
            .bind(lease_expires_at_ms)
            .bind(now_ms)
            .bind(STATUS_INFLIGHT)
            .bind(host_installation_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::renew_leases_for_host"))?;
            Ok(result.rows_affected())
        }
        StorePoolRef::Postgres(pool) => {
            let result = sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_expires_at_ms = $1,
                     updated_at_ms = $2
                 WHERE status = $3
                   AND lease_owner_host_id = $4",
            )
            .bind(lease_expires_at_ms)
            .bind(now_ms)
            .bind(STATUS_INFLIGHT)
            .bind(host_installation_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::renew_leases_for_host"))?;
            Ok(result.rows_affected())
        }
    }
}

/// Clear lease fields (e.g. after success, cancel, or reclaim).
pub async fn clear_lease<S>(store: &S, delivery_id: &str, now_ms: i64) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_owner_host_id = NULL,
                     lease_expires_at_ms = NULL,
                     updated_at_ms = ?1
                 WHERE dispatch_id = ?2",
            )
            .bind(now_ms)
            .bind(delivery_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::clear_lease"))?;
            Ok(())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "UPDATE bot_message_deliveries
                 SET lease_owner_host_id = NULL,
                     lease_expires_at_ms = NULL,
                     updated_at_ms = $1
                 WHERE dispatch_id = $2",
            )
            .bind(now_ms)
            .bind(delivery_id)
            .execute(pool)
            .await
            .map_err(store_err("agent_dispatch_queue::clear_lease"))?;
            Ok(())
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
    use crate::store::{test_support::memory_pool, StoreHandle};

    async fn mem() -> StoreHandle {
        StoreHandle::from(memory_pool().await)
    }

    #[tokio::test]
    async fn enqueue_is_idempotent_on_origin_and_agent() {
        let store = mem().await;
        let now = 1_000_i64;
        let row = AgentDispatchRow {
            dispatch_id: "d1".into(),
            origin_message_id: "origin-1".into(),
            conversation_id: "c1".into(),
            account_id: "a1".into(),
            agent_id: "agent1".into(),
            session_id: None,
            forwarded_text: "hi".into(),
            mention_sender: false,
            sender_minos_id: None,
            status: STATUS_PENDING.into(),
            attempts: 0,
            next_attempt_at_ms: now,
            last_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            lease_owner_host_id: None,
            lease_expires_at_ms: None,
            automation_hop: 0,
        };
        assert!(enqueue(&store, &row).await.unwrap());
        let mut row2 = row.clone();
        row2.dispatch_id = "d2".into();
        assert!(!enqueue(&store, &row2).await.unwrap());
        // Multi-@ fan-out: different agent on same origin is a new row.
        let mut row3 = row.clone();
        row3.dispatch_id = "d3".into();
        row3.agent_id = "agent2".into();
        assert!(enqueue(&store, &row3).await.unwrap());
        let got = get_by_origin(&store, "origin-1").await.unwrap().unwrap();
        assert_eq!(got.dispatch_id, "d1");
    }

    #[tokio::test]
    async fn claim_due_marks_inflight_and_increments_attempts() {
        let store = mem().await;
        let now = 5_000_i64;
        let row = AgentDispatchRow {
            dispatch_id: "d1".into(),
            origin_message_id: "o1".into(),
            conversation_id: "c1".into(),
            account_id: "a1".into(),
            agent_id: "agent1".into(),
            session_id: None,
            forwarded_text: "hi".into(),
            mention_sender: true,
            sender_minos_id: Some("alice".into()),
            status: STATUS_PENDING.into(),
            attempts: 0,
            next_attempt_at_ms: now,
            last_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            lease_owner_host_id: None,
            lease_expires_at_ms: None,
            automation_hop: 0,
        };
        enqueue(&store, &row).await.unwrap();
        let claimed = claim_due(&store, now, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].status, STATUS_INFLIGHT);
        assert_eq!(claimed[0].attempts, 1);
        // Not due again while inflight.
        assert!(claim_due(&store, now, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn set_lease_and_clear_lease_round_trip() {
        let store = mem().await;
        let now = 7_000_i64;
        let row = AgentDispatchRow {
            dispatch_id: "d-lease".into(),
            origin_message_id: "o-lease".into(),
            conversation_id: "c1".into(),
            account_id: "a1".into(),
            agent_id: "agent1".into(),
            session_id: None,
            forwarded_text: "hi".into(),
            mention_sender: false,
            sender_minos_id: None,
            status: STATUS_PENDING.into(),
            attempts: 0,
            next_attempt_at_ms: now,
            last_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            lease_owner_host_id: None,
            lease_expires_at_ms: None,
            automation_hop: 0,
        };
        enqueue(&store, &row).await.unwrap();
        let expires = now + DEFAULT_LEASE_TTL_MS;
        set_lease(&store, "d-lease", "host-install-1", expires, now)
            .await
            .unwrap();
        let got = get_by_id(&store, "d-lease").await.unwrap().unwrap();
        assert_eq!(got.lease_owner_host_id.as_deref(), Some("host-install-1"));
        assert_eq!(got.lease_expires_at_ms, Some(expires));
        clear_lease(&store, "d-lease", now + 1).await.unwrap();
        let cleared = get_by_id(&store, "d-lease").await.unwrap().unwrap();
        assert!(cleared.lease_owner_host_id.is_none());
        assert!(cleared.lease_expires_at_ms.is_none());
    }

    #[tokio::test]
    async fn claim_due_reclaims_expired_lease_before_stale_updated_at() {
        let store = mem().await;
        let now = 20_000_i64;
        let row = AgentDispatchRow {
            dispatch_id: "d-exp".into(),
            origin_message_id: "o-exp".into(),
            conversation_id: "c1".into(),
            account_id: "a1".into(),
            agent_id: "agent1".into(),
            session_id: Some("mailbox-d-exp".into()),
            forwarded_text: "hi".into(),
            mention_sender: false,
            sender_minos_id: None,
            status: STATUS_PENDING.into(),
            attempts: 0,
            next_attempt_at_ms: now,
            last_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            lease_owner_host_id: None,
            lease_expires_at_ms: None,
            automation_hop: 0,
        };
        enqueue(&store, &row).await.unwrap();
        // Claim → inflight.
        let claimed = claim_due(&store, now, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        // Lease expires soon, but updated_at is fresh (would not reclaim on STALE alone).
        let lease_expires = now + 1_000;
        set_lease(&store, "d-exp", "host-1", lease_expires, now)
            .await
            .unwrap();
        // Touch updated_at by rebinding session (keeps updated_at recent).
        set_session_id(&store, "d-exp", "mailbox-d-exp", now + 500)
            .await
            .unwrap();
        // Before lease expiry: not reclaimed.
        assert!(claim_due(&store, now + 500, 10).await.unwrap().is_empty());
        // After lease expiry: reclaimed even though updated_at is within STALE window.
        let reclaimed = claim_due(&store, lease_expires + 1, 10).await.unwrap();
        assert_eq!(reclaimed.len(), 1);
        assert_eq!(reclaimed[0].dispatch_id, "d-exp");
        assert_eq!(reclaimed[0].status, STATUS_INFLIGHT);
        assert!(reclaimed[0].lease_owner_host_id.is_none());
        assert!(reclaimed[0].lease_expires_at_ms.is_none());
    }

    #[tokio::test]
    async fn force_due_for_accounts_makes_backoff_rows_claimable() {
        let store = mem().await;
        let now = 10_000_i64;
        let row = AgentDispatchRow {
            dispatch_id: "d1".into(),
            origin_message_id: "o1".into(),
            conversation_id: "c1".into(),
            account_id: "acct-a".into(),
            agent_id: "agent1".into(),
            session_id: None,
            forwarded_text: "hi".into(),
            mention_sender: false,
            sender_minos_id: None,
            status: STATUS_PENDING.into(),
            attempts: 2,
            next_attempt_at_ms: now + 60_000,
            last_error: Some("no live host".into()),
            created_at_ms: now,
            updated_at_ms: now,
            lease_owner_host_id: None,
            lease_expires_at_ms: None,
            automation_hop: 0,
        };
        enqueue(&store, &row).await.unwrap();
        assert!(claim_due(&store, now, 10).await.unwrap().is_empty());
        let n = force_due_for_accounts(&store, &["acct-a".into()], now)
            .await
            .unwrap();
        assert_eq!(n, 1);
        let claimed = claim_due(&store, now, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].origin_message_id, "o1");
    }
}
