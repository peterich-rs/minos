use serde_json::Value;

use crate::app::tx::DbTx;
use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type OutboxEventRowTuple = (
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

/// Outbox delivery lane. Social fanout and host commands claim/process independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutboxLane {
    /// Chat / account / reaction durable events: publish then ack.
    SocialDurable,
    /// Host RPC commands: publish then wait for host observation asynchronously;
    /// expiry → dead_letter (never success-ack).
    HostCommand,
}

impl OutboxLane {
    pub const SOCIAL_DURABLE: &'static str = "social_durable";
    pub const HOST_COMMAND: &'static str = "host_command";

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SocialDurable => Self::SOCIAL_DURABLE,
            Self::HostCommand => Self::HOST_COMMAND,
        }
    }

    pub fn parse(raw: &str) -> Result<Self, BackendError> {
        match raw {
            Self::SOCIAL_DURABLE => Ok(Self::SocialDurable),
            Self::HOST_COMMAND => Ok(Self::HostCommand),
            other => Err(BackendError::StoreDecode {
                column: "outbox_events.lane".into(),
                message: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatus {
    Pending,
    Claimed,
    Acked,
    Dead,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEventRow {
    pub outbox_id: String,
    pub topic_kind: String,
    pub event_id: String,
    pub lane: OutboxLane,
    pub status: OutboxStatus,
    pub available_at_ms: i64,
    pub attempts: u32,
    pub claimed_by: Option<String>,
    pub claimed_at_ms: Option<i64>,
    pub ack_at_ms: Option<i64>,
    pub dead_at_ms: Option<i64>,
    pub last_error_json: Option<Value>,
}

pub async fn enqueue(
    store: &impl AsStorePool,
    outbox_id: &str,
    topic_kind: &str,
    event_id: &str,
    lane: OutboxLane,
    available_at_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, lane, available_at_ms, attempts)
                 VALUES (?, ?, ?, 'pending', ?, ?, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
        .bind(lane.as_str())
        .bind(available_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, lane, available_at_ms, attempts)
                 VALUES ($1, $2, $3, 'pending', $4, $5, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
        .bind(lane.as_str())
        .bind(available_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("outbox_events::enqueue"))?;
    Ok(())
}

pub async fn enqueue_in_tx(
    tx: &mut DbTx<'_>,
    outbox_id: &str,
    topic_kind: &str,
    event_id: &str,
    lane: OutboxLane,
    available_at_ms: i64,
) -> Result<(), BackendError> {
    match tx {
        DbTx::Sqlite(tx) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, lane, available_at_ms, attempts)
                 VALUES (?, ?, ?, 'pending', ?, ?, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
        .bind(lane.as_str())
        .bind(available_at_ms)
        .execute(&mut **tx)
        .await
        .map(|_| ()),
        DbTx::Postgres(tx) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, lane, available_at_ms, attempts)
                 VALUES ($1, $2, $3, 'pending', $4, $5, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
        .bind(lane.as_str())
        .bind(available_at_ms)
        .execute(&mut **tx)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("outbox_events::enqueue_in_tx"))?;
    Ok(())
}

pub async fn get(
    store: &impl AsStorePool,
    outbox_id: &str,
) -> Result<Option<OutboxEventRow>, BackendError> {
    let row =
        match store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => sqlx::query_as::<_, OutboxEventRowTuple>(
                "SELECT outbox_id, topic_kind, event_id, lane, status, available_at_ms, attempts,
                        claimed_by, claimed_at_ms, ack_at_ms, dead_at_ms, last_error_json
                   FROM outbox_events
                  WHERE outbox_id = ?",
            )
            .bind(outbox_id)
            .fetch_optional(pool)
            .await,
            StorePoolRef::Postgres(pool) => sqlx::query_as::<_, OutboxEventRowTuple>(
                "SELECT outbox_id, topic_kind, event_id, lane, status, available_at_ms, attempts,
                        claimed_by, claimed_at_ms, ack_at_ms, dead_at_ms, last_error_json::text
                   FROM outbox_events
                  WHERE outbox_id = $1",
            )
            .bind(outbox_id)
            .fetch_optional(pool)
            .await,
        }
        .map_err(store_err("outbox_events::get"))?;

    row.map(decode_row).transpose()
}

/// Claim pending outbox rows for a single lane (SKIP LOCKED / transactional claim).
pub async fn claim_available(
    store: &impl AsStorePool,
    claimed_by: &str,
    now_ms: i64,
    limit: u32,
    lane: OutboxLane,
) -> Result<Vec<OutboxEventRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            claim_available_sqlite(pool, claimed_by, now_ms, limit, lane).await
        }
        StorePoolRef::Postgres(pool) => {
            claim_available_postgres(pool, claimed_by, now_ms, limit, lane).await
        }
    }
}

async fn claim_available_sqlite(
    pool: &sqlx::SqlitePool,
    claimed_by: &str,
    now_ms: i64,
    limit: u32,
    lane: OutboxLane,
) -> Result<Vec<OutboxEventRow>, BackendError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("outbox_events::claim_available.begin_sqlite"))?;

    let candidate_ids = sqlx::query_scalar::<_, String>(
        "SELECT outbox_id
           FROM outbox_events
          WHERE status = 'pending'
            AND lane = ?
            AND available_at_ms <= ?
          ORDER BY available_at_ms ASC, outbox_id ASC
          LIMIT ?",
    )
    .bind(lane.as_str())
    .bind(now_ms)
    .bind(i64::from(limit))
    .fetch_all(&mut *tx)
    .await
    .map_err(store_err("outbox_events::claim_available.select_sqlite"))?;

    let mut claimed = Vec::new();
    for outbox_id in candidate_ids {
        let result = sqlx::query(
            "UPDATE outbox_events
                SET status = 'claimed', attempts = attempts + 1, claimed_by = ?, claimed_at_ms = ?
              WHERE outbox_id = ?
                AND status = 'pending'
                AND lane = ?
                AND available_at_ms <= ?",
        )
        .bind(claimed_by)
        .bind(now_ms)
        .bind(&outbox_id)
        .bind(lane.as_str())
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(store_err("outbox_events::claim_available.update_sqlite"))?;

        if result.rows_affected() == 1 {
            let row = sqlx::query_as::<_, OutboxEventRowTuple>(
                "SELECT outbox_id, topic_kind, event_id, lane, status, available_at_ms, attempts,
                        claimed_by, claimed_at_ms, ack_at_ms, dead_at_ms, last_error_json
                   FROM outbox_events
                  WHERE outbox_id = ?",
            )
            .bind(&outbox_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_err("outbox_events::claim_available.fetch_sqlite"))?;
            claimed.push(decode_row(row)?);
        }
    }

    tx.commit()
        .await
        .map_err(store_err("outbox_events::claim_available.commit_sqlite"))?;
    Ok(claimed)
}

async fn claim_available_postgres(
    pool: &sqlx::PgPool,
    claimed_by: &str,
    now_ms: i64,
    limit: u32,
    lane: OutboxLane,
) -> Result<Vec<OutboxEventRow>, BackendError> {
    let rows = sqlx::query_as::<_, OutboxEventRowTuple>(
        "WITH cte AS (
             SELECT outbox_id
               FROM outbox_events
              WHERE status = 'pending'
                AND lane = $4
                AND available_at_ms <= $1
              ORDER BY available_at_ms ASC, outbox_id ASC
              LIMIT $2
              FOR UPDATE SKIP LOCKED
         )
         UPDATE outbox_events o
            SET status = 'claimed',
                attempts = o.attempts + 1,
                claimed_by = $3,
                claimed_at_ms = $1
           FROM cte
          WHERE o.outbox_id = cte.outbox_id
      RETURNING o.outbox_id,
                o.topic_kind,
                o.event_id,
                o.lane,
                o.status,
                o.available_at_ms,
                o.attempts,
                o.claimed_by,
                o.claimed_at_ms,
                o.ack_at_ms,
                o.dead_at_ms,
                o.last_error_json::text",
    )
    .bind(now_ms)
    .bind(i64::from(limit))
    .bind(claimed_by)
    .bind(lane.as_str())
    .fetch_all(pool)
    .await
    .map_err(store_err("outbox_events::claim_available.postgres"))?;

    rows.into_iter().map(decode_row).collect()
}

/// SQL predicate: host observed delivery (ack or non-timeout host terminal).
/// Backend `mark_timed_out` writes `error_json.kind = 'timeout'` and must not unlock ack.
const HOST_OBSERVED_SQLITE: &str = "(\
    h.ack_at_ms IS NOT NULL \
    OR h.status = 'succeeded' \
    OR (h.status = 'failed' AND COALESCE(json_extract(h.error_json, '$.kind'), '') != 'timeout')\
)";
const HOST_OBSERVED_POSTGRES: &str = "(\
    h.ack_at_ms IS NOT NULL \
    OR h.status = 'succeeded' \
    OR (h.status = 'failed' AND COALESCE(h.error_json ->> 'kind', '') <> 'timeout')\
)";

/// Ack an outbox row after durable publish succeeded.
///
/// Accepts **`pending` or `claimed`**: the HTTP fast-path publishes then settles
/// without a worker claim; the outbox worker settles after claim+publish.
/// Host-command rows still refuse ack until the host has observed the command
/// (`ack_at_ms` or non-timeout host terminal). Backend timeout `finished_at_ms`
/// never unlocks a success ack — use [`dead_letter`] / [`dead_letter_host_command_events`].
pub async fn ack(
    store: &impl AsStorePool,
    outbox_id: &str,
    ack_at_ms: i64,
) -> Result<bool, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(&format!(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = ?
              WHERE outbox_id = ?
                AND status IN ('pending', 'claimed')
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND NOT EXISTS (
                    SELECT 1
                      FROM durable_event_log d
                     WHERE d.topic_kind = outbox_events.topic_kind
                       AND d.event_id = outbox_events.event_id
                       AND json_extract(d.payload_json, '$.kind') = 'host_command_issued'
                       AND NOT EXISTS (
                           SELECT 1
                             FROM host_commands h
                            WHERE h.command_id = json_extract(d.payload_json, '$.command_id')
                              AND {HOST_OBSERVED_SQLITE}
                       )
                )"
        ))
        .bind(ack_at_ms)
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(&format!(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = $1
              WHERE outbox_id = $2
                AND status IN ('pending', 'claimed')
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND NOT EXISTS (
                    SELECT 1
                      FROM durable_event_log d
                     WHERE d.topic_kind = outbox_events.topic_kind
                       AND d.event_id = outbox_events.event_id
                       AND d.payload_json ->> 'kind' = 'host_command_issued'
                       AND NOT EXISTS (
                           SELECT 1
                             FROM host_commands h
                            WHERE h.command_id = d.payload_json ->> 'command_id'
                              AND {HOST_OBSERVED_POSTGRES}
                       )
                )"
        ))
        .bind(ack_at_ms)
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("outbox_events::ack"))?;
    if result == 1 {
        return Ok(true);
    }

    let already_acked = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
               FROM outbox_events
              WHERE outbox_id = ?
                AND status = 'acked'
                AND ack_at_ms IS NOT NULL
                AND dead_at_ms IS NULL",
        )
        .bind(outbox_id)
        .fetch_one(pool)
        .await
        .map(|count| count > 0),
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                 SELECT 1
                   FROM outbox_events
                  WHERE outbox_id = $1
                    AND status = 'acked'
                    AND ack_at_ms IS NOT NULL
                    AND dead_at_ms IS NULL
             )",
            )
            .bind(outbox_id)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("outbox_events::ack.check_acked"))?;
    Ok(already_acked)
}

pub async fn ack_pending_host_command_events(
    store: &impl AsStorePool,
    command_id: &str,
    ack_at_ms: i64,
) -> Result<u64, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(&format!(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = ?
              WHERE status IN ('pending', 'claimed')
                AND lane = 'host_command'
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND EXISTS (
                    SELECT 1
                      FROM durable_event_log d
                     WHERE d.topic_kind = outbox_events.topic_kind
                       AND d.event_id = outbox_events.event_id
                       AND json_extract(d.payload_json, '$.kind') = 'host_command_issued'
                       AND json_extract(d.payload_json, '$.command_id') = ?
                )
                AND EXISTS (
                    SELECT 1
                      FROM host_commands h
                     WHERE h.command_id = ?
                       AND {HOST_OBSERVED_SQLITE}
                )"
        ))
        .bind(ack_at_ms)
        .bind(command_id)
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(&format!(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = $1
              WHERE status IN ('pending', 'claimed')
                AND lane = 'host_command'
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND EXISTS (
                    SELECT 1
                      FROM durable_event_log d
                     WHERE d.topic_kind = outbox_events.topic_kind
                       AND d.event_id = outbox_events.event_id
                       AND d.payload_json ->> 'kind' = 'host_command_issued'
                       AND d.payload_json ->> 'command_id' = $2
                )
                AND EXISTS (
                    SELECT 1
                      FROM host_commands h
                     WHERE h.command_id = $3
                       AND {HOST_OBSERVED_POSTGRES}
                )"
        ))
        .bind(ack_at_ms)
        .bind(command_id)
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("outbox_events::ack_pending_host_command_events"))?;
    Ok(result)
}

/// Dead-letter host_command outbox rows for a command that expired without host observation.
pub async fn dead_letter_host_command_events(
    store: &impl AsStorePool,
    command_id: &str,
    dead_at_ms: i64,
    last_error_json: &Value,
) -> Result<u64, BackendError> {
    let last_error_json = serialize_json(
        last_error_json,
        "outbox_events::dead_letter_host_command_events.last_error_json",
    )?;
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'dead', dead_at_ms = ?, last_error_json = ?
              WHERE status IN ('pending', 'claimed')
                AND lane = 'host_command'
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND EXISTS (
                    SELECT 1
                      FROM durable_event_log d
                     WHERE d.topic_kind = outbox_events.topic_kind
                       AND d.event_id = outbox_events.event_id
                       AND json_extract(d.payload_json, '$.kind') = 'host_command_issued'
                       AND json_extract(d.payload_json, '$.command_id') = ?
                )",
        )
        .bind(dead_at_ms)
        .bind(last_error_json.as_str())
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'dead', dead_at_ms = $1, last_error_json = CAST($2 AS JSONB)
              WHERE status IN ('pending', 'claimed')
                AND lane = 'host_command'
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND EXISTS (
                    SELECT 1
                      FROM durable_event_log d
                     WHERE d.topic_kind = outbox_events.topic_kind
                       AND d.event_id = outbox_events.event_id
                       AND d.payload_json ->> 'kind' = 'host_command_issued'
                       AND d.payload_json ->> 'command_id' = $3
                )",
        )
        .bind(dead_at_ms)
        .bind(last_error_json.as_str())
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("outbox_events::dead_letter_host_command_events"))?;
    Ok(result)
}

/// Requeue abandoned claims for a single lane (lease-based multi-instance recovery).
pub async fn requeue_stale_claims(
    store: &impl AsStorePool,
    claimed_before_ms: i64,
    available_at_ms: i64,
    last_error_json: &Value,
    lane: OutboxLane,
) -> Result<u64, BackendError> {
    let last_error_json = serialize_json(
        last_error_json,
        "outbox_events::requeue_stale_claims.last_error_json",
    )?;
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'pending',
                    available_at_ms = ?,
                    claimed_by = NULL,
                    claimed_at_ms = NULL,
                    last_error_json = ?
              WHERE status = 'claimed'
                AND lane = ?
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND claimed_at_ms IS NOT NULL
                AND claimed_at_ms <= ?",
        )
        .bind(available_at_ms)
        .bind(last_error_json.as_str())
        .bind(lane.as_str())
        .bind(claimed_before_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'pending',
                    available_at_ms = $1,
                    claimed_by = NULL,
                    claimed_at_ms = NULL,
                    last_error_json = CAST($2 AS JSONB)
              WHERE status = 'claimed'
                AND lane = $3
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL
                AND claimed_at_ms IS NOT NULL
                AND claimed_at_ms <= $4",
        )
        .bind(available_at_ms)
        .bind(last_error_json.as_str())
        .bind(lane.as_str())
        .bind(claimed_before_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("outbox_events::requeue_stale_claims"))?;
    Ok(result)
}

pub async fn retry(
    store: &impl AsStorePool,
    outbox_id: &str,
    available_at_ms: i64,
    last_error_json: &Value,
) -> Result<bool, BackendError> {
    let last_error_json = serialize_json(last_error_json, "outbox_events::retry.last_error_json")?;
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'pending',
                    available_at_ms = ?,
                    claimed_by = NULL,
                    claimed_at_ms = NULL,
                    last_error_json = ?
              WHERE outbox_id = ?
                AND status = 'claimed'
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL",
        )
        .bind(available_at_ms)
        .bind(last_error_json.as_str())
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'pending',
                    available_at_ms = $1,
                    claimed_by = NULL,
                    claimed_at_ms = NULL,
                    last_error_json = CAST($2 AS JSONB)
              WHERE outbox_id = $3
                AND status = 'claimed'
                AND ack_at_ms IS NULL
                AND dead_at_ms IS NULL",
        )
        .bind(available_at_ms)
        .bind(last_error_json.as_str())
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("outbox_events::retry"))?;
    Ok(result == 1)
}

pub async fn dead_letter(
    store: &impl AsStorePool,
    outbox_id: &str,
    dead_at_ms: i64,
    last_error_json: &Value,
) -> Result<bool, BackendError> {
    let last_error_json = serialize_json(
        last_error_json,
        "outbox_events::dead_letter.last_error_json",
    )?;
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'dead', dead_at_ms = ?, last_error_json = ?
              WHERE outbox_id = ?
                AND dead_at_ms IS NULL
                AND ack_at_ms IS NULL",
        )
        .bind(dead_at_ms)
        .bind(last_error_json.as_str())
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'dead', dead_at_ms = $1, last_error_json = CAST($2 AS JSONB)
              WHERE outbox_id = $3
                AND dead_at_ms IS NULL
                AND ack_at_ms IS NULL",
        )
        .bind(dead_at_ms)
        .bind(last_error_json.as_str())
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("outbox_events::dead_letter"))?;
    Ok(result == 1)
}

fn decode_row(row: OutboxEventRowTuple) -> Result<OutboxEventRow, BackendError> {
    let (
        outbox_id,
        topic_kind,
        event_id,
        lane,
        status,
        available_at_ms,
        attempts,
        claimed_by,
        claimed_at_ms,
        ack_at_ms,
        dead_at_ms,
        last_error_json,
    ) = row;

    Ok(OutboxEventRow {
        outbox_id,
        topic_kind,
        event_id,
        lane: OutboxLane::parse(&lane)?,
        status: parse_status(&status)?,
        available_at_ms,
        attempts: u32::try_from(attempts).map_err(|error| BackendError::StoreDecode {
            column: "outbox_events.attempts".into(),
            message: error.to_string(),
        })?,
        claimed_by,
        claimed_at_ms,
        ack_at_ms,
        dead_at_ms,
        last_error_json: last_error_json
            .map(|raw| {
                serde_json::from_str(&raw).map_err(|error| BackendError::StoreDecode {
                    column: "outbox_events.last_error_json".into(),
                    message: error.to_string(),
                })
            })
            .transpose()?,
    })
}

fn serialize_json(value: &Value, operation: &'static str) -> Result<String, BackendError> {
    serde_json::to_string(value).map_err(|error| BackendError::StoreQuery {
        operation: operation.into(),
        message: error.to_string(),
    })
}

fn parse_status(status: &str) -> Result<OutboxStatus, BackendError> {
    match status {
        "pending" => Ok(OutboxStatus::Pending),
        "claimed" => Ok(OutboxStatus::Claimed),
        "acked" => Ok(OutboxStatus::Acked),
        "dead" => Ok(OutboxStatus::Dead),
        other => Err(BackendError::StoreDecode {
            column: "outbox_events.status".into(),
            message: other.to_string(),
        }),
    }
}

fn store_err(operation: &'static str) -> impl FnOnce(sqlx::Error) -> BackendError {
    move |error| BackendError::StoreQuery {
        operation: operation.into(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::device_installations;
    use crate::store::durable_event_log;
    use crate::store::host_commands;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::{DeviceId, DeviceRole};

    #[tokio::test]
    async fn ack_settles_pending_social_row_without_claim() {
        // HTTP fast-path: publish then ack while still pending (no worker claim).
        let pool = memory_pool().await;
        durable_event_log::append(
            &pool,
            "evt-pending-ack",
            "conversation:c1",
            "conversation",
            1,
            "c1",
            &serde_json::json!({ "kind": "conversation.message_appended" }),
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-pending-ack",
            "conversation",
            "evt-pending-ack",
            OutboxLane::SocialDurable,
            T0,
        )
        .await
        .unwrap();
        let row = get(&pool, "out-pending-ack").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Pending);
        assert!(ack(&pool, "out-pending-ack", T0 + 1).await.unwrap());
        let row = get(&pool, "out-pending-ack").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert!(
            claim_available(&pool, "worker", T0 + 2, 10, OutboxLane::SocialDurable,)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn claim_retry_and_ack_round_trip() {
        let pool = memory_pool().await;
        durable_event_log::append(
            &pool,
            "evt-1",
            "host:dev1",
            "host",
            1,
            "dev1",
            &serde_json::json!({ "kind": "host.command.created" }),
            T0,
        )
        .await
        .unwrap();
        durable_event_log::append(
            &pool,
            "evt-2",
            "host:dev1",
            "host",
            2,
            "dev1",
            &serde_json::json!({ "kind": "host.command.acked" }),
            T0 + 1,
        )
        .await
        .unwrap();

        enqueue(
            &pool,
            "out-1",
            "host",
            "evt-1",
            OutboxLane::SocialDurable,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-2",
            "host",
            "evt-2",
            OutboxLane::SocialDurable,
            T0 + 500,
        )
        .await
        .unwrap();

        let claimed = claim_available(&pool, "worker-1", T0, 10, OutboxLane::SocialDurable)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].outbox_id, "out-1");
        assert_eq!(claimed[0].topic_kind, "host");
        assert_eq!(claimed[0].lane, OutboxLane::SocialDurable);
        assert_eq!(claimed[0].status, OutboxStatus::Claimed);
        assert_eq!(claimed[0].attempts, 1);
        assert_eq!(claimed[0].claimed_by.as_deref(), Some("worker-1"));

        assert!(retry(
            &pool,
            "out-1",
            T0 + 250,
            &serde_json::json!({ "kind": "temporary" })
        )
        .await
        .unwrap());
        assert!(
            claim_available(&pool, "worker-1", T0 + 200, 10, OutboxLane::SocialDurable,)
                .await
                .unwrap()
                .is_empty()
        );

        let claimed = claim_available(&pool, "worker-2", T0 + 300, 10, OutboxLane::SocialDurable)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].outbox_id, "out-1");
        assert_eq!(claimed[0].attempts, 2);
        assert_eq!(claimed[0].claimed_by.as_deref(), Some("worker-2"));

        assert!(ack(&pool, "out-1", T0 + 301).await.unwrap());
        assert!(ack(&pool, "out-1", T0 + 302).await.unwrap());
        assert!(
            claim_available(&pool, "worker-3", T0 + 1_000, 10, OutboxLane::SocialDurable,)
                .await
                .unwrap()
                .iter()
                .all(|row| row.outbox_id != "out-1")
        );

        let row = get(&pool, "out-1").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert_eq!(row.ack_at_ms, Some(T0 + 301));
        assert_eq!(
            row.last_error_json,
            Some(serde_json::json!({ "kind": "temporary" }))
        );
    }

    #[tokio::test]
    async fn claim_available_isolates_lanes() {
        let pool = memory_pool().await;
        durable_event_log::append(
            &pool,
            "evt-social",
            "account:acc1",
            "account",
            1,
            "acc1",
            &serde_json::json!({ "kind": "account_registered", "account_id": "acc1", "at_ms": T0 }),
            T0,
        )
        .await
        .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host",
            "host:dev1",
            "host",
            1,
            "dev1",
            &serde_json::json!({ "kind": "host_command_issued", "command_id": "cmd-x" }),
            T0,
        )
        .await
        .unwrap();

        enqueue(
            &pool,
            "out-social",
            "account",
            "evt-social",
            OutboxLane::SocialDurable,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host",
            "host",
            "evt-host",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();

        let social = claim_available(&pool, "social-worker", T0, 10, OutboxLane::SocialDurable)
            .await
            .unwrap();
        assert_eq!(social.len(), 1);
        assert_eq!(social[0].outbox_id, "out-social");
        assert_eq!(social[0].lane, OutboxLane::SocialDurable);

        let host = claim_available(&pool, "host-worker", T0, 10, OutboxLane::HostCommand)
            .await
            .unwrap();
        assert_eq!(host.len(), 1);
        assert_eq!(host[0].outbox_id, "out-host");
        assert_eq!(host[0].lane, OutboxLane::HostCommand);

        // Social claim must not drain host_command rows and vice versa.
        assert!(
            claim_available(&pool, "social-worker-2", T0, 10, OutboxLane::SocialDurable,)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            claim_available(&pool, "host-worker-2", T0, 10, OutboxLane::HostCommand,)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn requeue_stale_claims_is_lane_scoped() {
        let pool = memory_pool().await;
        durable_event_log::append(
            &pool,
            "evt-social-stale",
            "account:acc1",
            "account",
            1,
            "acc1",
            &serde_json::json!({ "kind": "account_registered", "account_id": "acc1", "at_ms": T0 }),
            T0,
        )
        .await
        .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host-stale",
            "host:dev1",
            "host",
            1,
            "dev1",
            &serde_json::json!({ "kind": "host_command_issued" }),
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-social-stale",
            "account",
            "evt-social-stale",
            OutboxLane::SocialDurable,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host-stale",
            "host",
            "evt-host-stale",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        assert_eq!(
            claim_available(&pool, "w-social", T0, 10, OutboxLane::SocialDurable,)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            claim_available(&pool, "w-host", T0, 10, OutboxLane::HostCommand)
                .await
                .unwrap()
                .len(),
            1
        );

        assert_eq!(
            requeue_stale_claims(
                &pool,
                T0 + 30_000,
                T0 + 30_001,
                &serde_json::json!({ "kind": "claim_recovered" }),
                OutboxLane::SocialDurable,
            )
            .await
            .unwrap(),
            1
        );

        let social = get(&pool, "out-social-stale").await.unwrap().unwrap();
        assert_eq!(social.status, OutboxStatus::Pending);
        let host = get(&pool, "out-host-stale").await.unwrap().unwrap();
        assert_eq!(host.status, OutboxStatus::Claimed);
    }

    #[tokio::test]
    async fn ack_refuses_host_command_until_command_is_observed() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host-command",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-1",
                "host_installation_id": host_id.to_string(),
                "method": "minos_health",
                "params": null,
                "deadline_at_ms": T0 + 1_000,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-1",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 1_000,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host-command",
            "host",
            "evt-host-command",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();

        let claimed = claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(!ack(&pool, "out-host-command", T0 + 1).await.unwrap());

        let row = get(&pool, "out-host-command").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Claimed);
        assert_eq!(row.ack_at_ms, None);

        assert!(host_commands::ack(&pool, "cmd-1", T0 + 2).await.unwrap());
        assert!(ack(&pool, "out-host-command", T0 + 3).await.unwrap());

        let row = get(&pool, "out-host-command").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert_eq!(row.ack_at_ms, Some(T0 + 3));
    }

    #[tokio::test]
    async fn expired_host_command_refuses_success_ack_and_dead_letters() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host-command-expired",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-expired",
                "host_installation_id": host_id.to_string(),
                "method": "minos_health",
                "params": null,
                "deadline_at_ms": T0 + 10,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host-command-expired",
            "host",
            "evt-host-command-expired",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();

        let claimed = claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        // Past deadline must never unlock a success ack.
        assert!(!ack(&pool, "out-host-command-expired", T0 + 11)
            .await
            .unwrap());

        assert!(dead_letter(
            &pool,
            "out-host-command-expired",
            T0 + 11,
            &serde_json::json!({ "kind": "host_command_expired" })
        )
        .await
        .unwrap());

        let row = get(&pool, "out-host-command-expired")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, OutboxStatus::Dead);
        assert_eq!(row.dead_at_ms, Some(T0 + 11));
        assert_eq!(row.ack_at_ms, None);
    }

    #[tokio::test]
    async fn backend_timeout_finished_does_not_unlock_outbox_ack_or_ack_pending() {
        // Observation must not treat mark_timed_out finished_at_ms as success unlock.
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-timeout-no-unlock",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-timeout-no-unlock",
                "host_installation_id": host_id.to_string(),
                "method": "minos_health",
                "params": null,
                "deadline_at_ms": T0 + 10,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-timeout-no-unlock",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 10,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-timeout-no-unlock",
            "host",
            "evt-timeout-no-unlock",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        let claimed = claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);

        // mark_timed_out alone (simulates race before dead_letter): still must not ack.
        assert!(host_commands::mark_timed_out(
            &pool,
            "cmd-timeout-no-unlock",
            &serde_json::json!({ "kind": "timeout", "timeout_ms": 10 }),
            T0 + 20,
        )
        .await
        .unwrap());
        let cmd = host_commands::get(&pool, "cmd-timeout-no-unlock")
            .await
            .unwrap()
            .unwrap();
        assert!(cmd.finished_at_ms.is_some());
        assert!(cmd.is_backend_timeout());
        assert!(!cmd.is_host_observed());

        assert!(!ack(&pool, "out-timeout-no-unlock", T0 + 21).await.unwrap());
        assert_eq!(
            ack_pending_host_command_events(&pool, "cmd-timeout-no-unlock", T0 + 22)
                .await
                .unwrap(),
            0
        );
        let row = get(&pool, "out-timeout-no-unlock").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Claimed);
        assert_eq!(row.ack_at_ms, None);
        assert_eq!(row.dead_at_ms, None);
    }

    #[tokio::test]
    async fn observed_host_ack_past_deadline_settles_outbox_acked_not_dead() {
        // Host acked, outbox still claimed, deadline passed → success-ack settle (not DL).
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-obs-past-deadline",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-obs-past-deadline",
                "host_installation_id": host_id.to_string(),
                "method": "minos_health",
                "params": null,
                "deadline_at_ms": T0 + 10,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-obs-past-deadline",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 10,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-obs-past-deadline",
            "host",
            "evt-obs-past-deadline",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        assert_eq!(
            claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(host_commands::ack(&pool, "cmd-obs-past-deadline", T0 + 5)
            .await
            .unwrap());
        let cmd = host_commands::get(&pool, "cmd-obs-past-deadline")
            .await
            .unwrap()
            .unwrap();
        assert!(cmd.is_host_observed());
        assert!(cmd.finished_at_ms.is_none());

        // Past deadline but observed: ack unlocks (same rule as dispatch observation-first).
        assert!(ack(&pool, "out-obs-past-deadline", T0 + 20).await.unwrap());
        let row = get(&pool, "out-obs-past-deadline").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert_eq!(row.ack_at_ms, Some(T0 + 20));
        assert_eq!(row.dead_at_ms, None);
    }

    #[tokio::test]
    async fn expire_with_host_ack_settles_outbox_acked_and_times_out_command() {
        // expire path: ack_at_ms set, unfinished, deadline passed → outbox acked, command timed out.
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-expire-obs",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-expire-obs",
                "deadline_at_ms": T0 + 10,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-expire-obs",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 10,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-expire-obs",
            "host",
            "evt-expire-obs",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        assert_eq!(
            claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(host_commands::ack(&pool, "cmd-expire-obs", T0 + 5)
            .await
            .unwrap());

        assert!(crate::host_commands::expire_command_if_deadline_passed(
            &pool,
            "cmd-expire-obs",
            &serde_json::json!({ "kind": "timeout", "timeout_ms": 10 }),
            T0 + 20,
        )
        .await
        .unwrap());

        let row = get(&pool, "out-expire-obs").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert_eq!(row.ack_at_ms, Some(T0 + 20));
        assert_eq!(row.dead_at_ms, None);

        let cmd = host_commands::get(&pool, "cmd-expire-obs")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cmd.finished_at_ms, Some(T0 + 20));
        assert!(cmd.is_backend_timeout());
        // Host ack still counts as observation even after backend timeout finish.
        assert!(cmd.ack_at_ms.is_some());
        assert!(cmd.is_host_observed());
    }

    #[tokio::test]
    async fn expire_deadline_dead_letters_outbox_before_mark_timed_out() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-expire-dl",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-expire-dl",
                "deadline_at_ms": T0 + 10,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-expire-dl",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 10,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-expire-dl",
            "host",
            "evt-expire-dl",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        assert_eq!(
            claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
                .await
                .unwrap()
                .len(),
            1
        );

        assert!(crate::host_commands::expire_command_if_deadline_passed(
            &pool,
            "cmd-expire-dl",
            &serde_json::json!({ "kind": "timeout", "timeout_ms": 10 }),
            T0 + 20,
        )
        .await
        .unwrap());

        let row = get(&pool, "out-expire-dl").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Dead);
        assert_eq!(row.dead_at_ms, Some(T0 + 20));
        assert_eq!(row.ack_at_ms, None);
        assert!(!ack(&pool, "out-expire-dl", T0 + 21).await.unwrap());
        assert_eq!(
            ack_pending_host_command_events(&pool, "cmd-expire-dl", T0 + 22)
                .await
                .unwrap(),
            0
        );

        let cmd = host_commands::get(&pool, "cmd-expire-dl")
            .await
            .unwrap()
            .unwrap();
        assert!(cmd.is_backend_timeout());
        assert!(!cmd.is_host_observed());
        assert_eq!(cmd.finished_at_ms, Some(T0 + 20));
    }

    #[tokio::test]
    async fn host_failed_result_without_timeout_kind_unlocks_ack_pending() {
        // Legitimate host terminal (failed) is observation; timeout is not.
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host-failed",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-host-failed",
                "deadline_at_ms": T0 + 1_000,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-host-failed",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 1_000,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host-failed",
            "host",
            "evt-host-failed",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        let _ = claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
            .await
            .unwrap();

        assert!(host_commands::finish(
            &pool,
            "cmd-host-failed",
            host_commands::HostCommandTerminalStatus::Failed,
            None,
            Some(&serde_json::json!({ "status": "failed", "message": "boom" })),
            T0 + 50,
        )
        .await
        .unwrap());
        let cmd = host_commands::get(&pool, "cmd-host-failed")
            .await
            .unwrap()
            .unwrap();
        assert!(cmd.is_host_observed());
        assert!(!cmd.is_backend_timeout());

        assert_eq!(
            ack_pending_host_command_events(&pool, "cmd-host-failed", T0 + 51)
                .await
                .unwrap(),
            1
        );
        let row = get(&pool, "out-host-failed").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
    }

    #[tokio::test]
    async fn dead_letter_host_command_events_marks_expired_command_outbox() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host-command-dl",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-dl",
                "deadline_at_ms": T0 + 10,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host-command-dl",
            "host",
            "evt-host-command-dl",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        let _ = claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
            .await
            .unwrap();

        assert_eq!(
            dead_letter_host_command_events(
                &pool,
                "cmd-dl",
                T0 + 20,
                &serde_json::json!({ "kind": "host_command_expired" })
            )
            .await
            .unwrap(),
            1
        );
        let row = get(&pool, "out-host-command-dl").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Dead);
    }

    #[tokio::test]
    async fn ack_pending_host_command_events_marks_observed_command_outbox() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host-command-pending",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-pending",
                "host_installation_id": host_id.to_string(),
                "method": "minos_health",
                "params": null,
                "deadline_at_ms": T0 + 1_000,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-pending",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 1_000,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host-command-pending",
            "host",
            "evt-host-command-pending",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();

        assert_eq!(
            ack_pending_host_command_events(&pool, "cmd-pending", T0 + 1)
                .await
                .unwrap(),
            0
        );
        assert!(host_commands::ack(&pool, "cmd-pending", T0 + 2)
            .await
            .unwrap());
        assert_eq!(
            ack_pending_host_command_events(&pool, "cmd-pending", T0 + 3)
                .await
                .unwrap(),
            1
        );

        let row = get(&pool, "out-host-command-pending")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert_eq!(row.ack_at_ms, Some(T0 + 3));
    }

    #[tokio::test]
    async fn ack_pending_host_command_events_marks_claimed_observed_command_outbox() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        device_installations::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        durable_event_log::append(
            &pool,
            "evt-host-command-claimed",
            &format!("host:{host_id}"),
            "host",
            1,
            &host_id.to_string(),
            &serde_json::json!({
                "kind": "host_command_issued",
                "command_id": "cmd-claimed",
                "host_installation_id": host_id.to_string(),
                "method": "minos_health",
                "params": null,
                "deadline_at_ms": T0 + 1_000,
                "at_ms": T0
            }),
            T0,
        )
        .await
        .unwrap();
        host_commands::enqueue(
            &pool,
            "cmd-claimed",
            host_id,
            None,
            "minos_health",
            &serde_json::Value::Null,
            None,
            T0 + 1_000,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-host-command-claimed",
            "host",
            "evt-host-command-claimed",
            OutboxLane::HostCommand,
            T0,
        )
        .await
        .unwrap();
        let claimed = claim_available(&pool, "worker-1", T0, 10, OutboxLane::HostCommand)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(host_commands::ack(&pool, "cmd-claimed", T0 + 2)
            .await
            .unwrap());

        assert_eq!(
            ack_pending_host_command_events(&pool, "cmd-claimed", T0 + 3)
                .await
                .unwrap(),
            1
        );

        let row = get(&pool, "out-host-command-claimed")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert_eq!(row.ack_at_ms, Some(T0 + 3));
        assert!(ack(&pool, "out-host-command-claimed", T0 + 4)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn requeue_stale_claims_restores_abandoned_claims() {
        let pool = memory_pool().await;
        durable_event_log::append(
            &pool,
            "evt-stale-claim",
            "account:acc1",
            "account",
            1,
            "acc1",
            &serde_json::json!({ "kind": "account_registered", "account_id": "acc1", "at_ms": T0 }),
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-stale-claim",
            "account",
            "evt-stale-claim",
            OutboxLane::SocialDurable,
            T0,
        )
        .await
        .unwrap();
        let claimed = claim_available(&pool, "bad-worker", T0, 10, OutboxLane::SocialDurable)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);

        assert_eq!(
            requeue_stale_claims(
                &pool,
                T0 + 30_000,
                T0 + 30_001,
                &serde_json::json!({ "kind": "claim_recovered" }),
                OutboxLane::SocialDurable,
            )
            .await
            .unwrap(),
            1
        );
        let row = get(&pool, "out-stale-claim").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Pending);
        assert_eq!(row.claimed_by, None);
        assert_eq!(row.claimed_at_ms, None);

        let claimed = claim_available(
            &pool,
            "realtime-worker",
            T0 + 30_001,
            10,
            OutboxLane::SocialDurable,
        )
        .await
        .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].claimed_by.as_deref(), Some("realtime-worker"));
    }

    #[tokio::test]
    async fn dead_letter_records_last_error() {
        let pool = memory_pool().await;
        durable_event_log::append(
            &pool,
            "evt-dead",
            "host:dev1",
            "host",
            1,
            "dev1",
            &serde_json::json!({ "kind": "host.command.created" }),
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "out-dead",
            "host",
            "evt-dead",
            OutboxLane::SocialDurable,
            T0,
        )
        .await
        .unwrap();

        assert!(dead_letter(
            &pool,
            "out-dead",
            T0 + 5,
            &serde_json::json!({ "kind": "orphan" })
        )
        .await
        .unwrap());

        let row = get(&pool, "out-dead").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Dead);
        assert_eq!(row.dead_at_ms, Some(T0 + 5));
        assert_eq!(
            row.last_error_json,
            Some(serde_json::json!({ "kind": "orphan" }))
        );
    }
}
