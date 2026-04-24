//! `threads` table CRUD (see spec §9.1).
//!
//! A `thread` is one live session on an agent-host. Rows are created
//! implicitly by the first `raw_event` ingest (`upsert`) and mutated as
//! subsequent events arrive: `update_title` when the translator produces a
//! `ThreadTitleUpdated`, `increment_message_count` when a new message is
//! placed, `mark_ended` when the backend sees `ThreadClosed`.
//!
//! List (for `LocalRpc::ListThreads`) lands in task C1.

use minos_domain::AgentName;
use minos_ui_protocol::ThreadEndReason;
use sqlx::SqlitePool;

use crate::error::RelayError;

/// Wire-value string for an `AgentName`, matching the DB CHECK constraint.
fn agent_str(a: AgentName) -> &'static str {
    match a {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
        AgentName::Gemini => "gemini",
    }
}

/// Insert-or-bump: on first ingest, create the row; on subsequent ingests
/// for the same `thread_id`, update `last_ts_ms` to `ts_ms`. `first_ts_ms`
/// is frozen at insert time, `message_count` starts at 0.
pub async fn upsert(
    pool: &SqlitePool,
    thread_id: &str,
    agent: AgentName,
    owner_device_id: &str,
    ts_ms: i64,
) -> Result<(), RelayError> {
    sqlx::query(
        r"INSERT INTO threads (thread_id, agent, owner_device_id, first_ts_ms, last_ts_ms, message_count)
           VALUES (?1, ?2, ?3, ?4, ?4, 0)
           ON CONFLICT(thread_id) DO UPDATE SET last_ts_ms = ?4",
    )
    .bind(thread_id)
    .bind(agent_str(agent))
    .bind(owner_device_id)
    .bind(ts_ms)
    .execute(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "threads.upsert".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Mark a thread as ended. `reason` is serialised as the same JSON the wire
/// protocol uses — `serde_json::to_string` on a `ThreadEndReason` produces
/// `{"kind":"agent_done"}` etc.
pub async fn mark_ended(
    pool: &SqlitePool,
    thread_id: &str,
    reason: &ThreadEndReason,
    ts_ms: i64,
) -> Result<(), RelayError> {
    let reason_json = serde_json::to_string(reason).map_err(|e| RelayError::StoreQuery {
        operation: "threads.mark_ended.serialise".into(),
        message: e.to_string(),
    })?;
    sqlx::query(r"UPDATE threads SET ended_at_ms = ?1, end_reason = ?2 WHERE thread_id = ?3")
        .bind(ts_ms)
        .bind(reason_json)
        .bind(thread_id)
        .execute(pool)
        .await
        .map_err(|e| RelayError::StoreQuery {
            operation: "threads.mark_ended".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

/// Set the human-friendly title. Called when the translator emits
/// `ThreadTitleUpdated` (codex surfaces this as a separate notification).
pub async fn update_title(
    pool: &SqlitePool,
    thread_id: &str,
    title: &str,
) -> Result<(), RelayError> {
    sqlx::query(r"UPDATE threads SET title = ?1 WHERE thread_id = ?2")
        .bind(title)
        .bind(thread_id)
        .execute(pool)
        .await
        .map_err(|e| RelayError::StoreQuery {
            operation: "threads.update_title".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

/// Bump `message_count` by 1. Called when the translator places a new
/// `MessageStarted` — gives the list view a cheap "N messages" badge.
pub async fn increment_message_count(pool: &SqlitePool, thread_id: &str) -> Result<(), RelayError> {
    sqlx::query(r"UPDATE threads SET message_count = message_count + 1 WHERE thread_id = ?1")
        .bind(thread_id)
        .execute(pool)
        .await
        .map_err(|e| RelayError::StoreQuery {
            operation: "threads.increment_message_count".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;

    async fn seed_agent_host(pool: &SqlitePool) {
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at)
               VALUES ('dev1','Dev','agent-host',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn upsert_inserts_then_bumps_last_ts() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;

        upsert(&pool, "thr1", AgentName::Codex, "dev1", 1000)
            .await
            .unwrap();
        upsert(&pool, "thr1", AgentName::Codex, "dev1", 2000)
            .await
            .unwrap();

        let (first, last): (i64, i64) =
            sqlx::query_as("SELECT first_ts_ms, last_ts_ms FROM threads WHERE thread_id = 'thr1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        // first_ts_ms is frozen at insert; last_ts_ms tracks the most recent update.
        assert_eq!(first, 1000);
        assert_eq!(last, 2000);
    }

    #[tokio::test]
    async fn mark_ended_stores_reason_json() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;
        upsert(&pool, "thr1", AgentName::Codex, "dev1", 1000)
            .await
            .unwrap();

        mark_ended(&pool, "thr1", &ThreadEndReason::HostDisconnected, 2000)
            .await
            .unwrap();

        let (ended_at, reason): (Option<i64>, Option<String>) =
            sqlx::query_as("SELECT ended_at_ms, end_reason FROM threads WHERE thread_id = 'thr1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(ended_at, Some(2000));
        let reason_s = reason.unwrap();
        assert!(
            reason_s.contains("host_disconnected"),
            "end_reason = {reason_s}"
        );
    }

    #[tokio::test]
    async fn update_title_sets_title() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;
        upsert(&pool, "thr1", AgentName::Codex, "dev1", 1000)
            .await
            .unwrap();

        update_title(&pool, "thr1", "rename branch").await.unwrap();

        let title: Option<String> =
            sqlx::query_scalar("SELECT title FROM threads WHERE thread_id = 'thr1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(title, Some("rename branch".into()));
    }

    #[tokio::test]
    async fn increment_message_count_accumulates() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;
        upsert(&pool, "thr1", AgentName::Codex, "dev1", 1000)
            .await
            .unwrap();

        increment_message_count(&pool, "thr1").await.unwrap();
        increment_message_count(&pool, "thr1").await.unwrap();
        increment_message_count(&pool, "thr1").await.unwrap();

        let n: i64 =
            sqlx::query_scalar("SELECT message_count FROM threads WHERE thread_id = 'thr1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(n, 3);
    }
}
