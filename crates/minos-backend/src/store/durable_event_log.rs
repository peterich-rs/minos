use serde_json::Value;

use crate::app::tx::DbTx;
use crate::error::BackendError;
use crate::realtime::{DurableEvent, RealtimeTopic};
use crate::store::{AsStorePool, StorePoolRef};

type DurableEventRowTuple = (String, String, String, i64, String, String, i64);

#[derive(Debug, Clone, PartialEq)]
pub struct DurableEventRow {
    pub event_id: String,
    pub topic: String,
    pub topic_kind: String,
    pub topic_seq: i64,
    pub partition_key: String,
    pub payload_json: Value,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicCursor {
    pub event_id: String,
    pub topic: RealtimeTopic,
    pub topic_seq: i64,
}

pub async fn append(
    store: &impl AsStorePool,
    event_id: &str,
    topic: &str,
    topic_kind: &str,
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

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO durable_event_log
                    (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(event_id)
            .bind(topic)
            .bind(topic_kind)
            .bind(topic_seq)
            .bind(partition_key)
            .bind(payload_json.as_str())
            .bind(created_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO durable_event_log
                    (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5, CAST($6 AS JSONB), $7)",
            )
            .bind(event_id)
            .bind(topic)
            .bind(topic_kind)
            .bind(topic_seq)
            .bind(partition_key)
            .bind(payload_json.as_str())
            .bind(created_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("durable_event_log::append"))?;
    Ok(())
}

pub async fn record_in_tx(
    tx: &mut DbTx<'_>,
    event_id: &str,
    event: &DurableEvent,
    created_at_ms: i64,
) -> Result<TopicCursor, BackendError> {
    let topic = event.topic();
    let topic_string = topic.topic_string();
    let topic_kind = topic.kind().as_str();
    let partition_key = topic.partition_key().to_string();
    let payload_json = serde_json::to_value(event).map_err(|error| BackendError::StoreQuery {
        operation: "durable_event_log::record_in_tx.serialize".into(),
        message: error.to_string(),
    })?;
    let payload_json =
        serde_json::to_string(&payload_json).map_err(|error| BackendError::StoreQuery {
            operation: "durable_event_log::record_in_tx.serialize_string".into(),
            message: error.to_string(),
        })?;

    let topic_seq = match tx {
        DbTx::Sqlite(tx) => {
            let next_seq = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(topic_seq), 0) + 1
                   FROM durable_event_log
                  WHERE topic_kind = ?
                    AND topic = ?",
            )
            .bind(topic_kind)
            .bind(&topic_string)
            .fetch_one(&mut **tx)
            .await
            .map_err(store_err("durable_event_log::record_in_tx.next_seq_sqlite"))?;

            sqlx::query(
                "INSERT INTO durable_event_log
                    (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(event_id)
            .bind(&topic_string)
            .bind(topic_kind)
            .bind(next_seq)
            .bind(&partition_key)
            .bind(payload_json.as_str())
            .bind(created_at_ms)
            .execute(&mut **tx)
            .await
            .map(|_| next_seq)
            .map_err(store_err("durable_event_log::record_in_tx.insert_sqlite"))?
        }
        DbTx::Postgres(tx) => {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
                .bind(&topic_string)
                .execute(&mut **tx)
                .await
                .map_err(store_err("durable_event_log::record_in_tx.lock_postgres"))?;

            let next_seq = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(topic_seq), 0) + 1
                   FROM durable_event_log
                  WHERE topic_kind = $1
                    AND topic = $2",
            )
            .bind(topic_kind)
            .bind(&topic_string)
            .fetch_one(&mut **tx)
            .await
            .map_err(store_err(
                "durable_event_log::record_in_tx.next_seq_postgres",
            ))?;

            sqlx::query(
                "INSERT INTO durable_event_log
                    (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5, CAST($6 AS JSONB), $7)",
            )
            .bind(event_id)
            .bind(&topic_string)
            .bind(topic_kind)
            .bind(next_seq)
            .bind(&partition_key)
            .bind(payload_json.as_str())
            .bind(created_at_ms)
            .execute(&mut **tx)
            .await
            .map(|_| next_seq)
            .map_err(store_err("durable_event_log::record_in_tx.insert_postgres"))?
        }
    };

    Ok(TopicCursor {
        event_id: event_id.to_string(),
        topic,
        topic_seq,
    })
}

pub async fn get(
    store: &impl AsStorePool,
    topic_kind: &str,
    event_id: &str,
) -> Result<Option<DurableEventRow>, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, DurableEventRowTuple>(
                "SELECT event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms
                   FROM durable_event_log
                  WHERE topic_kind = ? AND event_id = ?",
            )
            .bind(topic_kind)
            .bind(event_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, DurableEventRowTuple>(
                "SELECT event_id, topic, topic_kind, topic_seq, partition_key, payload_json::text, created_at_ms
                   FROM durable_event_log
                  WHERE topic_kind = $1 AND event_id = $2",
            )
            .bind(topic_kind)
            .bind(event_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("durable_event_log::get"))?;

    row.map(decode_row).transpose()
}

pub async fn read_topic_after(
    store: &impl AsStorePool,
    topic_kind: &str,
    topic: &str,
    after_topic_seq: i64,
    limit: u32,
) -> Result<Vec<DurableEventRow>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, DurableEventRowTuple>(
                "SELECT event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms
                   FROM durable_event_log
                  WHERE topic_kind = ?
                    AND topic = ?
                    AND topic_seq > ?
                  ORDER BY topic_seq ASC
                  LIMIT ?",
            )
            .bind(topic_kind)
            .bind(topic)
            .bind(after_topic_seq)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, DurableEventRowTuple>(
                "SELECT event_id, topic, topic_kind, topic_seq, partition_key, payload_json::text, created_at_ms
                   FROM durable_event_log
                  WHERE topic_kind = $1
                    AND topic = $2
                    AND topic_seq > $3
                  ORDER BY topic_seq ASC
                  LIMIT $4",
            )
            .bind(topic_kind)
            .bind(topic)
            .bind(after_topic_seq)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("durable_event_log::read_topic_after"))?;

    rows.into_iter().map(decode_row).collect()
}

pub async fn delete_ready_for_retention(
    store: &impl AsStorePool,
    older_than_ms: i64,
    limit: u32,
) -> Result<u64, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            delete_ready_for_retention_sqlite(pool, older_than_ms, limit).await
        }
        StorePoolRef::Postgres(pool) => {
            delete_ready_for_retention_postgres(pool, older_than_ms, limit).await
        }
    }
}

async fn delete_ready_for_retention_sqlite(
    pool: &sqlx::SqlitePool,
    older_than_ms: i64,
    limit: u32,
) -> Result<u64, BackendError> {
    let mut tx = pool.begin().await.map_err(store_err(
        "durable_event_log::delete_ready_for_retention.begin_sqlite",
    ))?;

    let keys = sqlx::query_as::<_, (String, String)>(
        "SELECT d.topic_kind, d.event_id
           FROM durable_event_log d
      LEFT JOIN outbox_events o
             ON o.topic_kind = d.topic_kind
            AND o.event_id = d.event_id
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
        "durable_event_log::delete_ready_for_retention.select_sqlite",
    ))?;

    let mut deleted = 0_u64;
    for (topic_kind, event_id) in keys {
        sqlx::query(
            "DELETE FROM outbox_events
              WHERE topic_kind = ?
                AND event_id = ?
                AND ack_at_ms IS NOT NULL",
        )
        .bind(&topic_kind)
        .bind(&event_id)
        .execute(&mut *tx)
        .await
        .map_err(store_err(
            "durable_event_log::delete_ready_for_retention.delete_outbox_sqlite",
        ))?;

        let result = sqlx::query(
            "DELETE FROM durable_event_log
              WHERE topic_kind = ?
                AND event_id = ?",
        )
        .bind(&topic_kind)
        .bind(&event_id)
        .execute(&mut *tx)
        .await
        .map_err(store_err(
            "durable_event_log::delete_ready_for_retention.delete_event_sqlite",
        ))?;
        deleted += result.rows_affected();
    }

    tx.commit().await.map_err(store_err(
        "durable_event_log::delete_ready_for_retention.commit_sqlite",
    ))?;
    Ok(deleted)
}

async fn delete_ready_for_retention_postgres(
    pool: &sqlx::PgPool,
    older_than_ms: i64,
    limit: u32,
) -> Result<u64, BackendError> {
    let mut tx = pool.begin().await.map_err(store_err(
        "durable_event_log::delete_ready_for_retention.begin_postgres",
    ))?;

    let keys = sqlx::query_as::<_, (String, String)>(
        "SELECT d.topic_kind, d.event_id
           FROM durable_event_log d
      LEFT JOIN outbox_events o
             ON o.topic_kind = d.topic_kind
            AND o.event_id = d.event_id
            AND o.ack_at_ms IS NULL
          WHERE d.created_at_ms < $1
            AND o.outbox_id IS NULL
       ORDER BY d.created_at_ms ASC, d.event_id ASC
          LIMIT $2",
    )
    .bind(older_than_ms)
    .bind(i64::from(limit))
    .fetch_all(&mut *tx)
    .await
    .map_err(store_err(
        "durable_event_log::delete_ready_for_retention.select_postgres",
    ))?;

    let mut deleted = 0_u64;
    for (topic_kind, event_id) in keys {
        sqlx::query(
            "DELETE FROM outbox_events
              WHERE topic_kind = $1
                AND event_id = $2
                AND ack_at_ms IS NOT NULL",
        )
        .bind(&topic_kind)
        .bind(&event_id)
        .execute(&mut *tx)
        .await
        .map_err(store_err(
            "durable_event_log::delete_ready_for_retention.delete_outbox_postgres",
        ))?;

        let result = sqlx::query(
            "DELETE FROM durable_event_log
              WHERE topic_kind = $1
                AND event_id = $2",
        )
        .bind(&topic_kind)
        .bind(&event_id)
        .execute(&mut *tx)
        .await
        .map_err(store_err(
            "durable_event_log::delete_ready_for_retention.delete_event_postgres",
        ))?;
        deleted += result.rows_affected();
    }

    tx.commit().await.map_err(store_err(
        "durable_event_log::delete_ready_for_retention.commit_postgres",
    ))?;
    Ok(deleted)
}

fn decode_row(row: DurableEventRowTuple) -> Result<DurableEventRow, BackendError> {
    let (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms) = row;
    Ok(DurableEventRow {
        event_id,
        topic,
        topic_kind,
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

pub async fn retention_floor(
    store: &impl AsStorePool,
    topic_kind: &str,
    topic: &str,
) -> Result<i64, BackendError> {
    let floor = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MIN(topic_seq), 0)
                   FROM durable_event_log
                  WHERE topic_kind = ?
                    AND topic = ?",
            )
            .bind(topic_kind)
            .bind(topic)
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MIN(topic_seq), 0)
                   FROM durable_event_log
                  WHERE topic_kind = $1
                    AND topic = $2",
            )
            .bind(topic_kind)
            .bind(topic)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("durable_event_log::retention_floor"))?;
    Ok(floor)
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
            "host",
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
            "host",
            2,
            "dev1",
            &serde_json::json!({ "kind": "acked" }),
            T0 + 1,
        )
        .await
        .unwrap();

        let rows = read_topic_after(&pool, "host", "host:dev1", 0, 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].event_id, "evt-1");
        assert_eq!(rows[1].event_id, "evt-2");
        assert_eq!(rows[1].topic_kind, "host");
        assert_eq!(rows[1].payload_json, serde_json::json!({ "kind": "acked" }));
    }

    #[tokio::test]
    async fn retention_cleanup_skips_events_with_unacked_outbox_rows() {
        let pool = memory_pool().await;

        append(
            &pool,
            "evt-acked",
            "host:dev1",
            "host",
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
            "host",
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
            "host",
            3,
            "dev1",
            &serde_json::json!({ "kind": "third" }),
            T0 + 2,
        )
        .await
        .unwrap();

        outbox_events::enqueue(&pool, "out-acked", "host", "evt-acked", T0)
            .await
            .unwrap();
        let claimed = outbox_events::claim_available(&pool, "worker-1", T0, 10)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(outbox_events::ack(&pool, "out-acked", T0 + 10)
            .await
            .unwrap());

        outbox_events::enqueue(&pool, "out-pending", "host", "evt-pending", T0)
            .await
            .unwrap();

        assert_eq!(
            delete_ready_for_retention(&pool, T0 + 100, 10)
                .await
                .unwrap(),
            2
        );
        assert!(get(&pool, "host", "evt-acked").await.unwrap().is_none());
        assert!(get(&pool, "host", "evt-free").await.unwrap().is_none());
        assert!(get(&pool, "host", "evt-pending").await.unwrap().is_some());
    }
}
