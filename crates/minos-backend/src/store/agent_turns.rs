//! Durable `agent_turns` storage and read helpers.

use sqlx::FromRow;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AgentTurnRow {
    pub turn_id: String,
    pub agent_session_id: String,
    pub turn_seq: i64,
    pub role: String,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub summary_text: Option<String>,
    pub usage_json: Option<String>,
}

pub async fn create(
    store: &impl AsStorePool,
    turn_id: &str,
    agent_session_id: &str,
    turn_seq: i64,
    role: &str,
    status: &str,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    summary_text: Option<&str>,
    usage_json: Option<&str>,
) -> Result<AgentTurnRow, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO agent_turns
                    (turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(turn_id)
            .bind(agent_session_id)
            .bind(turn_seq)
            .bind(role)
            .bind(status)
            .bind(started_at_ms)
            .bind(finished_at_ms)
            .bind(summary_text)
            .bind(usage_json)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO agent_turns
                    (turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(turn_id)
            .bind(agent_session_id)
            .bind(turn_seq)
            .bind(role)
            .bind(status)
            .bind(started_at_ms)
            .bind(finished_at_ms)
            .bind(summary_text)
            .bind(usage_json)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("agent_turns.create"))?;

    get(store, turn_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "agent_turns.create.load".into(),
            message: "turn missing after insert".into(),
        })
}

pub async fn get(
    store: &impl AsStorePool,
    turn_id: &str,
) -> Result<Option<AgentTurnRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentTurnRow>(
                "SELECT turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json
                   FROM agent_turns
                  WHERE turn_id = ?",
            )
            .bind(turn_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentTurnRow>(
                "SELECT turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json
                   FROM agent_turns
                  WHERE turn_id = $1",
            )
            .bind(turn_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("agent_turns.get"))
}

pub async fn get_for_account(
    store: &impl AsStorePool,
    turn_id: &str,
    account_id: &str,
) -> Result<Option<AgentTurnRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentTurnRow>(
                "SELECT t.turn_id, t.agent_session_id, t.turn_seq, t.role, t.status, t.started_at_ms, t.finished_at_ms, t.summary_text, t.usage_json
                   FROM agent_turns t
                   JOIN agent_sessions s
                     ON s.session_id = t.agent_session_id
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE t.turn_id = ?
                    AND cm.account_id = ?",
            )
            .bind(turn_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentTurnRow>(
                "SELECT t.turn_id, t.agent_session_id, t.turn_seq, t.role, t.status, t.started_at_ms, t.finished_at_ms, t.summary_text, t.usage_json
                   FROM agent_turns t
                   JOIN agent_sessions s
                     ON s.session_id = t.agent_session_id
                   JOIN conversation_members cm
                     ON cm.conversation_id = s.conversation_id
                  WHERE t.turn_id = $1
                    AND cm.account_id = $2",
            )
            .bind(turn_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("agent_turns.get_for_account"))
}

pub async fn list_for_session(
    store: &impl AsStorePool,
    session_id: &str,
    after_turn_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<AgentTurnRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentTurnRow>(
                "SELECT turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json
                   FROM agent_turns
                  WHERE agent_session_id = ?1
                    AND (?2 IS NULL OR turn_seq > ?2)
                  ORDER BY turn_seq ASC
                  LIMIT ?3",
            )
            .bind(session_id)
            .bind(after_turn_seq)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentTurnRow>(
                "SELECT turn_id, agent_session_id, turn_seq, role, status, started_at_ms, finished_at_ms, summary_text, usage_json
                   FROM agent_turns
                  WHERE agent_session_id = $1
                    AND ($2::BIGINT IS NULL OR turn_seq > $2)
                  ORDER BY turn_seq ASC
                  LIMIT $3",
            )
            .bind(session_id)
            .bind(after_turn_seq)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("agent_turns.list_for_session"))
}

pub async fn update_status(
    store: &impl AsStorePool,
    turn_id: &str,
    status: &str,
    finished_at_ms: Option<i64>,
) -> Result<Option<AgentTurnRow>, BackendError> {
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE agent_turns
                    SET status = ?, finished_at_ms = ?
                  WHERE turn_id = ?",
        )
        .bind(status)
        .bind(finished_at_ms)
        .bind(turn_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE agent_turns
                    SET status = $1, finished_at_ms = $2
                  WHERE turn_id = $3",
        )
        .bind(status)
        .bind(finished_at_ms)
        .bind(turn_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("agent_turns.update_status"))?;

    if rows_affected == 0 {
        return Ok(None);
    }

    get(store, turn_id).await
}

pub async fn update_summary_text(
    store: &impl AsStorePool,
    turn_id: &str,
    summary_text: Option<&str>,
) -> Result<Option<AgentTurnRow>, BackendError> {
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE agent_turns
                    SET summary_text = ?
                  WHERE turn_id = ?",
        )
        .bind(summary_text)
        .bind(turn_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE agent_turns
                    SET summary_text = $1
                  WHERE turn_id = $2",
        )
        .bind(summary_text)
        .bind(turn_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("agent_turns.update_summary_text"))?;

    if rows_affected == 0 {
        return Ok(None);
    }

    get(store, turn_id).await
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
    use crate::store::test_support::{insert_account, memory_pool};
    use crate::store::{agent_sessions, social};

    #[tokio::test]
    async fn list_for_session_uses_after_turn_seq() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "agent-turns@example.com").await;
        let members = vec![account_id.clone()];
        let conversation =
            social::create_group_conversation(&pool, &account_id, "Turn Paging", &members, 100)
                .await
                .unwrap();
        agent_sessions::create(
            &pool,
            "sess_turns",
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

        for seq in 1..=3 {
            create(
                &pool,
                &format!("turn_{seq}"),
                "sess_turns",
                seq,
                if seq == 1 { "user" } else { "assistant" },
                "completed",
                100 + seq,
                Some(100 + seq),
                None,
                None,
            )
            .await
            .unwrap();
        }

        let turns = list_for_session(&pool, "sess_turns", Some(1), 2)
            .await
            .unwrap();

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].turn_seq, 2);
        assert_eq!(turns[1].turn_seq, 3);
    }
}
