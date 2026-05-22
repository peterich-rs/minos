use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::BackendError;

type DurableEventRowTuple = (String, String, i64, String, String, i64);

#[derive(Debug, Clone, PartialEq)]
pub struct DurableEventRow {
    pub event_id: String,
    pub topic: String,
    pub topic_seq: i64,
    pub partition_key: String,
    pub payload_json: Value,
    pub created_at_ms: i64,
}

pub async fn append(
    pool: &SqlitePool,
    event_id: &str,
    topic: &str,
    topic_seq: i64,
    partition_key: &str,
    payload_json: &Value,
    created_at_ms: i64,
) -> Result<(), BackendError> {
    let payload_json =
        serde_json::to_string(payload_json).map_err(|error| BackendError::StoreQuery {
            operation: "durable_event_log::append.serialize".into(),
            message: error.to_string(),
        })?;

    sqlx::query(
        "INSERT INTO durable_event_log
            (event_id, topic, topic_seq, partition_key, payload_json, created_at_ms)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(event_id)
    .bind(topic)
    .bind(topic_seq)
    .bind(partition_key)
    .bind(payload_json)
    .bind(created_at_ms)
    .execute(pool)
    .await
    .map_err(store_err("durable_event_log::append"))?;
    Ok(())
}

pub async fn get(
    pool: &SqlitePool,
    event_id: &str,
) -> Result<Option<DurableEventRow>, BackendError> {
    let row = sqlx::query_as::<_, DurableEventRowTuple>(
        "SELECT event_id, topic, topic_seq, partition_key, payload_json, created_at_ms
           FROM durable_event_log
          WHERE event_id = ?",
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("durable_event_log::get"))?;

    row.map(decode_row).transpose()
}

pub async fn read_topic_after(
    pool: &SqlitePool,
    topic: &str,
    after_topic_seq: i64,
    limit: u32,
) -> Result<Vec<DurableEventRow>, BackendError> {
    let rows = sqlx::query_as::<_, DurableEventRowTuple>(
        "SELECT event_id, topic, topic_seq, partition_key, payload_json, created_at_ms
           FROM durable_event_log
          WHERE topic = ?
            AND topic_seq > ?
          ORDER BY topic_seq ASC
          LIMIT ?",
    )
    .bind(topic)
    .bind(after_topic_seq)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(store_err("durable_event_log::read_topic_after"))?;

    rows.into_iter().map(decode_row).collect()
}

pub async fn delete_ready_for_retention(
    pool: &SqlitePool,
    older_than_ms: i64,
    limit: u32,
) -> Result<u64, BackendError> {
    let mut tx = pool.begin().await.map_err(store_err(
        "durable_event_log::delete_ready_for_retention.begin",
    ))?;

    let event_ids = sqlx::query_scalar::<_, String>(
        "SELECT d.event_id
           FROM durable_event_log d
      LEFT JOIN outbox_events o
             ON o.event_id = d.event_id
            AND o.ack_at_ms IS NULL
          WHERE d.created_at_ms < ?
            AND o.outbox_id IS NULL
       ORDER BY d.created_at_ms ASC, d.event_id ASC
          LIMIT ?",
    )
    .bind(older_than_ms)
    .bind(i64::from(limit))
    .fetch_all(&mut *tx)
    .await
    .map_err(store_err(
        "durable_event_log::delete_ready_for_retention.select",
    ))?;

    let mut deleted = 0_u64;
    for event_id in event_ids {
        sqlx::query("DELETE FROM outbox_events WHERE event_id = ? AND ack_at_ms IS NOT NULL")
            .bind(&event_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err(
                "durable_event_log::delete_ready_for_retention.delete_outbox",
            ))?;

        let result = sqlx::query("DELETE FROM durable_event_log WHERE event_id = ?")
            .bind(&event_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err(
                "durable_event_log::delete_ready_for_retention.delete_event",
            ))?;
        deleted += result.rows_affected();
    }

    tx.commit().await.map_err(store_err(
        "durable_event_log::delete_ready_for_retention.commit",
    ))?;
    Ok(deleted)
}

fn decode_row(row: DurableEventRowTuple) -> Result<DurableEventRow, BackendError> {
    let (event_id, topic, topic_seq, partition_key, payload_json, created_at_ms) = row;
    Ok(DurableEventRow {
        event_id,
        topic,
        topic_seq,
        partition_key,
        payload_json: serde_json::from_str(&payload_json).map_err(|error| {
            BackendError::StoreDecode {
                column: "durable_event_log.payload_json".into(),
                message: error.to_string(),
            }
        })?,
        created_at_ms,
    })
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
    use crate::store::outbox_events;
    use crate::store::test_support::{memory_pool, T0};

    #[tokio::test]
    async fn append_and_read_topic_after_returns_ordered_rows() {
        let pool = memory_pool().await;

        append(
            &pool,
            "evt-1",
            "host:dev1",
            1,
            "dev1",
            &serde_json::json!({ "kind": "created" }),
            T0,
        )
        .await
        .unwrap();
        append(
            &pool,
            "evt-2",
            "host:dev1",
            2,
            "dev1",
            &serde_json::json!({ "kind": "acked" }),
            T0 + 1,
        )
        .await
        .unwrap();

        let rows = read_topic_after(&pool, "host:dev1", 0, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].event_id, "evt-1");
        assert_eq!(rows[1].event_id, "evt-2");
        assert_eq!(rows[1].payload_json, serde_json::json!({ "kind": "acked" }));
    }

    #[tokio::test]
    async fn retention_cleanup_skips_events_with_unacked_outbox_rows() {
        let pool = memory_pool().await;

        append(
            &pool,
            "evt-acked",
            "host:dev1",
            1,
            "dev1",
            &serde_json::json!({ "kind": "first" }),
            T0,
        )
        .await
        .unwrap();
        append(
            &pool,
            "evt-pending",
            "host:dev1",
            2,
            "dev1",
            &serde_json::json!({ "kind": "second" }),
            T0 + 1,
        )
        .await
        .unwrap();
        append(
            &pool,
            "evt-free",
            "host:dev1",
            3,
            "dev1",
            &serde_json::json!({ "kind": "third" }),
            T0 + 2,
        )
        .await
        .unwrap();

        outbox_events::enqueue(&pool, "out-acked", "evt-acked", T0)
            .await
            .unwrap();
        let claimed = outbox_events::claim_available(&pool, "worker-1", T0, 10)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(outbox_events::ack(&pool, "out-acked", T0 + 10)
            .await
            .unwrap());

        outbox_events::enqueue(&pool, "out-pending", "evt-pending", T0)
            .await
            .unwrap();

        assert_eq!(
            delete_ready_for_retention(&pool, T0 + 100, 10)
                .await
                .unwrap(),
            2
        );
        assert!(get(&pool, "evt-acked").await.unwrap().is_none());
        assert!(get(&pool, "evt-free").await.unwrap().is_none());
        assert!(get(&pool, "evt-pending").await.unwrap().is_some());
    }
}
