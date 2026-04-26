//! `threads` table CRUD (see spec §9.1).
//!
//! A `thread` is one live session on an agent-host. Rows are created
//! implicitly by the first `raw_event` ingest (`upsert`) and mutated as
//! subsequent events arrive: `update_title` when the translator produces a
//! `ThreadTitleUpdated`, `increment_message_count` when a new message is
//! placed, `mark_ended` when the backend sees `ThreadClosed`.
//!
//! List backs the HTTP `GET /v1/threads` route (see `http::v1::threads`).

use minos_domain::AgentName;
use minos_ui_protocol::ThreadEndReason;
use sqlx::SqlitePool;

use crate::error::BackendError;

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
) -> Result<(), BackendError> {
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
    .map_err(|e| BackendError::StoreQuery {
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
) -> Result<(), BackendError> {
    let reason_json = serde_json::to_string(reason).map_err(|e| BackendError::StoreQuery {
        operation: "threads.mark_ended.serialise".into(),
        message: e.to_string(),
    })?;
    sqlx::query(r"UPDATE threads SET ended_at_ms = ?1, end_reason = ?2 WHERE thread_id = ?3")
        .bind(ts_ms)
        .bind(reason_json)
        .bind(thread_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
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
) -> Result<(), BackendError> {
    sqlx::query(r"UPDATE threads SET title = ?1 WHERE thread_id = ?2")
        .bind(title)
        .bind(thread_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "threads.update_title".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

/// List thread summaries for the `GET /v1/threads` HTTP response.
///
/// Filters (all optional):
/// - `owner_device_id`  — restrict to threads owned by this device.
/// - `agent`            — restrict to a single CLI agent.
/// - `before_ts_ms`     — only threads whose `last_ts_ms` is strictly less
///   than this (exclusive cursor for pagination).
/// - `account_id`       — restrict to threads whose `owner_device_id`
///   belongs to a device row with this `account_id`. Spec §5.5; Phase 2
///   Task 2.6. The check uses an `EXISTS` clause against `devices` rather
///   than a `JOIN` so the optional-cursor + ordering plan stays simple.
///
/// Ordering: `last_ts_ms DESC` — most-recently-active first. Capped at
/// `limit` rows; the caller pins the upper bound in the dispatch layer.
pub async fn list(
    pool: &SqlitePool,
    owner_device_id: Option<&str>,
    agent: Option<AgentName>,
    before_ts_ms: Option<i64>,
    limit: u32,
    account_id: Option<&str>,
) -> Result<Vec<minos_protocol::ThreadSummary>, BackendError> {
    let agent_s = agent.map(agent_str);
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            i64,
            i64,
            i64,
            Option<i64>,
            Option<String>,
        ),
    >(
        r"SELECT thread_id, agent, title, first_ts_ms, last_ts_ms, message_count, ended_at_ms, end_reason
           FROM threads
           WHERE (?1 IS NULL OR owner_device_id = ?1)
             AND (?2 IS NULL OR agent = ?2)
             AND (?3 IS NULL OR last_ts_ms < ?3)
             AND (
                 ?5 IS NULL
                 OR EXISTS (
                     SELECT 1 FROM devices d
                     WHERE d.device_id = threads.owner_device_id
                       AND d.account_id = ?5
                 )
             )
           ORDER BY last_ts_ms DESC
           LIMIT ?4",
    )
    .bind(owner_device_id)
    .bind(agent_s)
    .bind(before_ts_ms)
    .bind(i64::from(limit))
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "threads.list".into(),
        message: e.to_string(),
    })?;

    rows.into_iter()
        .map(
            |(
                thread_id,
                agent_s,
                title,
                first_ts_ms,
                last_ts_ms,
                message_count,
                ended_at_ms,
                end_reason_json,
            )| {
                let agent = match agent_s.as_str() {
                    "codex" => AgentName::Codex,
                    "claude" => AgentName::Claude,
                    "gemini" => AgentName::Gemini,
                    other => {
                        return Err(BackendError::StoreDecode {
                            column: "threads.agent".into(),
                            message: other.to_string(),
                        })
                    }
                };
                let end_reason = end_reason_json
                    .as_ref()
                    .map(|s| serde_json::from_str::<ThreadEndReason>(s))
                    .transpose()
                    .map_err(|e| BackendError::StoreDecode {
                        column: "threads.end_reason".into(),
                        message: e.to_string(),
                    })?;
                Ok(minos_protocol::ThreadSummary {
                    thread_id,
                    agent,
                    title,
                    first_ts_ms,
                    last_ts_ms,
                    message_count: u32::try_from(message_count).unwrap_or(u32::MAX),
                    ended_at_ms,
                    end_reason,
                })
            },
        )
        .collect()
}

/// Bump `message_count` by 1. Called when the translator places a new
/// `MessageStarted` — gives the list view a cheap "N messages" badge.
pub async fn increment_message_count(
    pool: &SqlitePool,
    thread_id: &str,
) -> Result<(), BackendError> {
    sqlx::query(r"UPDATE threads SET message_count = message_count + 1 WHERE thread_id = ?1")
        .bind(thread_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
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

    #[tokio::test]
    async fn list_orders_by_last_ts_desc_and_limits() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;
        for i in 0..5 {
            upsert(
                &pool,
                &format!("thr{i}"),
                AgentName::Codex,
                "dev1",
                i * 1000,
            )
            .await
            .unwrap();
        }

        let r = list(&pool, Some("dev1"), None, None, 3, None)
            .await
            .unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].thread_id, "thr4");
        assert_eq!(r[1].thread_id, "thr3");
        assert_eq!(r[2].thread_id, "thr2");
    }

    #[tokio::test]
    async fn list_filters_by_owner() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at)
               VALUES ('dev2','Other','agent-host',0,0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        upsert(&pool, "mine", AgentName::Codex, "dev1", 1000)
            .await
            .unwrap();
        upsert(&pool, "theirs", AgentName::Codex, "dev2", 2000)
            .await
            .unwrap();

        let r = list(&pool, Some("dev1"), None, None, 50, None)
            .await
            .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].thread_id, "mine");
    }

    #[tokio::test]
    async fn list_filters_by_account_id() {
        // Phase 2 Task 2.6: when an `account_id` is supplied, only
        // threads whose owner device row carries that account_id are
        // returned. Threads owned by devices on a different account, or
        // by devices with no account_id, must be excluded.
        let pool = memory_pool().await;
        // Account A + device owning thread "mine".
        let acct_a = crate::store::accounts::create(&pool, "alice@example.com", "phc")
            .await
            .unwrap();
        let acct_b = crate::store::accounts::create(&pool, "bob@example.com", "phc")
            .await
            .unwrap();
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at, account_id)
               VALUES ('a-mac','Mac-A','agent-host',0,0,?1)",
        )
        .bind(&acct_a.account_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at, account_id)
               VALUES ('b-mac','Mac-B','agent-host',0,0,?1)",
        )
        .bind(&acct_b.account_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r"INSERT INTO devices (device_id, display_name, role, created_at, last_seen_at)
               VALUES ('orphan','Mac-O','agent-host',0,0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        upsert(&pool, "thr-a", AgentName::Codex, "a-mac", 1000)
            .await
            .unwrap();
        upsert(&pool, "thr-b", AgentName::Codex, "b-mac", 2000)
            .await
            .unwrap();
        upsert(&pool, "thr-orphan", AgentName::Codex, "orphan", 3000)
            .await
            .unwrap();

        // Filtering by account A should return only thr-a.
        let r = list(&pool, None, None, None, 50, Some(&acct_a.account_id))
            .await
            .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].thread_id, "thr-a");

        // No account filter: all three.
        let r = list(&pool, None, None, None, 50, None).await.unwrap();
        assert_eq!(r.len(), 3);
    }

    #[tokio::test]
    async fn list_before_ts_cursor_excludes_boundary() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;
        for i in 0..5 {
            upsert(
                &pool,
                &format!("thr{i}"),
                AgentName::Codex,
                "dev1",
                i * 1000,
            )
            .await
            .unwrap();
        }

        // before_ts_ms = 3000 must strictly exclude last_ts_ms = 3000.
        let r = list(&pool, Some("dev1"), None, Some(3000), 50, None)
            .await
            .unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].thread_id, "thr2");
    }
}
