use serde_json::Value;

use crate::app::tx::DbTx;
use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type OutboxEventRowTuple = (
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
    available_at_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, available_at_ms, attempts)
                 VALUES (?, ?, ?, 'pending', ?, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
        .bind(available_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, available_at_ms, attempts)
                 VALUES ($1, $2, $3, 'pending', $4, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
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
    available_at_ms: i64,
) -> Result<(), BackendError> {
    match tx {
        DbTx::Sqlite(tx) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, available_at_ms, attempts)
                 VALUES (?, ?, ?, 'pending', ?, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
        .bind(available_at_ms)
        .execute(&mut **tx)
        .await
        .map(|_| ()),
        DbTx::Postgres(tx) => sqlx::query(
            "INSERT INTO outbox_events
                    (outbox_id, topic_kind, event_id, status, available_at_ms, attempts)
                 VALUES ($1, $2, $3, 'pending', $4, 0)",
        )
        .bind(outbox_id)
        .bind(topic_kind)
        .bind(event_id)
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
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, OutboxEventRowTuple>(
                "SELECT outbox_id, topic_kind, event_id, status, available_at_ms, attempts,
                        claimed_by, claimed_at_ms, ack_at_ms, dead_at_ms, last_error_json
                   FROM outbox_events
                  WHERE outbox_id = ?",
            )
            .bind(outbox_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, OutboxEventRowTuple>(
                "SELECT outbox_id, topic_kind, event_id, status, available_at_ms, attempts,
                        claimed_by, claimed_at_ms, ack_at_ms, dead_at_ms, last_error_json::text
                   FROM outbox_events
                  WHERE outbox_id = $1",
            )
            .bind(outbox_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("outbox_events::get"))?;

    row.map(decode_row).transpose()
}

pub async fn claim_available(
    store: &impl AsStorePool,
    claimed_by: &str,
    now_ms: i64,
    limit: u32,
) -> Result<Vec<OutboxEventRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => claim_available_sqlite(pool, claimed_by, now_ms, limit).await,
        StorePoolRef::Postgres(pool) => {
            claim_available_postgres(pool, claimed_by, now_ms, limit).await
        }
    }
}

async fn claim_available_sqlite(
    pool: &sqlx::SqlitePool,
    claimed_by: &str,
    now_ms: i64,
    limit: u32,
) -> Result<Vec<OutboxEventRow>, BackendError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("outbox_events::claim_available.begin_sqlite"))?;

    let candidate_ids = sqlx::query_scalar::<_, String>(
        "SELECT outbox_id
           FROM outbox_events
          WHERE status = 'pending'
            AND available_at_ms <= ?
          ORDER BY available_at_ms ASC, outbox_id ASC
          LIMIT ?",
    )
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
                AND available_at_ms <= ?",
        )
        .bind(claimed_by)
        .bind(now_ms)
        .bind(&outbox_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(store_err("outbox_events::claim_available.update_sqlite"))?;

        if result.rows_affected() == 1 {
            let row = sqlx::query_as::<_, OutboxEventRowTuple>(
                "SELECT outbox_id, topic_kind, event_id, status, available_at_ms, attempts,
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
) -> Result<Vec<OutboxEventRow>, BackendError> {
    let rows = sqlx::query_as::<_, OutboxEventRowTuple>(
        "WITH cte AS (
             SELECT outbox_id
               FROM outbox_events
              WHERE status = 'pending'
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
    .fetch_all(pool)
    .await
    .map_err(store_err("outbox_events::claim_available.postgres"))?;

    rows.into_iter().map(decode_row).collect()
}

pub async fn ack(
    store: &impl AsStorePool,
    outbox_id: &str,
    ack_at_ms: i64,
) -> Result<bool, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = ?
              WHERE outbox_id = ?
                AND status = 'claimed'
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
                              AND (h.ack_at_ms IS NOT NULL OR h.finished_at_ms IS NOT NULL)
                       )
                )",
        )
        .bind(ack_at_ms)
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = $1
              WHERE outbox_id = $2
                AND status = 'claimed'
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
                              AND (h.ack_at_ms IS NOT NULL OR h.finished_at_ms IS NOT NULL)
                       )
                )",
        )
        .bind(ack_at_ms)
        .bind(outbox_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("outbox_events::ack"))?;
    Ok(result == 1)
}

pub async fn ack_pending_host_command_events(
    store: &impl AsStorePool,
    command_id: &str,
    ack_at_ms: i64,
) -> Result<u64, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = ?
              WHERE status = 'pending'
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
                       AND (h.ack_at_ms IS NOT NULL OR h.finished_at_ms IS NOT NULL)
                )",
        )
        .bind(ack_at_ms)
        .bind(command_id)
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE outbox_events
                SET status = 'acked', ack_at_ms = $1
              WHERE status = 'pending'
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
                       AND (h.ack_at_ms IS NOT NULL OR h.finished_at_ms IS NOT NULL)
                )",
        )
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
    use crate::store::devices;
    use crate::store::durable_event_log;
    use crate::store::host_commands;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::{DeviceId, DeviceRole};

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

        enqueue(&pool, "out-1", "host", "evt-1", T0).await.unwrap();
        enqueue(&pool, "out-2", "host", "evt-2", T0 + 500)
            .await
            .unwrap();

        let claimed = claim_available(&pool, "worker-1", T0, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].outbox_id, "out-1");
        assert_eq!(claimed[0].topic_kind, "host");
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
        assert!(claim_available(&pool, "worker-1", T0 + 200, 10)
            .await
            .unwrap()
            .is_empty());

        let claimed = claim_available(&pool, "worker-2", T0 + 300, 10)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].outbox_id, "out-1");
        assert_eq!(claimed[0].attempts, 2);
        assert_eq!(claimed[0].claimed_by.as_deref(), Some("worker-2"));

        assert!(ack(&pool, "out-1", T0 + 301).await.unwrap());
        assert!(!ack(&pool, "out-1", T0 + 302).await.unwrap());
        assert!(claim_available(&pool, "worker-3", T0 + 1_000, 10)
            .await
            .unwrap()
            .iter()
            .all(|row| row.outbox_id != "out-1"));

        let row = get(&pool, "out-1").await.unwrap().unwrap();
        assert_eq!(row.status, OutboxStatus::Acked);
        assert_eq!(row.ack_at_ms, Some(T0 + 301));
        assert_eq!(
            row.last_error_json,
            Some(serde_json::json!({ "kind": "temporary" }))
        );
    }

    #[tokio::test]
    async fn ack_refuses_host_command_until_command_is_observed() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        devices::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
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
        enqueue(&pool, "out-host-command", "host", "evt-host-command", T0)
            .await
            .unwrap();

        let claimed = claim_available(&pool, "worker-1", T0, 10).await.unwrap();
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
    async fn ack_pending_host_command_events_marks_observed_command_outbox() {
        let pool = memory_pool().await;
        let host_id = DeviceId::new();
        devices::insert_device(&pool, host_id, "Test Mac", DeviceRole::AgentHost, T0)
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
        enqueue(&pool, "out-dead", "host", "evt-dead", T0)
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
