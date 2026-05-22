use sqlx::SqlitePool;

use crate::error::BackendError;

type OutboxEventRowTuple = (
    String,
    String,
    String,
    i64,
    i64,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatus {
    Pending,
    Claimed,
    Acked,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEventRow {
    pub outbox_id: String,
    pub event_id: String,
    pub status: OutboxStatus,
    pub available_at_ms: i64,
    pub attempts: u32,
    pub claimed_by: Option<String>,
    pub claimed_at_ms: Option<i64>,
    pub ack_at_ms: Option<i64>,
    pub dead_at_ms: Option<i64>,
}

pub async fn enqueue(
    pool: &SqlitePool,
    outbox_id: &str,
    event_id: &str,
    available_at_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO outbox_events
            (outbox_id, event_id, status, available_at_ms, attempts)
         VALUES (?, ?, 'pending', ?, 0)",
    )
    .bind(outbox_id)
    .bind(event_id)
    .bind(available_at_ms)
    .execute(pool)
    .await
    .map_err(store_err("outbox_events::enqueue"))?;
    Ok(())
}

pub async fn get(
    pool: &SqlitePool,
    outbox_id: &str,
) -> Result<Option<OutboxEventRow>, BackendError> {
    let row = sqlx::query_as::<_, OutboxEventRowTuple>(
        "SELECT outbox_id, event_id, status, available_at_ms, attempts,
                claimed_by, claimed_at_ms, ack_at_ms, dead_at_ms
           FROM outbox_events
          WHERE outbox_id = ?",
    )
    .bind(outbox_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("outbox_events::get"))?;

    row.map(decode_row).transpose()
}

pub async fn claim_available(
    pool: &SqlitePool,
    claimed_by: &str,
    now_ms: i64,
    limit: u32,
) -> Result<Vec<OutboxEventRow>, BackendError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("outbox_events::claim_available.begin"))?;

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
    .map_err(store_err("outbox_events::claim_available.select"))?;

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
        .map_err(store_err("outbox_events::claim_available.update"))?;

        if result.rows_affected() == 1 {
            let row = sqlx::query_as::<_, OutboxEventRowTuple>(
                "SELECT outbox_id, event_id, status, available_at_ms, attempts,
                        claimed_by, claimed_at_ms, ack_at_ms, dead_at_ms
                   FROM outbox_events
                  WHERE outbox_id = ?",
            )
            .bind(&outbox_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_err("outbox_events::claim_available.fetch_claimed"))?;
            claimed.push(decode_row(row)?);
        }
    }

    tx.commit()
        .await
        .map_err(store_err("outbox_events::claim_available.commit"))?;
    Ok(claimed)
}

pub async fn ack(pool: &SqlitePool, outbox_id: &str, ack_at_ms: i64) -> Result<bool, BackendError> {
    let result = sqlx::query(
        "UPDATE outbox_events
            SET status = 'acked', ack_at_ms = ?
          WHERE outbox_id = ?
            AND status = 'claimed'
            AND ack_at_ms IS NULL
            AND dead_at_ms IS NULL",
    )
    .bind(ack_at_ms)
    .bind(outbox_id)
    .execute(pool)
    .await
    .map_err(store_err("outbox_events::ack"))?;
    Ok(result.rows_affected() == 1)
}

pub async fn retry(
    pool: &SqlitePool,
    outbox_id: &str,
    available_at_ms: i64,
) -> Result<bool, BackendError> {
    let result = sqlx::query(
        "UPDATE outbox_events
            SET status = 'pending', available_at_ms = ?, claimed_by = NULL, claimed_at_ms = NULL
          WHERE outbox_id = ?
            AND status = 'claimed'
            AND ack_at_ms IS NULL
            AND dead_at_ms IS NULL",
    )
    .bind(available_at_ms)
    .bind(outbox_id)
    .execute(pool)
    .await
    .map_err(store_err("outbox_events::retry"))?;
    Ok(result.rows_affected() == 1)
}

fn decode_row(row: OutboxEventRowTuple) -> Result<OutboxEventRow, BackendError> {
    let (
        outbox_id,
        event_id,
        status,
        available_at_ms,
        attempts,
        claimed_by,
        claimed_at_ms,
        ack_at_ms,
        dead_at_ms,
    ) = row;

    Ok(OutboxEventRow {
        outbox_id,
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
    use crate::store::durable_event_log;
    use crate::store::test_support::{memory_pool, T0};

    #[tokio::test]
    async fn claim_retry_and_ack_round_trip() {
        let pool = memory_pool().await;
        durable_event_log::append(
            &pool,
            "evt-1",
            "host:dev1",
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
            2,
            "dev1",
            &serde_json::json!({ "kind": "host.command.acked" }),
            T0 + 1,
        )
        .await
        .unwrap();

        enqueue(&pool, "out-1", "evt-1", T0).await.unwrap();
        enqueue(&pool, "out-2", "evt-2", T0 + 500).await.unwrap();

        let claimed = claim_available(&pool, "worker-1", T0, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].outbox_id, "out-1");
        assert_eq!(claimed[0].status, OutboxStatus::Claimed);
        assert_eq!(claimed[0].attempts, 1);
        assert_eq!(claimed[0].claimed_by.as_deref(), Some("worker-1"));

        assert!(retry(&pool, "out-1", T0 + 250).await.unwrap());
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
    }
}
