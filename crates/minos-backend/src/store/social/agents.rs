use std::collections::HashMap;

use sqlx::{Postgres, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::{get_message, store_err, AgentRow, ChatMessageRow};

/// Legacy description marker (no longer used for lookup; kept for display).
pub const HOST_RUNTIME_AGENT_DESCRIPTION: &str = "minos:host-runtime";
/// Canonical agents.source value for Host/Desktop runtime projections.
pub const AGENT_SOURCE_USER: &str = "user";
pub const AGENT_SOURCE_HOST_RUNTIME: &str = "host_runtime";
pub const AGENT_SOURCE_SYSTEM: &str = "system";

const AGENT_SELECT_COLS: &str =
    "agent_id, owner_account_id, name, description, source, runtime_agent, model, workspace_path, created_at_ms, updated_at_ms";

pub async fn register_agent(
    store: &impl AsStorePool,
    owner_account_id: &str,
    name: &str,
    description: &str,
    runtime_agent: &str,
    model: &str,
    workspace_path: Option<&str>,
    now_ms: i64,
) -> Result<AgentRow, BackendError> {
    register_agent_with_source(
        store,
        owner_account_id,
        name,
        description,
        AGENT_SOURCE_USER,
        runtime_agent,
        model,
        workspace_path,
        now_ms,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn register_agent_with_source(
    store: &impl AsStorePool,
    owner_account_id: &str,
    name: &str,
    description: &str,
    source: &str,
    runtime_agent: &str,
    model: &str,
    workspace_path: Option<&str>,
    now_ms: i64,
) -> Result<AgentRow, BackendError> {
    let agent_id = format!("bot-{}", Uuid::new_v4());
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO agents (agent_id, owner_account_id, name, description, source, runtime_agent, model, workspace_path, created_at_ms, updated_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&agent_id)
            .bind(owner_account_id)
            .bind(name)
            .bind(description)
            .bind(source)
            .bind(runtime_agent)
            .bind(model)
            .bind(workspace_path)
            .bind(now_ms)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO agents (agent_id, owner_account_id, name, description, source, runtime_agent, model, workspace_path, created_at_ms, updated_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(&agent_id)
            .bind(owner_account_id)
            .bind(name)
            .bind(description)
            .bind(source)
            .bind(runtime_agent)
            .bind(model)
            .bind(workspace_path)
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

/// Find the Host/Desktop runtime agent for `(owner, runtime)`, if any.
pub async fn find_host_runtime_agent(
    store: &impl AsStorePool,
    owner_account_id: &str,
    runtime_agent: &str,
) -> Result<Option<AgentRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS}
                   FROM agents
                  WHERE owner_account_id = ?
                    AND runtime_agent = ?
                    AND source = ?
                  ORDER BY created_at_ms ASC
                  LIMIT 1"
            ))
            .bind(owner_account_id)
            .bind(runtime_agent)
            .bind(AGENT_SOURCE_HOST_RUNTIME)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS}
                   FROM agents
                  WHERE owner_account_id = $1
                    AND runtime_agent = $2
                    AND source = $3
                  ORDER BY created_at_ms ASC
                  LIMIT 1"
            ))
            .bind(owner_account_id)
            .bind(runtime_agent)
            .bind(AGENT_SOURCE_HOST_RUNTIME)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::find_host_runtime_agent"))
}

/// Ensure a Host/Desktop runtime agent exists (stable mapping local bin → cloud id).
pub async fn ensure_host_runtime_agent(
    store: &impl AsStorePool,
    owner_account_id: &str,
    runtime_agent: &str,
    name: &str,
    model: &str,
    workspace_path: Option<&str>,
    now_ms: i64,
) -> Result<AgentRow, BackendError> {
    if let Some(existing) = find_host_runtime_agent(store, owner_account_id, runtime_agent).await? {
        return Ok(existing);
    }
    register_agent_with_source(
        store,
        owner_account_id,
        name,
        HOST_RUNTIME_AGENT_DESCRIPTION,
        AGENT_SOURCE_HOST_RUNTIME,
        runtime_agent,
        model,
        workspace_path,
        now_ms,
    )
    .await
}

pub async fn get_agent(
    store: &impl AsStorePool,
    agent_id: &str,
) -> Result<Option<AgentRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS} FROM agents WHERE agent_id = ?"
            ))
            .bind(agent_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS} FROM agents WHERE agent_id = $1"
            ))
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
            let mut builder = QueryBuilder::<Sqlite>::new(format!(
                "SELECT {AGENT_SELECT_COLS}\n           FROM agents\n          WHERE agent_id IN ("
            ));
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
            let mut builder = QueryBuilder::<Postgres>::new(format!(
                "SELECT {AGENT_SELECT_COLS}\n           FROM agents\n          WHERE agent_id IN ("
            ));
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
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS}
                   FROM agents WHERE owner_account_id = ? ORDER BY created_at_ms DESC"
            ))
            .bind(owner_account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS}
                   FROM agents WHERE owner_account_id = $1 ORDER BY created_at_ms DESC"
            ))
            .bind(owner_account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_agents_for_owner"))
}

#[allow(clippy::too_many_arguments)]
pub async fn update_agent(
    store: &impl AsStorePool,
    agent_id: &str,
    owner_account_id: &str,
    name: &str,
    description: &str,
    runtime_agent: &str,
    model: &str,
    workspace_path: Option<&str>,
    now_ms: i64,
) -> Result<Option<AgentRow>, BackendError> {
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE agents
                    SET name = ?,
                        description = ?,
                        runtime_agent = ?,
                        model = ?,
                        workspace_path = ?,
                        updated_at_ms = ?
                  WHERE agent_id = ? AND owner_account_id = ?",
        )
        .bind(name)
        .bind(description)
        .bind(runtime_agent)
        .bind(model)
        .bind(workspace_path)
        .bind(now_ms)
        .bind(agent_id)
        .bind(owner_account_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE agents
                    SET name = $1,
                        description = $2,
                        runtime_agent = $3,
                        model = $4,
                        workspace_path = $5,
                        updated_at_ms = $6
                  WHERE agent_id = $7 AND owner_account_id = $8",
        )
        .bind(name)
        .bind(description)
        .bind(runtime_agent)
        .bind(model)
        .bind(workspace_path)
        .bind(now_ms)
        .bind(agent_id)
        .bind(owner_account_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("social::update_agent"))?;

    if rows_affected == 0 {
        return Ok(None);
    }
    get_agent(store, agent_id).await
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
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT a.agent_id, a.owner_account_id, a.name, a.description, a.source, a.runtime_agent, a.model, a.workspace_path, a.created_at_ms, a.updated_at_ms
                   FROM agents a
                   JOIN conversation_agent_members cam ON cam.agent_id = a.agent_id
                  WHERE cam.conversation_id = ?
                  ORDER BY cam.joined_at_ms ASC"
            ))
            .bind(conversation_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT a.agent_id, a.owner_account_id, a.name, a.description, a.source, a.runtime_agent, a.model, a.workspace_path, a.created_at_ms, a.updated_at_ms
                   FROM agents a
                   JOIN conversation_agent_members cam ON cam.agent_id = a.agent_id
                  WHERE cam.conversation_id = $1
                  ORDER BY cam.joined_at_ms ASC"
            ))
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
        None,
    )
    .await
}

/// Insert an agent chat message. Optional `client_message_id` makes multi-end
/// dual-write idempotent (Desktop `agent-result:…` ids).
#[allow(clippy::too_many_arguments)]
pub async fn insert_agent_message_with_session(
    store: &impl AsStorePool,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
    now_ms: i64,
    reply_to_message_id: Option<&str>,
    agent_session_id: Option<&str>,
    mentioned_account_ids: &[String],
    client_message_id: Option<&str>,
) -> Result<ChatMessageRow, BackendError> {
    let agent = get_agent(store, agent_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::insert_agent_message.load_agent".into(),
            message: format!("agent not found: {agent_id}"),
        })?;

    let message_id = match client_message_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => {
            if let Some(existing) = get_message(store, id).await? {
                if existing.conversation_id == conversation_id {
                    return Ok(existing);
                }
                return Err(BackendError::StoreQuery {
                    operation: "social::insert_agent_message.conflict".into(),
                    message: format!("message_id {id} already exists in a different conversation"),
                });
            }
            id.to_string()
        }
        None => Uuid::new_v4().to_string(),
    };

    let mut unique_mentions = mentioned_account_ids.to_vec();
    unique_mentions.sort();
    unique_mentions.dedup();

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

            for mentioned_id in &unique_mentions {
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

            for mentioned_id in &unique_mentions {
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
