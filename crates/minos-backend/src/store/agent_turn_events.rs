//! `agent_turn_events` cold-replay storage.

use sqlx::FromRow;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AgentTurnEventRow {
    pub turn_id: String,
    pub event_seq: i64,
    pub kind: String,
    pub payload_json: String,
    pub created_at_ms: i64,
}

pub async fn append(
    store: &impl AsStorePool,
    turn_id: &str,
    event_seq: i64,
    kind: &str,
    payload: &serde_json::Value,
    created_at_ms: i64,
) -> Result<AgentTurnEventRow, BackendError> {
    let payload_json = serde_json::to_string(payload).map_err(|e| BackendError::StoreQuery {
        operation: "agent_turn_events.append.serialize".into(),
        message: e.to_string(),
    })?;

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO agent_turn_events (turn_id, event_seq, kind, payload_json, created_at_ms)
                 VALUES (?, ?, ?, ?, ?)",
        )
        .bind(turn_id)
        .bind(event_seq)
        .bind(kind)
        .bind(&payload_json)
        .bind(created_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO agent_turn_events (turn_id, event_seq, kind, payload_json, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(turn_id)
        .bind(event_seq)
        .bind(kind)
        .bind(&payload_json)
        .bind(created_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("agent_turn_events.append"))?;

    Ok(AgentTurnEventRow {
        turn_id: turn_id.to_string(),
        event_seq,
        kind: kind.to_string(),
        payload_json,
        created_at_ms,
    })
}

pub async fn list_for_turn(
    store: &impl AsStorePool,
    turn_id: &str,
    after_event_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<AgentTurnEventRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentTurnEventRow>(
                "SELECT turn_id, event_seq, kind, payload_json, created_at_ms
                   FROM agent_turn_events
                  WHERE turn_id = ?1
                    AND (?2 IS NULL OR event_seq > ?2)
                  ORDER BY event_seq ASC
                  LIMIT ?3",
            )
            .bind(turn_id)
            .bind(after_event_seq)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentTurnEventRow>(
                "SELECT turn_id, event_seq, kind, payload_json, created_at_ms
                   FROM agent_turn_events
                  WHERE turn_id = $1
                    AND ($2::BIGINT IS NULL OR event_seq > $2)
                  ORDER BY event_seq ASC
                  LIMIT $3",
            )
            .bind(turn_id)
            .bind(after_event_seq)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("agent_turn_events.list_for_turn"))
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
    use crate::store::{agent_sessions, agent_turns, social};

    #[tokio::test]
    async fn list_for_turn_uses_after_event_seq() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "agent-turn-events@example.com").await;
        let members = vec![account_id.clone()];
        let conversation =
            social::create_group_conversation(&pool, &account_id, "Event Paging", &members, 100)
                .await
                .unwrap();
        agent_sessions::create(
            &pool,
            "sess_events",
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
        agent_turns::create(
            &pool,
            "turn_events",
            "sess_events",
            1,
            "assistant",
            "completed",
            102,
            Some(103),
            None,
            None,
        )
        .await
        .unwrap();

        for seq in 1..=3 {
            append(
                &pool,
                "turn_events",
                seq,
                "agent_text_delta",
                &serde_json::json!({ "delta": format!("chunk-{seq}") }),
                200 + seq,
            )
            .await
            .unwrap();
        }

        let events = list_for_turn(&pool, "turn_events", Some(1), 2)
            .await
            .unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_seq, 2);
        assert_eq!(events[1].event_seq, 3);
    }
}
