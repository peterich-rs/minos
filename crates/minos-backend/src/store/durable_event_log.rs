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

/// Test/seed helper: insert a log row and advance sequence authority so
/// `high_watermark` never lags behind an inserted `topic_seq`.
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
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("durable_event_log::append.begin_sqlite"))?;
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
            .execute(&mut *tx)
            .await
            .map_err(store_err("durable_event_log::append.insert_sqlite"))?;
            ensure_watermark_at_least_sqlite(&mut tx, topic_kind, topic, topic_seq, created_at_ms)
                .await?;
            tx.commit()
                .await
                .map_err(store_err("durable_event_log::append.commit_sqlite"))?;
            Ok(())
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("durable_event_log::append.begin_postgres"))?;
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
            .execute(&mut *tx)
            .await
            .map_err(store_err("durable_event_log::append.insert_postgres"))?;
            ensure_watermark_at_least_postgres(
                &mut tx,
                topic_kind,
                topic,
                topic_seq,
                created_at_ms,
            )
            .await?;
            tx.commit()
                .await
                .map_err(store_err("durable_event_log::append.commit_postgres"))?;
            Ok(())
        }
    }
}

async fn ensure_watermark_at_least_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    topic_kind: &str,
    topic: &str,
    topic_seq: i64,
    at_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO topic_metadata
            (topic_kind, topic, high_watermark, retention_floor, updated_at_ms)
         VALUES (?, ?, ?, 0, ?)
         ON CONFLICT(topic_kind, topic) DO UPDATE SET
            high_watermark = MAX(topic_metadata.high_watermark, excluded.high_watermark),
            updated_at_ms = excluded.updated_at_ms",
    )
    .bind(topic_kind)
    .bind(topic)
    .bind(topic_seq)
    .bind(at_ms)
    .execute(&mut **tx)
    .await
    .map_err(store_err("durable_event_log::ensure_watermark_sqlite"))?;
    Ok(())
}

async fn ensure_watermark_at_least_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    topic_kind: &str,
    topic: &str,
    topic_seq: i64,
    at_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT INTO topic_metadata
            (topic_kind, topic, high_watermark, retention_floor, updated_at_ms)
         VALUES ($1, $2, $3, 0, $4)
         ON CONFLICT(topic_kind, topic) DO UPDATE SET
            high_watermark = GREATEST(topic_metadata.high_watermark, EXCLUDED.high_watermark),
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(topic_kind)
    .bind(topic)
    .bind(topic_seq)
    .bind(at_ms)
    .execute(&mut **tx)
    .await
    .map_err(store_err("durable_event_log::ensure_watermark_postgres"))?;
    Ok(())
}

/// Allocate the next monotonic topic_seq from sequence authority and insert the row.
async fn allocate_topic_seq_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    topic_kind: &str,
    topic: &str,
    at_ms: i64,
) -> Result<i64, BackendError> {
    // Upsert bumps high_watermark; RETURNING yields the allocated seq.
    let next_seq = sqlx::query_scalar::<_, i64>(
        "INSERT INTO topic_metadata
            (topic_kind, topic, high_watermark, retention_floor, updated_at_ms)
         VALUES (?, ?, 1, 0, ?)
         ON CONFLICT(topic_kind, topic) DO UPDATE SET
            high_watermark = topic_metadata.high_watermark + 1,
            updated_at_ms = excluded.updated_at_ms
         RETURNING high_watermark",
    )
    .bind(topic_kind)
    .bind(topic)
    .bind(at_ms)
    .fetch_one(&mut **tx)
    .await
    .map_err(store_err("durable_event_log::allocate_seq_sqlite"))?;
    Ok(next_seq)
}

async fn allocate_topic_seq_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    topic_kind: &str,
    topic: &str,
    at_ms: i64,
) -> Result<i64, BackendError> {
    // Serialize per-topic allocation on Postgres (in addition to row lock on upsert).
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(topic)
        .execute(&mut **tx)
        .await
        .map_err(store_err("durable_event_log::allocate_seq_lock_postgres"))?;

    let next_seq = sqlx::query_scalar::<_, i64>(
        "INSERT INTO topic_metadata
            (topic_kind, topic, high_watermark, retention_floor, updated_at_ms)
         VALUES ($1, $2, 1, 0, $3)
         ON CONFLICT(topic_kind, topic) DO UPDATE SET
            high_watermark = topic_metadata.high_watermark + 1,
            updated_at_ms = EXCLUDED.updated_at_ms
         RETURNING high_watermark",
    )
    .bind(topic_kind)
    .bind(topic)
    .bind(at_ms)
    .fetch_one(&mut **tx)
    .await
    .map_err(store_err("durable_event_log::allocate_seq_postgres"))?;
    Ok(next_seq)
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
            let next_seq =
                allocate_topic_seq_sqlite(tx, topic_kind, &topic_string, created_at_ms).await?;

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
            let next_seq =
                allocate_topic_seq_postgres(tx, topic_kind, &topic_string, created_at_ms).await?;

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

    // Include topic_seq so retention_floor advances without relying on payload rows.
    // Dead-letter outbox rows keep ack_at_ms NULL; treat terminal dead as reclaimable.
    let keys = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT d.topic_kind, d.topic, d.event_id, d.topic_seq
           FROM durable_event_log d
      LEFT JOIN outbox_events o
             ON o.topic_kind = d.topic_kind
            AND o.event_id = d.event_id
            AND o.ack_at_ms IS NULL
            AND o.dead_at_ms IS NULL
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

    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut deleted = 0_u64;
    for (topic_kind, topic, event_id, topic_seq) in keys {
        sqlx::query(
            "DELETE FROM outbox_events
              WHERE topic_kind = ?
                AND event_id = ?
                AND (ack_at_ms IS NOT NULL OR dead_at_ms IS NOT NULL OR status = 'dead')",
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
        if result.rows_affected() > 0 {
            deleted += result.rows_affected();
            // Advance floor to the highest deleted seq; high_watermark is untouched.
            sqlx::query(
                "INSERT INTO topic_metadata
                    (topic_kind, topic, high_watermark, retention_floor, updated_at_ms)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(topic_kind, topic) DO UPDATE SET
                    retention_floor = MAX(topic_metadata.retention_floor, excluded.retention_floor),
                    high_watermark = MAX(topic_metadata.high_watermark, excluded.high_watermark),
                    updated_at_ms = excluded.updated_at_ms",
            )
            .bind(&topic_kind)
            .bind(&topic)
            .bind(topic_seq)
            .bind(topic_seq)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(store_err(
                "durable_event_log::delete_ready_for_retention.floor_sqlite",
            ))?;
        }
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

    let keys = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT d.topic_kind, d.topic, d.event_id, d.topic_seq
           FROM durable_event_log d
      LEFT JOIN outbox_events o
             ON o.topic_kind = d.topic_kind
            AND o.event_id = d.event_id
            AND o.ack_at_ms IS NULL
            AND o.dead_at_ms IS NULL
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

    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut deleted = 0_u64;
    for (topic_kind, topic, event_id, topic_seq) in keys {
        sqlx::query(
            "DELETE FROM outbox_events
              WHERE topic_kind = $1
                AND event_id = $2
                AND (ack_at_ms IS NOT NULL OR dead_at_ms IS NOT NULL OR status = 'dead')",
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
        if result.rows_affected() > 0 {
            deleted += result.rows_affected();
            sqlx::query(
                "INSERT INTO topic_metadata
                    (topic_kind, topic, high_watermark, retention_floor, updated_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(topic_kind, topic) DO UPDATE SET
                    retention_floor = GREATEST(topic_metadata.retention_floor, EXCLUDED.retention_floor),
                    high_watermark = GREATEST(topic_metadata.high_watermark, EXCLUDED.high_watermark),
                    updated_at_ms = EXCLUDED.updated_at_ms",
            )
            .bind(&topic_kind)
            .bind(&topic)
            .bind(topic_seq)
            .bind(topic_seq)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(store_err(
                "durable_event_log::delete_ready_for_retention.floor_postgres",
            ))?;
        }
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

/// Sequence authority metadata for a topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TopicSequenceMeta {
    /// Highest seq ever allocated (never decreases).
    pub high_watermark: i64,
    /// Highest deleted payload seq. Resume with `after < retention_floor` → SnapshotRequired.
    pub retention_floor: i64,
}

/// Read sequence authority. Missing topic → (0, 0) — never used, safe full replay.
pub async fn topic_sequence_meta(
    store: &impl AsStorePool,
    topic_kind: &str,
    topic: &str,
) -> Result<TopicSequenceMeta, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT high_watermark, retention_floor
                   FROM topic_metadata
                  WHERE topic_kind = ?
                    AND topic = ?",
            )
            .bind(topic_kind)
            .bind(topic)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT high_watermark, retention_floor
                   FROM topic_metadata
                  WHERE topic_kind = $1
                    AND topic = $2",
            )
            .bind(topic_kind)
            .bind(topic)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("durable_event_log::topic_sequence_meta"))?;

    Ok(row
        .map(|(high_watermark, retention_floor)| TopicSequenceMeta {
            high_watermark,
            retention_floor,
        })
        .unwrap_or_default())
}

/// Highest deleted payload seq for SnapshotRequired checks.
/// Prefer authority table; fall back to min retained - 1 for rows written
/// before metadata existed (dev-only; production always has metadata).
pub async fn retention_floor(
    store: &impl AsStorePool,
    topic_kind: &str,
    topic: &str,
) -> Result<i64, BackendError> {
    let meta = topic_sequence_meta(store, topic_kind, topic).await?;
    if meta.high_watermark > 0 || meta.retention_floor > 0 {
        return Ok(meta.retention_floor);
    }
    // Never-used topic: floor 0.
    let min_seq = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT MIN(topic_seq)
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
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT MIN(topic_seq)
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
    .map_err(store_err("durable_event_log::retention_floor.fallback"))?;
    Ok(min_seq.map(|s| s.saturating_sub(1)).unwrap_or(0))
}

/// Highest allocated topic_seq (replay barrier watermark).
pub async fn high_watermark(
    store: &impl AsStorePool,
    topic_kind: &str,
    topic: &str,
) -> Result<i64, BackendError> {
    Ok(topic_sequence_meta(store, topic_kind, topic)
        .await?
        .high_watermark)
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
    async fn sequence_authority_survives_full_retention_purge() {
        let pool = memory_pool().await;

        // Seed contiguous history 1..=3 via authority-aware append.
        for seq in 1..=3 {
            append(
                &pool,
                &format!("evt-{seq}"),
                "account:a1",
                "account",
                seq,
                "a1",
                &serde_json::json!({ "n": seq }),
                T0 + seq,
            )
            .await
            .unwrap();
        }
        let meta = topic_sequence_meta(&pool, "account", "account:a1")
            .await
            .unwrap();
        assert_eq!(meta.high_watermark, 3);
        assert_eq!(meta.retention_floor, 0);

        // Purge all payload rows — authority floor advances, watermark stays.
        assert_eq!(
            delete_ready_for_retention(&pool, T0 + 100, 10)
                .await
                .unwrap(),
            3
        );
        assert!(get(&pool, "account", "evt-1").await.unwrap().is_none());
        let meta = topic_sequence_meta(&pool, "account", "account:a1")
            .await
            .unwrap();
        assert_eq!(meta.high_watermark, 3);
        assert_eq!(meta.retention_floor, 3);
        assert_eq!(
            retention_floor(&pool, "account", "account:a1")
                .await
                .unwrap(),
            3
        );

        // Next allocated seq must continue at 4 (not reset to 1).
        let mut tx = pool.begin().await.unwrap();
        let next = allocate_topic_seq_sqlite(&mut tx, "account", "account:a1", T0 + 200)
            .await
            .unwrap();
        assert_eq!(next, 4);
        tx.commit().await.unwrap();
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

        outbox_events::enqueue(
            &pool,
            "out-acked",
            "host",
            "evt-acked",
            outbox_events::OutboxLane::SocialDurable,
            T0,
        )
        .await
        .unwrap();
        let claimed = outbox_events::claim_available(
            &pool,
            "worker-1",
            T0,
            10,
            outbox_events::OutboxLane::SocialDurable,
        )
        .await
        .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(outbox_events::ack(&pool, "out-acked", T0 + 10)
            .await
            .unwrap());

        outbox_events::enqueue(
            &pool,
            "out-pending",
            "host",
            "evt-pending",
            outbox_events::OutboxLane::SocialDurable,
            T0,
        )
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
