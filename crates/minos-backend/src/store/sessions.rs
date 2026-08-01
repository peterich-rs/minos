//! `sessions` table CRUD (see spec §9.1).
//!
//! A `session` is one live session on an agent-host. Rows are created
//! implicitly by the first `raw_event` ingest (`upsert`) and mutated as
//! subsequent events arrive: `update_title` when the translator produces a
//! `SessionTitleUpdated`, `increment_message_count` when a new message is
//! placed, `mark_ended` when the backend sees `SessionClosed`.
//!
//! The formal agent-session and project APIs still use these summaries while
//! the ingest path is being folded into room-first storage.

use std::collections::HashMap;

use minos_domain::AgentName;
use minos_ui_protocol::SessionEndReason;
use sqlx::{Postgres, QueryBuilder, Sqlite};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type SessionSummaryRow = (
    String,
    String,
    Option<String>,
    i64,
    i64,
    i64,
    Option<i64>,
    Option<String>,
);

/// Wire-value string for an `AgentName`, matching the DB CHECK constraint.
fn agent_str(a: AgentName) -> &'static str {
    match a {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
        AgentName::Gemini => "gemini",
        AgentName::Opencode => "opencode",
        AgentName::Grok => "grok",
    }
}

/// Insert-or-bump: on first ingest, create the row; on subsequent ingests
/// for the same `session_id`, update `last_ts_ms` to `ts_ms`. `first_ts_ms`
/// is frozen at insert time, `message_count` starts at 0.
pub async fn upsert(
    store: &impl AsStorePool,
    session_id: &str,
    agent: AgentName,
    owner_device_id: &str,
    ts_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                r"INSERT INTO sessions (session_id, agent, owner_device_id, first_ts_ms, last_ts_ms, message_count)
                   VALUES (?1, ?2, ?3, ?4, ?4, 0)
                   ON CONFLICT(session_id) DO UPDATE SET last_ts_ms = ?4",
            )
            .bind(session_id)
            .bind(agent_str(agent))
            .bind(owner_device_id)
            .bind(ts_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                r"INSERT INTO sessions (session_id, agent, owner_device_id, first_ts_ms, last_ts_ms, message_count)
                   VALUES ($1, $2, $3, $4, $4, 0)
                   ON CONFLICT(session_id) DO UPDATE SET last_ts_ms = $4",
            )
            .bind(session_id)
            .bind(agent_str(agent))
            .bind(owner_device_id)
            .bind(ts_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "sessions.upsert".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Mark a session as ended. `reason` is serialised as the same JSON the wire
/// protocol uses — `serde_json::to_string` on a `SessionEndReason` produces
/// `{"kind":"agent_done"}` etc.
pub async fn mark_ended(
    store: &impl AsStorePool,
    session_id: &str,
    reason: &SessionEndReason,
    ts_ms: i64,
) -> Result<(), BackendError> {
    let reason_json = serde_json::to_string(reason).map_err(|e| BackendError::StoreQuery {
        operation: "sessions.mark_ended.serialise".into(),
        message: e.to_string(),
    })?;
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            r"UPDATE sessions SET ended_at_ms = ?1, end_reason = ?2 WHERE session_id = ?3",
        )
        .bind(ts_ms)
        .bind(&reason_json)
        .bind(session_id)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            r"UPDATE sessions SET ended_at_ms = $1, end_reason = $2 WHERE session_id = $3",
        )
        .bind(ts_ms)
        .bind(&reason_json)
        .bind(session_id)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "sessions.mark_ended".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Set the human-friendly title. Called when the translator emits
/// `SessionTitleUpdated` (codex surfaces this as a separate notification).
pub async fn update_title(
    store: &impl AsStorePool,
    session_id: &str,
    title: &str,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(r"UPDATE sessions SET title = ?1 WHERE session_id = ?2")
                .bind(title)
                .bind(session_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(r"UPDATE sessions SET title = $1 WHERE session_id = $2")
                .bind(title)
                .bind(session_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "sessions.update_title".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Check whether a session exists and is visible to `account_id`.
///
/// Visibility: owner installation bound to the account, **or** owner is a
/// host linked via `host_links` (hosts keep `account_id` NULL).
pub async fn exists_for_account(
    store: &impl AsStorePool,
    session_id: &str,
    account_id: &str,
) -> Result<bool, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
                   FROM sessions t
                  WHERE t.session_id = ?
                    AND (
                        EXISTS (
                            SELECT 1 FROM device_installations d
                             WHERE d.installation_id = t.owner_device_id
                               AND d.account_id = ?
                        )
                        OR EXISTS (
                            SELECT 1 FROM host_links hl
                             WHERE hl.host_installation_id = t.owner_device_id
                               AND hl.account_id = ?
                        )
                    )",
        )
        .bind(session_id)
        .bind(account_id)
        .bind(account_id)
        .fetch_one(pool)
        .await
        .map(|count| count > 0),
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                    SELECT 1
                      FROM sessions t
                     WHERE t.session_id = $1
                       AND (
                           EXISTS (
                               SELECT 1 FROM device_installations d
                                WHERE d.installation_id = t.owner_device_id
                                  AND d.account_id = $2
                           )
                           OR EXISTS (
                               SELECT 1 FROM host_links hl
                                WHERE hl.host_installation_id = t.owner_device_id
                                  AND hl.account_id = $2
                           )
                       )
                )",
            )
            .bind(session_id)
            .bind(account_id)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "sessions.exists_for_account".into(),
        message: e.to_string(),
    })
}

/// List thread summaries for formal agent-session/project query responses.
///
/// Filters (all optional):
/// - `owner_device_id`  — restrict to sessions owned by this device.
/// - `agent`            — restrict to a single CLI agent.
/// - `before_ts_ms`     — only sessions whose `last_ts_ms` is strictly less
///   than this (exclusive cursor for pagination).
/// - `account_id`       — restrict to sessions visible to this account:
///   owner installation bound to the account, or host linked via
///   `host_links` (hosts keep `account_id` NULL).
///
/// Ordering: `last_ts_ms DESC` — most-recently-active first. Capped at
/// `limit` rows; the caller pins the upper bound in the dispatch layer.
pub async fn list(
    store: &impl AsStorePool,
    owner_device_id: Option<&str>,
    agent: Option<AgentName>,
    before_ts_ms: Option<i64>,
    limit: u32,
    account_id: Option<&str>,
) -> Result<Vec<minos_protocol::SessionSummary>, BackendError> {
    let agent_s = agent.map(agent_str);
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, SessionSummaryRow>(
                r"SELECT session_id, agent, title, first_ts_ms, last_ts_ms, message_count, ended_at_ms, end_reason
                   FROM sessions
                   WHERE (?1 IS NULL OR owner_device_id = ?1)
                     AND (?2 IS NULL OR agent = ?2)
                     AND (?3 IS NULL OR last_ts_ms < ?3)
                     AND (
                         ?5 IS NULL
                         OR EXISTS (
                             SELECT 1 FROM device_installations d
                             WHERE d.installation_id = sessions.owner_device_id
                               AND d.account_id = ?5
                         )
                         OR EXISTS (
                             SELECT 1 FROM host_links hl
                             WHERE hl.host_installation_id = sessions.owner_device_id
                               AND hl.account_id = ?5
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
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, SessionSummaryRow>(
                r"SELECT session_id, agent, title, first_ts_ms, last_ts_ms, message_count, ended_at_ms, end_reason
                   FROM sessions
                   WHERE ($1::TEXT IS NULL OR owner_device_id = $1)
                     AND ($2::TEXT IS NULL OR agent = $2)
                     AND ($3::BIGINT IS NULL OR last_ts_ms < $3)
                     AND (
                         $5::TEXT IS NULL
                         OR EXISTS (
                             SELECT 1 FROM device_installations d
                             WHERE d.installation_id = sessions.owner_device_id
                               AND d.account_id = $5
                         )
                         OR EXISTS (
                             SELECT 1 FROM host_links hl
                             WHERE hl.host_installation_id = sessions.owner_device_id
                               AND hl.account_id = $5
                         )
                     )
                   ORDER BY last_ts_ms DESC
                   LIMIT $4",
            )
            .bind(owner_device_id)
            .bind(agent_s)
            .bind(before_ts_ms)
            .bind(i64::from(limit))
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "sessions.list".into(),
        message: e.to_string(),
    })?;

    rows.into_iter().map(decode_thread_summary_row).collect()
}

/// Load thread summaries for a specific set of ids, scoped to one account.
pub async fn summaries_for_ids(
    store: &impl AsStorePool,
    account_id: &str,
    session_ids: &[String],
) -> Result<HashMap<String, minos_protocol::SessionSummary>, BackendError> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<SessionSummaryRow> = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut query = QueryBuilder::<Sqlite>::new(
                "SELECT session_id, agent, title, first_ts_ms, last_ts_ms, message_count, ended_at_ms, end_reason \
                 FROM sessions WHERE session_id IN (",
            );
            {
                let mut separated = query.separated(", ");
                for session_id in session_ids {
                    separated.push_bind(session_id);
                }
            }
            query.push(
                ") AND (\
                    EXISTS (\
                        SELECT 1 FROM device_installations d \
                        WHERE d.installation_id = sessions.owner_device_id \
                          AND d.account_id = ",
            );
            query.push_bind(account_id);
            query.push(
                ") OR EXISTS (\
                        SELECT 1 FROM host_links hl \
                        WHERE hl.host_installation_id = sessions.owner_device_id \
                          AND hl.account_id = ",
            );
            query.push_bind(account_id);
            query.push("))");

            query.build_query_as().fetch_all(pool).await
        }
        StorePoolRef::Postgres(pool) => {
            let mut query = QueryBuilder::<Postgres>::new(
                "SELECT session_id, agent, title, first_ts_ms, last_ts_ms, message_count, ended_at_ms, end_reason \
                 FROM sessions WHERE session_id IN (",
            );
            {
                let mut separated = query.separated(", ");
                for session_id in session_ids {
                    separated.push_bind(session_id);
                }
            }
            query.push(
                ") AND (\
                    EXISTS (\
                        SELECT 1 FROM device_installations d \
                        WHERE d.installation_id = sessions.owner_device_id \
                          AND d.account_id = ",
            );
            query.push_bind(account_id);
            query.push(
                ") OR EXISTS (\
                        SELECT 1 FROM host_links hl \
                        WHERE hl.host_installation_id = sessions.owner_device_id \
                          AND hl.account_id = ",
            );
            query.push_bind(account_id);
            query.push("))");

            query.build_query_as().fetch_all(pool).await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "sessions.summaries_for_ids".into(),
        message: e.to_string(),
    })?;

    let mut summaries = HashMap::with_capacity(rows.len());
    for row in rows {
        let summary = decode_thread_summary_row(row)?;
        summaries.insert(summary.session_id.clone(), summary);
    }
    Ok(summaries)
}

/// Bump `message_count` by 1. Called when the translator places a new
/// `MessageStarted` — gives the list view a cheap "N messages" badge.
pub async fn increment_message_count(
    store: &impl AsStorePool,
    session_id: &str,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            r"UPDATE sessions SET message_count = message_count + 1 WHERE session_id = ?1",
        )
        .bind(session_id)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            r"UPDATE sessions SET message_count = message_count + 1 WHERE session_id = $1",
        )
        .bind(session_id)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "sessions.increment_message_count".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

fn decode_thread_summary_row(
    (
        session_id,
        agent_s,
        title,
        first_ts_ms,
        last_ts_ms,
        message_count,
        ended_at_ms,
        end_reason_json,
    ): SessionSummaryRow,
) -> Result<minos_protocol::SessionSummary, BackendError> {
    let agent = match agent_s.as_str() {
        "codex" => AgentName::Codex,
        "claude" => AgentName::Claude,
        "gemini" => AgentName::Gemini,
        "opencode" => AgentName::Opencode,
        "grok" => AgentName::Grok,
        other => {
            return Err(BackendError::StoreDecode {
                column: "sessions.agent".into(),
                message: other.to_string(),
            })
        }
    };
    let end_reason = end_reason_json
        .as_ref()
        .map(|s| serde_json::from_str::<SessionEndReason>(s))
        .transpose()
        .map_err(|e| BackendError::StoreDecode {
            column: "sessions.end_reason".into(),
            message: e.to_string(),
        })?;
    let state = if ended_at_ms.is_some() {
        minos_protocol::SessionState::Closed {
            reason: minos_protocol::CloseReason::UserClose,
        }
    } else {
        minos_protocol::SessionState::Idle
    };
    Ok(minos_protocol::SessionSummary {
        session_id,
        agent,
        title,
        first_ts_ms,
        last_ts_ms,
        message_count: u32::try_from(message_count).unwrap_or(u32::MAX),
        ended_at_ms,
        end_reason,
        parent_session_id: None,
        state,
        needs_continue: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;
    use sqlx::SqlitePool;

    async fn seed_agent_host(pool: &SqlitePool) {
        sqlx::query(
            r"INSERT INTO device_installations (installation_id, kind, display_name, created_at_ms, last_seen_at_ms)
               VALUES ('dev1','host','Dev',0,0)",
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

        let (first, last): (i64, i64) = sqlx::query_as(
            "SELECT first_ts_ms, last_ts_ms FROM sessions WHERE session_id = 'thr1'",
        )
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

        mark_ended(&pool, "thr1", &SessionEndReason::HostDisconnected, 2000)
            .await
            .unwrap();

        let (ended_at, reason): (Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT ended_at_ms, end_reason FROM sessions WHERE session_id = 'thr1'",
        )
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
            sqlx::query_scalar("SELECT title FROM sessions WHERE session_id = 'thr1'")
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
            sqlx::query_scalar("SELECT message_count FROM sessions WHERE session_id = 'thr1'")
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
        assert_eq!(r[0].session_id, "thr4");
        assert_eq!(r[1].session_id, "thr3");
        assert_eq!(r[2].session_id, "thr2");
    }

    #[tokio::test]
    async fn list_filters_by_owner() {
        let pool = memory_pool().await;
        seed_agent_host(&pool).await;
        sqlx::query(
            r"INSERT INTO device_installations (installation_id, kind, display_name, created_at_ms, last_seen_at_ms)
               VALUES ('dev2','host','Other',0,0)",
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
        assert_eq!(r[0].session_id, "mine");
    }

    #[tokio::test]
    async fn list_filters_by_account_id() {
        // When an `account_id` is supplied, only sessions whose owner
        // installation carries that account_id are returned. Hosts keep
        // account_id NULL; use client installations as owners here.
        let pool = memory_pool().await;
        let acct_a = crate::store::accounts::create(&pool, "alice@example.com", "phc")
            .await
            .unwrap();
        let acct_b = crate::store::accounts::create(&pool, "bob@example.com", "phc")
            .await
            .unwrap();
        sqlx::query(
            r"INSERT INTO device_installations
                (installation_id, kind, display_name, created_at_ms, last_seen_at_ms, account_id)
               VALUES ('a-phone','mobile','Phone-A',0,0,?1)",
        )
        .bind(&acct_a.account_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r"INSERT INTO device_installations
                (installation_id, kind, display_name, created_at_ms, last_seen_at_ms, account_id)
               VALUES ('b-phone','mobile','Phone-B',0,0,?1)",
        )
        .bind(&acct_b.account_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r"INSERT INTO device_installations
                (installation_id, kind, display_name, created_at_ms, last_seen_at_ms)
               VALUES ('orphan','mobile','Phone-O',0,0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        upsert(&pool, "thr-a", AgentName::Codex, "a-phone", 1000)
            .await
            .unwrap();
        upsert(&pool, "thr-b", AgentName::Codex, "b-phone", 2000)
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
        assert_eq!(r[0].session_id, "thr-a");

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
        assert_eq!(r[0].session_id, "thr2");
    }
}
