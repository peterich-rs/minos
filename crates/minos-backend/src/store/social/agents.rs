use std::collections::HashMap;

use sqlx::{Postgres, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::{get_message, store_err, AgentRow, ChatMessageRow};

pub async fn register_agent(
    store: &impl AsStorePool,
    owner_account_id: &str,
    name: &str,
    description: &str,
    runtime_agent: &str,
    model: &str,
    now_ms: i64,
) -> Result<AgentRow, BackendError> {
    let agent_id = format!("bot-{}", Uuid::new_v4());
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO agents (agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&agent_id)
            .bind(owner_account_id)
            .bind(name)
            .bind(description)
            .bind(runtime_agent)
            .bind(model)
            .bind(now_ms)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO agents (agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&agent_id)
            .bind(owner_account_id)
            .bind(name)
            .bind(description)
            .bind(runtime_agent)
            .bind(model)
            .bind(now_ms)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("social::register_agent"))?;

    get_agent(store, &agent_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::register_agent.load".into(),
            message: "agent missing after insert".into(),
        })
}

pub async fn get_agent(
    store: &impl AsStorePool,
    agent_id: &str,
) -> Result<Option<AgentRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentRow>(
                "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms
                   FROM agents WHERE agent_id = ?",
            )
            .bind(agent_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(
                "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms
                   FROM agents WHERE agent_id = $1",
            )
            .bind(agent_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::get_agent"))
}

pub async fn agents_by_ids(
    store: &impl AsStorePool,
    agent_ids: &[String],
) -> Result<HashMap<String, AgentRow>, BackendError> {
    if agent_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms\n           FROM agents\n          WHERE agent_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for agent_id in agent_ids {
                    separated.push_bind(agent_id);
                }
            }
            builder.push(')');
            builder.build_query_as::<AgentRow>().fetch_all(pool).await
        }
        StorePoolRef::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(
                "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms\n           FROM agents\n          WHERE agent_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for agent_id in agent_ids {
                    separated.push_bind(agent_id);
                }
            }
            builder.push(')');
            builder.build_query_as::<AgentRow>().fetch_all(pool).await
        }
    }
    .map_err(store_err("social::agents_by_ids"))?;

    Ok(rows
        .into_iter()
        .map(|row| (row.agent_id.clone(), row))
        .collect())
}

pub async fn list_agents_for_owner(
    store: &impl AsStorePool,
    owner_account_id: &str,
) -> Result<Vec<AgentRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentRow>(
                "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms
                   FROM agents WHERE owner_account_id = ? ORDER BY created_at_ms DESC",
            )
            .bind(owner_account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(
                "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms
                   FROM agents WHERE owner_account_id = $1 ORDER BY created_at_ms DESC",
            )
            .bind(owner_account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_agents_for_owner"))
}

pub async fn delete_agent(
    store: &impl AsStorePool,
    agent_id: &str,
    owner_account_id: &str,
) -> Result<bool, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query("DELETE FROM agents WHERE agent_id = ? AND owner_account_id = ?")
                .bind(agent_id)
                .bind(owner_account_id)
                .execute(pool)
                .await
                .map(|result| result.rows_affected())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query("DELETE FROM agents WHERE agent_id = $1 AND owner_account_id = $2")
                .bind(agent_id)
                .bind(owner_account_id)
                .execute(pool)
                .await
                .map(|result| result.rows_affected())
        }
    }
    .map_err(store_err("social::delete_agent"))?;

    Ok(result > 0)
}

pub async fn add_agent_to_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
    agent_id: &str,
    added_by_account_id: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT OR IGNORE INTO conversation_agent_members (conversation_id, agent_id, added_by_account_id, joined_at_ms)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(conversation_id)
            .bind(agent_id)
            .bind(added_by_account_id)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO conversation_agent_members (conversation_id, agent_id, added_by_account_id, joined_at_ms)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
            )
            .bind(conversation_id)
            .bind(agent_id)
            .bind(added_by_account_id)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("social::add_agent_to_conversation"))?;

    Ok(())
}

pub async fn remove_agent_from_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
    agent_id: &str,
) -> Result<bool, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "DELETE FROM conversation_agent_members WHERE conversation_id = ? AND agent_id = ?",
        )
        .bind(conversation_id)
        .bind(agent_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "DELETE FROM conversation_agent_members WHERE conversation_id = $1 AND agent_id = $2",
        )
        .bind(conversation_id)
        .bind(agent_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("social::remove_agent_from_conversation"))?;

    Ok(result > 0)
}

pub async fn list_conversation_agents(
    store: &impl AsStorePool,
    conversation_id: &str,
) -> Result<Vec<AgentRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentRow>(
                "SELECT a.agent_id, a.owner_account_id, a.name, a.description, a.runtime_agent, a.model, a.created_at_ms, a.updated_at_ms
                   FROM agents a
                   JOIN conversation_agent_members cam ON cam.agent_id = a.agent_id
                  WHERE cam.conversation_id = ?
                  ORDER BY cam.joined_at_ms ASC",
            )
            .bind(conversation_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(
                "SELECT a.agent_id, a.owner_account_id, a.name, a.description, a.runtime_agent, a.model, a.created_at_ms, a.updated_at_ms
                   FROM agents a
                   JOIN conversation_agent_members cam ON cam.agent_id = a.agent_id
                  WHERE cam.conversation_id = $1
                  ORDER BY cam.joined_at_ms ASC",
            )
            .bind(conversation_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_conversation_agents"))
}

pub async fn is_agent_in_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
    agent_id: &str,
) -> Result<bool, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM conversation_agent_members
                  WHERE conversation_id = ? AND agent_id = ?",
            )
            .bind(conversation_id)
            .bind(agent_id)
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM conversation_agent_members
                  WHERE conversation_id = $1 AND agent_id = $2",
            )
            .bind(conversation_id)
            .bind(agent_id)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("social::is_agent_in_conversation"))?;
    Ok(row > 0)
}

pub async fn insert_agent_message(
    store: &impl AsStorePool,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
    now_ms: i64,
    reply_to_message_id: Option<&str>,
    mentioned_account_ids: &[String],
) -> Result<ChatMessageRow, BackendError> {
    insert_agent_message_with_session(
        store,
        conversation_id,
        agent_id,
        text,
        now_ms,
        reply_to_message_id,
        None,
        mentioned_account_ids,
    )
    .await
}

pub async fn insert_agent_message_with_session(
    store: &impl AsStorePool,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
    now_ms: i64,
    reply_to_message_id: Option<&str>,
    agent_session_id: Option<&str>,
    mentioned_account_ids: &[String],
) -> Result<ChatMessageRow, BackendError> {
    let agent = get_agent(store, agent_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::insert_agent_message.load_agent".into(),
            message: format!("agent not found: {agent_id}"),
        })?;
    let message_id = Uuid::new_v4().to_string();

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::insert_agent_message.begin"))?;

            sqlx::query(
                "INSERT INTO chat_messages (
                    message_id,
                    conversation_id,
                    sender_account_id,
                    sender_agent_id,
                    text,
                    created_at_ms,
                    reply_to_message_id,
                    agent_session_id,
                    sender_type
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'agent')",
            )
            .bind(&message_id)
            .bind(conversation_id)
            .bind(&agent.owner_account_id)
            .bind(&agent.agent_id)
            .bind(text)
            .bind(now_ms)
            .bind(reply_to_message_id)
            .bind(agent_session_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::insert_agent_message.insert"))?;

            for mentioned_id in mentioned_account_ids {
                sqlx::query(
                    "INSERT OR IGNORE INTO chat_message_mentions (message_id, mentioned_account_id)
                     VALUES (?, ?)",
                )
                .bind(&message_id)
                .bind(mentioned_id)
                .execute(&mut *tx)
                .await
                .map_err(store_err("social::insert_agent_message.mention"))?;
            }

            sqlx::query("UPDATE conversations SET updated_at_ms = ? WHERE conversation_id = ?")
                .bind(now_ms)
                .bind(conversation_id)
                .execute(&mut *tx)
                .await
                .map_err(store_err(
                    "social::insert_agent_message.update_conversation",
                ))?;

            tx.commit()
                .await
                .map_err(store_err("social::insert_agent_message.commit"))?;

            get_message(pool, &message_id).await
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::insert_agent_message.begin"))?;

            sqlx::query(
                "INSERT INTO chat_messages (
                    message_id,
                    conversation_id,
                    sender_account_id,
                    sender_agent_id,
                    text,
                    created_at_ms,
                    reply_to_message_id,
                    agent_session_id,
                    sender_type
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'agent')",
            )
            .bind(&message_id)
            .bind(conversation_id)
            .bind(&agent.owner_account_id)
            .bind(&agent.agent_id)
            .bind(text)
            .bind(now_ms)
            .bind(reply_to_message_id)
            .bind(agent_session_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::insert_agent_message.insert"))?;

            for mentioned_id in mentioned_account_ids {
                sqlx::query(
                    "INSERT INTO chat_message_mentions (message_id, mentioned_account_id)
                     VALUES ($1, $2)
                     ON CONFLICT DO NOTHING",
                )
                .bind(&message_id)
                .bind(mentioned_id)
                .execute(&mut *tx)
                .await
                .map_err(store_err("social::insert_agent_message.mention"))?;
            }

            sqlx::query("UPDATE conversations SET updated_at_ms = $1 WHERE conversation_id = $2")
                .bind(now_ms)
                .bind(conversation_id)
                .execute(&mut *tx)
                .await
                .map_err(store_err(
                    "social::insert_agent_message.update_conversation",
                ))?;

            tx.commit()
                .await
                .map_err(store_err("social::insert_agent_message.commit"))?;

            get_message(pool, &message_id).await
        }
    }?
    .ok_or_else(|| BackendError::StoreQuery {
        operation: "social::insert_agent_message.load".into(),
        message: "message missing after insert".into(),
    })
}
