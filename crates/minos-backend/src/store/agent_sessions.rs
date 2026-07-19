//! Additive `agent_sessions` storage.

use sqlx::FromRow;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AgentSessionRow {
    pub session_id: String,
    pub conversation_id: String,
    pub project_id: Option<String>,
    pub host_device_id: Option<String>,
    pub agent_id: Option<String>,
    pub status: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
}

pub async fn create(
    store: &impl AsStorePool,
    session_id: &str,
    conversation_id: &str,
    project_id: Option<&str>,
    host_device_id: Option<&str>,
    agent_id: Option<&str>,
    status: &str,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
) -> Result<AgentSessionRow, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO agent_sessions
                    (session_id, conversation_id, project_id, host_device_id, agent_id, status, started_at_ms, ended_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session_id)
            .bind(conversation_id)
            .bind(project_id)
            .bind(host_device_id)
            .bind(agent_id)
            .bind(status)
            .bind(started_at_ms)
            .bind(ended_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO agent_sessions
                    (session_id, conversation_id, project_id, host_device_id, agent_id, status, started_at_ms, ended_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(session_id)
            .bind(conversation_id)
            .bind(project_id)
            .bind(host_device_id)
            .bind(agent_id)
            .bind(status)
            .bind(started_at_ms)
            .bind(ended_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("agent_sessions.create"))?;

    get(store, session_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "agent_sessions.create.load".into(),
            message: "session missing after insert".into(),
        })
}

pub async fn get(
    store: &impl AsStorePool,
    session_id: &str,
) -> Result<Option<AgentSessionRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT session_id, conversation_id, project_id, host_device_id, agent_id, status, started_at_ms, ended_at_ms
                   FROM agent_sessions
                  WHERE session_id = ?",
            )
            .bind(session_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT session_id, conversation_id, project_id, host_device_id, agent_id, status, started_at_ms, ended_at_ms
                   FROM agent_sessions
                  WHERE session_id = $1",
            )
            .bind(session_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("agent_sessions.get"))
}

pub async fn get_for_account(
    store: &impl AsStorePool,
    session_id: &str,
    account_id: &str,
) -> Result<Option<AgentSessionRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.session_id = ?
                    AND cm.account_id = ?",
            )
            .bind(session_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.session_id = $1
                    AND cm.account_id = $2",
            )
            .bind(session_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("agent_sessions.get_for_account"))
}

pub async fn latest_for_account_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
) -> Result<Option<AgentSessionRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.conversation_id = ?
                    AND cm.account_id = ?
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT 1",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.conversation_id = $1
                    AND cm.account_id = $2
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT 1",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("agent_sessions.latest_for_account_conversation"))
}

pub async fn list_for_account(
    store: &impl AsStorePool,
    account_id: &str,
    before_started_at_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<AgentSessionRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE cm.account_id = ?
                    AND (? IS NULL OR s.started_at_ms < ?)
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT ?",
            )
            .bind(account_id)
            .bind(before_started_at_ms)
            .bind(before_started_at_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE cm.account_id = $1
                    AND ($2::BIGINT IS NULL OR s.started_at_ms < $3)
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT $4",
            )
            .bind(account_id)
            .bind(before_started_at_ms)
            .bind(before_started_at_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("agent_sessions.list_for_account"))
}

pub async fn list_for_account_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
    before_started_at_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<AgentSessionRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.conversation_id = ?
                    AND cm.account_id = ?
                    AND (? IS NULL OR s.started_at_ms < ?)
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT ?",
            )
            .bind(conversation_id)
            .bind(account_id)
            .bind(before_started_at_ms)
            .bind(before_started_at_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.conversation_id = $1
                    AND cm.account_id = $2
                    AND ($3::BIGINT IS NULL OR s.started_at_ms < $4)
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT $5",
            )
            .bind(conversation_id)
            .bind(account_id)
            .bind(before_started_at_ms)
            .bind(before_started_at_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("agent_sessions.list_for_account_conversation"))
}

pub async fn list_for_account_project(
    store: &impl AsStorePool,
    project_id: &str,
    account_id: &str,
    before_started_at_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<AgentSessionRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.project_id = ?
                    AND cm.account_id = ?
                    AND (? IS NULL OR s.started_at_ms < ?)
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT ?",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(before_started_at_ms)
            .bind(before_started_at_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentSessionRow>(
                "SELECT s.session_id, s.conversation_id, s.project_id, s.host_device_id, s.agent_id, s.status, s.started_at_ms, s.ended_at_ms
                   FROM agent_sessions s
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE s.project_id = $1
                    AND cm.account_id = $2
                    AND ($3::BIGINT IS NULL OR s.started_at_ms < $4)
                  ORDER BY s.started_at_ms DESC, s.session_id DESC
                  LIMIT $5",
            )
            .bind(project_id)
            .bind(account_id)
            .bind(before_started_at_ms)
            .bind(before_started_at_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("agent_sessions.list_for_account_project"))
}

pub async fn assign_project_for_account(
    store: &impl AsStorePool,
    session_id: &str,
    account_id: &str,
    project_id: Option<&str>,
) -> Result<Option<AgentSessionRow>, BackendError> {
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE agent_sessions
                    SET project_id = ?
                  WHERE session_id = ?
                    AND conversation_id IN (
                        SELECT cm.conversation_id
                          FROM conversation_members cm
                         WHERE cm.account_id = ?
                    )",
        )
        .bind(project_id)
        .bind(session_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE agent_sessions
                    SET project_id = $1
                  WHERE session_id = $2
                    AND conversation_id IN (
                        SELECT cm.conversation_id
                          FROM conversation_members cm
                         WHERE cm.account_id = $3
                    )",
        )
        .bind(project_id)
        .bind(session_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("agent_sessions.assign_project_for_account"))?;

    if rows_affected == 0 {
        return Ok(None);
    }

    get_for_account(store, session_id, account_id).await
}

pub async fn update_status(
    store: &impl AsStorePool,
    session_id: &str,
    status: &str,
    ended_at_ms: Option<i64>,
) -> Result<Option<AgentSessionRow>, BackendError> {
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE agent_sessions
                    SET status = ?, ended_at_ms = ?
                  WHERE session_id = ?",
        )
        .bind(status)
        .bind(ended_at_ms)
        .bind(session_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE agent_sessions
                    SET status = $1, ended_at_ms = $2
                  WHERE session_id = $3",
        )
        .bind(status)
        .bind(ended_at_ms)
        .bind(session_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("agent_sessions.update_status"))?;

    if rows_affected == 0 {
        return Ok(None);
    }

    get(store, session_id).await
}

pub async fn claim_host_if_empty(
    store: &impl AsStorePool,
    session_id: &str,
    host_device_id: &str,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE agent_sessions
                    SET host_device_id = ?
                  WHERE session_id = ?
                    AND host_device_id IS NULL",
        )
        .bind(host_device_id)
        .bind(session_id)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE agent_sessions
                    SET host_device_id = $1
                  WHERE session_id = $2
                    AND host_device_id IS NULL",
        )
        .bind(host_device_id)
        .bind(session_id)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("agent_sessions.claim_host_if_empty"))
}

fn store_err(operation: &'static str) -> impl Fn(sqlx::Error) -> BackendError {
    move |e| BackendError::StoreQuery {
        operation: operation.into(),
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::social;
    use crate::store::test_support::{insert_account, memory_pool};

    #[tokio::test]
    async fn get_for_account_scopes_by_conversation_membership() {
        let pool = memory_pool().await;
        let account_a = insert_account(&pool, "agent-session-a@example.com").await;
        let account_b = insert_account(&pool, "agent-session-b@example.com").await;

        let members = vec![account_a.clone()];
        let conversation =
            social::create_group_conversation(&pool, &account_a, "Session Scope", &members, 100)
                .await
                .unwrap();

        create(
            &pool,
            "sess_scope",
            &conversation.conversation_id,
            None,
            None,
            None,
            "running",
            101,
            None,
        )
        .await
        .unwrap();

        let visible = get_for_account(&pool, "sess_scope", &account_a)
            .await
            .unwrap();
        let hidden = get_for_account(&pool, "sess_scope", &account_b)
            .await
            .unwrap();

        assert!(visible.is_some());
        assert!(hidden.is_none());
    }

    #[tokio::test]
    async fn assign_project_for_account_updates_project_scope_and_lists_by_project() {
        let pool = memory_pool().await;
        let account = insert_account(&pool, "agent-session-project@example.com").await;

        crate::store::projects::create(
            &pool,
            "proj-session-scope",
            &account,
            "Project Session Scope",
            "project-session-scope",
            None,
            99,
        )
        .await
        .unwrap();

        let members = vec![account.clone()];
        let conversation =
            social::create_group_conversation(&pool, &account, "Project Scoped", &members, 100)
                .await
                .unwrap();

        create(
            &pool,
            "sess_project_scope",
            &conversation.conversation_id,
            None,
            None,
            Some("agent_codex"),
            "running",
            101,
            None,
        )
        .await
        .unwrap();

        let assigned = assign_project_for_account(
            &pool,
            "sess_project_scope",
            &account,
            Some("proj-session-scope"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(assigned.project_id.as_deref(), Some("proj-session-scope"));

        let project_rows =
            list_for_account_project(&pool, "proj-session-scope", &account, None, 10)
                .await
                .unwrap();
        assert_eq!(project_rows.len(), 1);
        assert_eq!(project_rows[0].session_id, "sess_project_scope");

        let conversation_rows =
            list_for_account_conversation(&pool, &conversation.conversation_id, &account, None, 10)
                .await
                .unwrap();
        assert_eq!(conversation_rows.len(), 1);
        assert_eq!(
            conversation_rows[0].project_id.as_deref(),
            Some("proj-session-scope")
        );
    }
}
