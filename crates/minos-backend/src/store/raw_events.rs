//! `raw_events` table CRUD.
//!
//! Verbatim native events keyed on `(thread_id, seq)`. The backend persists
//! them on ingest and the translator re-reads them for history (`ReadThread`,
//! task C2). Dedup on insert is authoritative: a retransmit with the same
//! `(thread_id, seq)` is a no-op.

use minos_domain::AgentName;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::RelayError;

/// Decoded row from the `raw_events` table.
#[derive(Debug, Clone)]
pub struct RawEventRow {
    pub seq: i64,
    pub agent: AgentName,
    pub payload: Value,
    pub ts_ms: i64,
}

/// Wire-value string for an `AgentName`, matching the DB CHECK constraint.
fn agent_str(a: AgentName) -> &'static str {
    match a {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
        AgentName::Gemini => "gemini",
    }
}

fn parse_agent(s: &str) -> Result<AgentName, RelayError> {
    match s {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        other => Err(RelayError::StoreDecode {
            column: "raw_events.agent".into(),
            message: other.to_string(),
        }),
    }
}

/// Insert one raw event. If `(thread_id, seq)` already exists, the insert
/// is a no-op (retransmit safety) and we return `Ok(false)`; otherwise
/// `Ok(true)`.
pub async fn insert_if_absent(
    pool: &SqlitePool,
    thread_id: &str,
    seq: u64,
    agent: AgentName,
    payload: &Value,
    ts_ms: i64,
) -> Result<bool, RelayError> {
    let payload_s = serde_json::to_string(payload).map_err(|e| RelayError::StoreQuery {
        operation: "raw_events.insert_if_absent.serialise".into(),
        message: e.to_string(),
    })?;
    let result = sqlx::query(
        r"INSERT OR IGNORE INTO raw_events (thread_id, seq, agent, payload_json, ts_ms)
           VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(thread_id)
    .bind(i64::try_from(seq).unwrap_or(i64::MAX))
    .bind(agent_str(agent))
    .bind(payload_s)
    .bind(ts_ms)
    .execute(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "raw_events.insert_if_absent".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected() == 1)
}

/// Read a contiguous window of raw events for `thread_id` starting at
/// `from_seq` (inclusive), capped at `limit`. Rows are returned in
/// ascending `seq` order.
pub async fn read_range(
    pool: &SqlitePool,
    thread_id: &str,
    from_seq: u64,
    limit: u32,
) -> Result<Vec<RawEventRow>, RelayError> {
    let rows = sqlx::query_as::<_, (i64, String, String, i64)>(
        r"SELECT seq, agent, payload_json, ts_ms FROM raw_events
           WHERE thread_id = ?1 AND seq >= ?2
           ORDER BY seq ASC LIMIT ?3",
    )
    .bind(thread_id)
    .bind(i64::try_from(from_seq).unwrap_or(i64::MAX))
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "raw_events.read_range".into(),
        message: e.to_string(),
    })?;

    rows.into_iter()
        .map(|(seq, agent, payload, ts_ms)| {
            let agent = parse_agent(&agent)?;
            let payload = serde_json::from_str(&payload).map_err(|e| RelayError::StoreDecode {
                column: "raw_events.payload_json".into(),
                message: e.to_string(),
            })?;
            Ok(RawEventRow {
                seq,
                agent,
                payload,
                ts_ms,
            })
        })
        .collect()
}

/// Return the largest `seq` ever persisted for `thread_id`, or `0` if no
/// rows exist. Used by the agent-host to decide whether to re-ingest on
/// startup (`GetThreadLastSeq` LocalRpc).
pub async fn last_seq(pool: &SqlitePool, thread_id: &str) -> Result<u64, RelayError> {
    let v: Option<i64> =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM raw_events WHERE thread_id = ?1")
            .bind(thread_id)
            .fetch_one(pool)
            .await
            .map_err(|e| RelayError::StoreQuery {
                operation: "raw_events.last_seq".into(),
                message: e.to_string(),
            })?;
    Ok(u64::try_from(v.unwrap_or(0)).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;

    async fn seed_host_and_thread(pool: &SqlitePool) {
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at)
               VALUES ('dev1','Dev','agent-host',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            r"INSERT INTO threads (thread_id, agent, owner_device_id, first_ts_ms, last_ts_ms)
               VALUES ('thr1','codex','dev1',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn insert_is_idempotent() {
        let pool = memory_pool().await;
        seed_host_and_thread(&pool).await;
        let payload = serde_json::json!({"x":1});

        assert!(
            insert_if_absent(&pool, "thr1", 1, AgentName::Codex, &payload, 100)
                .await
                .unwrap()
        );
        // Second insert with same (thread_id, seq) must be a no-op.
        assert!(
            !insert_if_absent(&pool, "thr1", 1, AgentName::Codex, &payload, 100)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn read_range_returns_in_order() {
        let pool = memory_pool().await;
        seed_host_and_thread(&pool).await;

        for i in 1..=5u64 {
            let _ = insert_if_absent(
                &pool,
                "thr1",
                i,
                AgentName::Codex,
                &serde_json::json!({"i":i}),
                i64::try_from(i).unwrap() * 100,
            )
            .await
            .unwrap();
        }

        let rows = read_range(&pool, "thr1", 2, 10).await.unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].seq, 2);
        assert_eq!(rows[3].seq, 5);
        assert_eq!(rows[0].agent, AgentName::Codex);
        assert_eq!(rows[0].payload, serde_json::json!({"i":2}));
    }

    #[tokio::test]
    async fn read_range_respects_limit() {
        let pool = memory_pool().await;
        seed_host_and_thread(&pool).await;

        for i in 1..=5u64 {
            let _ = insert_if_absent(
                &pool,
                "thr1",
                i,
                AgentName::Codex,
                &serde_json::json!({}),
                i64::try_from(i).unwrap(),
            )
            .await
            .unwrap();
        }

        let rows = read_range(&pool, "thr1", 1, 2).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 1);
        assert_eq!(rows[1].seq, 2);
    }

    #[tokio::test]
    async fn last_seq_empty_thread_returns_zero() {
        let pool = memory_pool().await;
        seed_host_and_thread(&pool).await;
        assert_eq!(last_seq(&pool, "thr1").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn last_seq_returns_max() {
        let pool = memory_pool().await;
        seed_host_and_thread(&pool).await;

        for i in [1u64, 2, 3, 7] {
            let _ = insert_if_absent(
                &pool,
                "thr1",
                i,
                AgentName::Codex,
                &serde_json::json!({}),
                i64::try_from(i).unwrap(),
            )
            .await
            .unwrap();
        }

        assert_eq!(last_seq(&pool, "thr1").await.unwrap(), 7);
    }
}
