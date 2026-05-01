//! `raw_events` table CRUD.
//!
//! Verbatim native events keyed on `(thread_id, seq)`. The backend persists
//! them on ingest and the translator re-reads them for history (`ReadThread`,
//! task C2). Exact retransmits are deduped, while `(thread_id, seq)`
//! collisions with different payloads are appended at a fresh backend seq so
//! daemon restart/resume cannot silently drop new output.

use minos_domain::AgentName;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::BackendError;

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

fn parse_agent(s: &str) -> Result<AgentName, BackendError> {
    match s {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        other => Err(BackendError::StoreDecode {
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
) -> Result<bool, BackendError> {
    let payload_s = serde_json::to_string(payload).map_err(|e| BackendError::StoreQuery {
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
    .map_err(|e| BackendError::StoreQuery {
        operation: "raw_events.insert_if_absent".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected() == 1)
}

/// Insert one raw event and return the persisted sequence number.
///
/// Host-side sequence counters are process-local. After a daemon restart or
/// a stopped session resume, the host can legitimately restart a thread's
/// outgoing seq at 1 while the backend already has older rows for that
/// thread. Treating every `(thread_id, seq)` collision as a retransmit would
/// silently drop fresh Codex output. To keep reconnect/resume safe:
///
/// - same `(thread_id, seq, payload)` is a retransmit and returns `Ok(None)`;
/// - same `(thread_id, seq)` with a different payload is appended at the next
///   available seq and returns that assigned value.
pub async fn insert_assigning_seq(
    pool: &SqlitePool,
    thread_id: &str,
    requested_seq: u64,
    agent: AgentName,
    payload: &Value,
    ts_ms: i64,
) -> Result<Option<u64>, BackendError> {
    let payload_s = serde_json::to_string(payload).map_err(|e| BackendError::StoreQuery {
        operation: "raw_events.insert_assigning_seq.serialise".into(),
        message: e.to_string(),
    })?;

    if insert_payload_at_seq(pool, thread_id, requested_seq, agent, &payload_s, ts_ms).await? {
        return Ok(Some(requested_seq));
    }

    let requested_seq_i64 = i64::try_from(requested_seq).unwrap_or(i64::MAX);
    let existing: Option<String> =
        sqlx::query_scalar("SELECT payload_json FROM raw_events WHERE thread_id = ?1 AND seq = ?2")
            .bind(thread_id)
            .bind(requested_seq_i64)
            .fetch_optional(pool)
            .await
            .map_err(|e| BackendError::StoreQuery {
                operation: "raw_events.insert_assigning_seq.lookup_collision".into(),
                message: e.to_string(),
            })?;

    if existing.as_deref() == Some(payload_s.as_str()) {
        return Ok(None);
    }

    for _ in 0..8 {
        let next = last_seq(pool, thread_id).await?.saturating_add(1);
        if insert_payload_at_seq(pool, thread_id, next, agent, &payload_s, ts_ms).await? {
            tracing::warn!(
                target: "minos_backend::raw_events",
                thread_id,
                requested_seq,
                assigned_seq = next,
                "raw event seq collision with different payload; appended at next seq",
            );
            return Ok(Some(next));
        }
    }

    Err(BackendError::StoreQuery {
        operation: "raw_events.insert_assigning_seq".into(),
        message: "could not allocate non-conflicting seq after retries".into(),
    })
}

async fn insert_payload_at_seq(
    pool: &SqlitePool,
    thread_id: &str,
    seq: u64,
    agent: AgentName,
    payload_s: &str,
    ts_ms: i64,
) -> Result<bool, BackendError> {
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
    .map_err(|e| BackendError::StoreQuery {
        operation: "raw_events.insert_payload_at_seq".into(),
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
) -> Result<Vec<RawEventRow>, BackendError> {
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
    .map_err(|e| BackendError::StoreQuery {
        operation: "raw_events.read_range".into(),
        message: e.to_string(),
    })?;

    rows.into_iter()
        .map(|(seq, agent, payload, ts_ms)| {
            let agent = parse_agent(&agent)?;
            let payload =
                serde_json::from_str(&payload).map_err(|e| BackendError::StoreDecode {
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

/// Return the largest `seq` per thread across every thread owned by
/// `owner_device_id`. Threads with zero raw events are omitted (the
/// `INNER JOIN` excludes them).
///
/// Used by `/v1/devices/ws` to compute the `IngestCheckpoint` frame the
/// daemon receives on connect (Phase D / spec §9 reconciliation): the
/// daemon compares each backend max against its local watermark and
/// replays the gap.
pub async fn last_seq_per_owner(
    pool: &SqlitePool,
    owner_device_id: &str,
) -> Result<std::collections::HashMap<String, u64>, BackendError> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        r"SELECT t.thread_id, MAX(r.seq)
           FROM raw_events r
           INNER JOIN threads t ON t.thread_id = r.thread_id
           WHERE t.owner_device_id = ?1
           GROUP BY t.thread_id",
    )
    .bind(owner_device_id)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "raw_events.last_seq_per_owner".into(),
        message: e.to_string(),
    })?;

    Ok(rows
        .into_iter()
        .map(|(thread_id, max_seq)| (thread_id, u64::try_from(max_seq).unwrap_or(0)))
        .collect())
}

/// Return the largest `seq` ever persisted for `thread_id`, or `0` if no
/// rows exist. Used by the agent-host to decide whether to re-ingest on
/// startup (`GET /v1/threads/{id}/last_seq`).
pub async fn last_seq(pool: &SqlitePool, thread_id: &str) -> Result<u64, BackendError> {
    let v: Option<i64> =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM raw_events WHERE thread_id = ?1")
            .bind(thread_id)
            .fetch_one(pool)
            .await
            .map_err(|e| BackendError::StoreQuery {
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
    async fn insert_assigning_seq_dedupes_same_payload_but_appends_different_collision() {
        let pool = memory_pool().await;
        seed_host_and_thread(&pool).await;

        let first = serde_json::json!({"method":"a"});
        let duplicate = first.clone();
        let fresh_after_counter_reset = serde_json::json!({"method":"b"});

        assert_eq!(
            insert_assigning_seq(&pool, "thr1", 1, AgentName::Codex, &first, 100)
                .await
                .unwrap(),
            Some(1),
        );
        assert_eq!(
            insert_assigning_seq(&pool, "thr1", 1, AgentName::Codex, &duplicate, 100)
                .await
                .unwrap(),
            None,
        );
        assert_eq!(
            insert_assigning_seq(
                &pool,
                "thr1",
                1,
                AgentName::Codex,
                &fresh_after_counter_reset,
                200,
            )
            .await
            .unwrap(),
            Some(2),
        );

        let rows = read_range(&pool, "thr1", 1, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 1);
        assert_eq!(rows[0].payload, first);
        assert_eq!(rows[1].seq, 2);
        assert_eq!(rows[1].payload, fresh_after_counter_reset);
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
    async fn last_seq_per_owner_groups_by_thread() {
        let pool = memory_pool().await;
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at)
               VALUES ('host_a','HostA','agent-host',0,0),
                      ('host_b','HostB','agent-host',0,0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r"INSERT INTO threads (thread_id, agent, owner_device_id, first_ts_ms, last_ts_ms)
               VALUES ('thr_1','codex','host_a',0,0),
                      ('thr_2','codex','host_a',0,0),
                      ('thr_3','codex','host_b',0,0),
                      ('thr_empty','codex','host_a',0,0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (tid, seq) in [("thr_1", 7), ("thr_1", 3), ("thr_2", 12), ("thr_3", 1)] {
            insert_if_absent(&pool, tid, seq, AgentName::Codex, &serde_json::json!({}), 0)
                .await
                .unwrap();
        }

        let map = last_seq_per_owner(&pool, "host_a").await.unwrap();
        // thr_empty has no raw_events row → excluded by the INNER JOIN.
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("thr_1").copied(), Some(7));
        assert_eq!(map.get("thr_2").copied(), Some(12));
        assert!(!map.contains_key("thr_3"), "must not leak host_b's thread");
    }

    #[tokio::test]
    async fn last_seq_per_owner_empty_for_unknown_owner() {
        let pool = memory_pool().await;
        let map = last_seq_per_owner(&pool, "ghost").await.unwrap();
        assert!(map.is_empty());
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
