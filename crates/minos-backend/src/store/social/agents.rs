use std::collections::HashMap;

use sqlx::{Postgres, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::{store_err, AgentRow, ChatMessageRow};

/// Legacy description marker (no longer used for lookup; kept for display).
pub const HOST_RUNTIME_AGENT_DESCRIPTION: &str = "minos:host-runtime";
/// Canonical agents.source value for Host/Desktop runtime projections.
pub const AGENT_SOURCE_USER: &str = "user";
pub const AGENT_SOURCE_HOST_RUNTIME: &str = "host_runtime";
pub const AGENT_SOURCE_SYSTEM: &str = "system";

const AGENT_SELECT_COLS: &str = "agent_id, owner_account_id, name, display_name, description, \
     avatar_url, source, status, runtime_agent, model, default_reasoning_effort, system_prompt, \
     workspace_path, created_at_ms, updated_at_ms";

pub const AGENT_STATUS_ACTIVE: &str = "active";
pub const AGENT_STATUS_DISABLED: &str = "disabled";

/// Parameters for creating a global bot identity (digital body).
#[derive(Debug, Clone)]
pub struct RegisterAgentParams<'a> {
    pub owner_account_id: &'a str,
    pub name: &'a str,
    pub display_name: &'a str,
    pub description: &'a str,
    pub avatar_url: Option<&'a str>,
    pub source: &'a str,
    pub runtime_agent: &'a str,
    pub model: &'a str,
    pub default_reasoning_effort: &'a str,
    pub system_prompt: &'a str,
    pub workspace_path: Option<&'a str>,
    pub now_ms: i64,
}

/// Parameters for updating a global bot owned by the caller.
#[derive(Debug, Clone)]
pub struct UpdateAgentParams<'a> {
    pub agent_id: &'a str,
    pub owner_account_id: &'a str,
    pub name: &'a str,
    pub display_name: &'a str,
    pub description: &'a str,
    pub avatar_url: Option<&'a str>,
    pub status: &'a str,
    pub runtime_agent: &'a str,
    pub model: &'a str,
    pub default_reasoning_effort: &'a str,
    pub system_prompt: &'a str,
    pub workspace_path: Option<&'a str>,
    pub now_ms: i64,
}

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
    register_agent_full(
        store,
        RegisterAgentParams {
            owner_account_id,
            name,
            display_name: name,
            description,
            avatar_url: None,
            source: AGENT_SOURCE_USER,
            runtime_agent,
            model,
            default_reasoning_effort: "",
            system_prompt: "",
            workspace_path,
            now_ms,
        },
    )
    .await
}

pub async fn register_agent_full(
    store: &impl AsStorePool,
    params: RegisterAgentParams<'_>,
) -> Result<AgentRow, BackendError> {
    let display_name = if params.display_name.trim().is_empty() {
        params.name
    } else {
        params.display_name
    };
    let agent_id = format!("bot-{}", Uuid::new_v4());
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO agents (
                    agent_id, owner_account_id, name, display_name, description, avatar_url,
                    source, status, runtime_agent, model, default_reasoning_effort, system_prompt,
                    workspace_path, created_at_ms, updated_at_ms
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&agent_id)
            .bind(params.owner_account_id)
            .bind(params.name)
            .bind(display_name)
            .bind(params.description)
            .bind(params.avatar_url)
            .bind(params.source)
            .bind(AGENT_STATUS_ACTIVE)
            .bind(params.runtime_agent)
            .bind(params.model)
            .bind(params.default_reasoning_effort)
            .bind(params.system_prompt)
            .bind(params.workspace_path)
            .bind(params.now_ms)
            .bind(params.now_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO agents (
                    agent_id, owner_account_id, name, display_name, description, avatar_url,
                    source, status, runtime_agent, model, default_reasoning_effort, system_prompt,
                    workspace_path, created_at_ms, updated_at_ms
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
            )
            .bind(&agent_id)
            .bind(params.owner_account_id)
            .bind(params.name)
            .bind(display_name)
            .bind(params.description)
            .bind(params.avatar_url)
            .bind(params.source)
            .bind(AGENT_STATUS_ACTIVE)
            .bind(params.runtime_agent)
            .bind(params.model)
            .bind(params.default_reasoning_effort)
            .bind(params.system_prompt)
            .bind(params.workspace_path)
            .bind(params.now_ms)
            .bind(params.now_ms)
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
    register_agent_full(
        store,
        RegisterAgentParams {
            owner_account_id,
            name,
            display_name: name,
            description: HOST_RUNTIME_AGENT_DESCRIPTION,
            avatar_url: None,
            source: AGENT_SOURCE_HOST_RUNTIME,
            runtime_agent,
            model,
            default_reasoning_effort: "",
            system_prompt: "",
            workspace_path,
            now_ms,
        },
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
    update_agent_full(
        store,
        UpdateAgentParams {
            agent_id,
            owner_account_id,
            name,
            display_name: name,
            description,
            avatar_url: None,
            status: AGENT_STATUS_ACTIVE,
            runtime_agent,
            model,
            default_reasoning_effort: "",
            system_prompt: "",
            workspace_path,
            now_ms,
        },
    )
    .await
}

pub async fn update_agent_full(
    store: &impl AsStorePool,
    params: UpdateAgentParams<'_>,
) -> Result<Option<AgentRow>, BackendError> {
    let display_name = if params.display_name.trim().is_empty() {
        params.name
    } else {
        params.display_name
    };
    let status = if params.status.trim().is_empty() {
        AGENT_STATUS_ACTIVE
    } else {
        params.status
    };
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE agents
                    SET name = ?,
                        display_name = ?,
                        description = ?,
                        avatar_url = ?,
                        status = ?,
                        runtime_agent = ?,
                        model = ?,
                        default_reasoning_effort = ?,
                        system_prompt = ?,
                        workspace_path = ?,
                        updated_at_ms = ?
                  WHERE agent_id = ? AND owner_account_id = ?",
        )
        .bind(params.name)
        .bind(display_name)
        .bind(params.description)
        .bind(params.avatar_url)
        .bind(status)
        .bind(params.runtime_agent)
        .bind(params.model)
        .bind(params.default_reasoning_effort)
        .bind(params.system_prompt)
        .bind(params.workspace_path)
        .bind(params.now_ms)
        .bind(params.agent_id)
        .bind(params.owner_account_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE agents
                    SET name = $1,
                        display_name = $2,
                        description = $3,
                        avatar_url = $4,
                        status = $5,
                        runtime_agent = $6,
                        model = $7,
                        default_reasoning_effort = $8,
                        system_prompt = $9,
                        workspace_path = $10,
                        updated_at_ms = $11
                  WHERE agent_id = $12 AND owner_account_id = $13",
        )
        .bind(params.name)
        .bind(display_name)
        .bind(params.description)
        .bind(params.avatar_url)
        .bind(status)
        .bind(params.runtime_agent)
        .bind(params.model)
        .bind(params.default_reasoning_effort)
        .bind(params.system_prompt)
        .bind(params.workspace_path)
        .bind(params.now_ms)
        .bind(params.agent_id)
        .bind(params.owner_account_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("social::update_agent"))?;

    if rows_affected == 0 {
        return Ok(None);
    }
    get_agent(store, params.agent_id).await
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
                "SELECT a.agent_id, a.owner_account_id, a.name, a.display_name, a.description, a.avatar_url, a.source, a.status, a.runtime_agent, a.model, a.default_reasoning_effort, a.system_prompt, a.workspace_path, a.created_at_ms, a.updated_at_ms
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
                "SELECT a.agent_id, a.owner_account_id, a.name, a.display_name, a.description, a.avatar_url, a.source, a.status, a.runtime_agent, a.model, a.default_reasoning_effort, a.system_prompt, a.workspace_path, a.created_at_ms, a.updated_at_ms
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

/// Conversation bot members eligible for @ resolution and mailbox delivery.
///
/// Membership may still list disabled bots; delivery and mention SSOT only
/// include `status = active`.
pub async fn list_conversation_agents_active(
    store: &impl AsStorePool,
    conversation_id: &str,
) -> Result<Vec<AgentRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT a.agent_id, a.owner_account_id, a.name, a.display_name, a.description, a.avatar_url, a.source, a.status, a.runtime_agent, a.model, a.default_reasoning_effort, a.system_prompt, a.workspace_path, a.created_at_ms, a.updated_at_ms
                   FROM agents a
                   JOIN conversation_agent_members cam ON cam.agent_id = a.agent_id
                  WHERE cam.conversation_id = ?
                    AND a.status = ?
                  ORDER BY cam.joined_at_ms ASC"
            ))
            .bind(conversation_id)
            .bind(AGENT_STATUS_ACTIVE)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT a.agent_id, a.owner_account_id, a.name, a.display_name, a.description, a.avatar_url, a.source, a.status, a.runtime_agent, a.model, a.default_reasoning_effort, a.system_prompt, a.workspace_path, a.created_at_ms, a.updated_at_ms
                   FROM agents a
                   JOIN conversation_agent_members cam ON cam.agent_id = a.agent_id
                  WHERE cam.conversation_id = $1
                    AND a.status = $2
                  ORDER BY cam.joined_at_ms ASC"
            ))
            .bind(conversation_id)
            .bind(AGENT_STATUS_ACTIVE)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_conversation_agents_active"))
}

/// Another active bot under the same owner with the same name (case-insensitive).
///
/// Used before register/update so clients get a clean conflict instead of a
/// unique-index store error. `exclude_agent_id` skips self on rename.
pub async fn find_active_agent_name_conflict(
    store: &impl AsStorePool,
    owner_account_id: &str,
    name: &str,
    exclude_agent_id: Option<&str>,
) -> Result<Option<AgentRow>, BackendError> {
    let name_lower = name.trim().to_ascii_lowercase();
    if name_lower.is_empty() {
        return Ok(None);
    }
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let row = sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS}
                   FROM agents
                  WHERE owner_account_id = ?
                    AND status = ?
                    AND lower(name) = ?
                  LIMIT 1"
            ))
            .bind(owner_account_id)
            .bind(AGENT_STATUS_ACTIVE)
            .bind(&name_lower)
            .fetch_optional(pool)
            .await
            .map_err(store_err("social::find_active_agent_name_conflict"))?;
            Ok(row.filter(|r| exclude_agent_id.is_none_or(|id| r.agent_id != id)))
        }
        StorePoolRef::Postgres(pool) => {
            let row = sqlx::query_as::<_, AgentRow>(&format!(
                "SELECT {AGENT_SELECT_COLS}
                   FROM agents
                  WHERE owner_account_id = $1
                    AND status = $2
                    AND lower(name) = $3
                  LIMIT 1"
            ))
            .bind(owner_account_id)
            .bind(AGENT_STATUS_ACTIVE)
            .bind(&name_lower)
            .fetch_optional(pool)
            .await
            .map_err(store_err("social::find_active_agent_name_conflict"))?;
            Ok(row.filter(|r| exclude_agent_id.is_none_or(|id| r.agent_id != id)))
        }
    }
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
        &[],
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
    mentioned_agent_ids: &[String],
    client_message_id: Option<&str>,
) -> Result<ChatMessageRow, BackendError> {
    let agent = get_agent(store, agent_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::insert_agent_message.load_agent".into(),
            message: format!("agent not found: {agent_id}"),
        })?;
    let mut tx = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => pool
            .begin()
            .await
            .map(crate::app::tx::DbTx::Sqlite)
            .map_err(store_err("social::insert_agent_message.begin"))?,
        StorePoolRef::Postgres(pool) => pool
            .begin()
            .await
            .map(crate::app::tx::DbTx::Postgres)
            .map_err(store_err("social::insert_agent_message.begin"))?,
    };
    let outcome = insert_agent_message_with_session_in_tx(
        &mut tx,
        &agent,
        conversation_id,
        text,
        now_ms,
        reply_to_message_id,
        agent_session_id,
        mentioned_account_ids,
        mentioned_agent_ids,
        client_message_id,
    )
    .await?;
    tx.commit().await?;
    Ok(outcome.row)
}

/// Insert an agent message on an open transaction (for Transactional Outbox).
///
/// Caller must load `agent` before opening the tx (avoid nested pool checkout).
#[allow(clippy::too_many_arguments)]
pub async fn insert_agent_message_with_session_in_tx(
    tx: &mut crate::app::tx::DbTx<'_>,
    agent: &AgentRow,
    conversation_id: &str,
    text: &str,
    now_ms: i64,
    reply_to_message_id: Option<&str>,
    agent_session_id: Option<&str>,
    mentioned_account_ids: &[String],
    mentioned_agent_ids: &[String],
    client_message_id: Option<&str>,
) -> Result<super::conversation_messages::InsertMessageOutcome, BackendError> {
    let message_id = match client_message_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => {
            if let Some(existing) = super::conversation_messages::get_message_in_tx(tx, id).await? {
                if existing.conversation_id != conversation_id {
                    return Err(BackendError::StoreQuery {
                        operation: "social::insert_agent_message.conflict".into(),
                        message: format!(
                            "message_id {id} already exists in a different conversation"
                        ),
                    });
                }
                // Agent bubble fingerprint: same text/reply/agent (not silent body swap).
                if existing.sender_type != "agent"
                    || existing.sender_agent_id.as_deref() != Some(agent.agent_id.as_str())
                {
                    return Err(BackendError::StoreQuery {
                        operation: "social::insert_agent_message.idempotency_conflict".into(),
                        message: format!(
                            "client_message_id {id} already used by a non-matching agent sender"
                        ),
                    });
                }
                if existing.text != text {
                    return Err(BackendError::StoreQuery {
                        operation: "social::insert_agent_message.idempotency_conflict".into(),
                        message: format!(
                            "client_message_id {id} reused with different message body"
                        ),
                    });
                }
                if existing.reply_to_message_id.as_deref() != reply_to_message_id {
                    return Err(BackendError::StoreQuery {
                        operation: "social::insert_agent_message.idempotency_conflict".into(),
                        message: format!(
                            "client_message_id {id} reused with different reply target"
                        ),
                    });
                }
                return Ok(super::conversation_messages::InsertMessageOutcome {
                    row: existing,
                    inserted: false,
                });
            }
            id.to_string()
        }
        None => Uuid::new_v4().to_string(),
    };

    let mentions = super::MessageMentions {
        account_ids: mentioned_account_ids.to_vec(),
        agent_ids: mentioned_agent_ids.to_vec(),
    };

    let message_seq =
        super::conversation_messages::allocate_message_seq_in_tx(tx, conversation_id, now_ms)
            .await?;

    match tx {
        crate::app::tx::DbTx::Sqlite(tx) => {
            sqlx::query(
                "INSERT INTO chat_messages (
                    message_id,
                    conversation_id,
                    sender_account_id,
                    sender_agent_id,
                    text,
                    created_at_ms,
                    message_seq,
                    reply_to_message_id,
                    agent_session_id,
                    sender_type,
                    message_source
                 ) VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, 'agent', 'host_projection')",
            )
            .bind(&message_id)
            .bind(conversation_id)
            .bind(&agent.agent_id)
            .bind(text)
            .bind(now_ms)
            .bind(message_seq)
            .bind(reply_to_message_id)
            .bind(agent_session_id)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::insert_agent_message.insert"))?;

            // Preserve appearance order across account + agent mentions by ordinal.
            let mut ordinal: i64 = 0;
            for target_id in &mentions.account_ids {
                sqlx::query(
                    "INSERT OR IGNORE INTO chat_message_mentions
                        (message_id, target_kind, target_id, ordinal)
                     VALUES (?, 'account', ?, ?)",
                )
                .bind(&message_id)
                .bind(target_id)
                .bind(ordinal)
                .execute(&mut **tx)
                .await
                .map_err(store_err("social::insert_agent_message.mention"))?;
                ordinal += 1;
            }
            for target_id in &mentions.agent_ids {
                sqlx::query(
                    "INSERT OR IGNORE INTO chat_message_mentions
                        (message_id, target_kind, target_id, ordinal)
                     VALUES (?, 'agent', ?, ?)",
                )
                .bind(&message_id)
                .bind(target_id)
                .bind(ordinal)
                .execute(&mut **tx)
                .await
                .map_err(store_err("social::insert_agent_message.mention"))?;
                ordinal += 1;
            }
        }
        crate::app::tx::DbTx::Postgres(tx) => {
            sqlx::query(
                "INSERT INTO chat_messages (
                    message_id,
                    conversation_id,
                    sender_account_id,
                    sender_agent_id,
                    text,
                    created_at_ms,
                    message_seq,
                    reply_to_message_id,
                    agent_session_id,
                    sender_type,
                    message_source
                 ) VALUES ($1, $2, NULL, $3, $4, $5, $6, $7, $8, 'agent', 'host_projection')",
            )
            .bind(&message_id)
            .bind(conversation_id)
            .bind(&agent.agent_id)
            .bind(text)
            .bind(now_ms)
            .bind(message_seq)
            .bind(reply_to_message_id)
            .bind(agent_session_id)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::insert_agent_message.insert"))?;

            let mut ordinal: i64 = 0;
            for target_id in &mentions.account_ids {
                sqlx::query(
                    "INSERT INTO chat_message_mentions
                        (message_id, target_kind, target_id, ordinal)
                     VALUES ($1, 'account', $2, $3)
                     ON CONFLICT DO NOTHING",
                )
                .bind(&message_id)
                .bind(target_id)
                .bind(ordinal)
                .execute(&mut **tx)
                .await
                .map_err(store_err("social::insert_agent_message.mention"))?;
                ordinal += 1;
            }
            for target_id in &mentions.agent_ids {
                sqlx::query(
                    "INSERT INTO chat_message_mentions
                        (message_id, target_kind, target_id, ordinal)
                     VALUES ($1, 'agent', $2, $3)
                     ON CONFLICT DO NOTHING",
                )
                .bind(&message_id)
                .bind(target_id)
                .bind(ordinal)
                .execute(&mut **tx)
                .await
                .map_err(store_err("social::insert_agent_message.mention"))?;
                ordinal += 1;
            }
        }
    }

    Ok(super::conversation_messages::InsertMessageOutcome {
        row: ChatMessageRow {
            message_id,
            conversation_id: conversation_id.to_string(),
            // Bot author is sender_agent_id; owner is not the wire/sender principal.
            sender_account_id: None,
            sender_agent_id: Some(agent.agent_id.clone()),
            text: text.to_string(),
            created_at_ms: now_ms,
            message_seq,
            reply_to_message_id: reply_to_message_id.map(ToOwned::to_owned),
            recalled_at_ms: None,
            sender_type: "agent".to_string(),
            message_source: "host_projection".to_string(),
        },
        inserted: true,
    })
}

/// Persist an immutable digital-body snapshot used at mailbox schedule time.
pub async fn insert_bot_revision(
    store: &impl AsStorePool,
    agent: &AgentRow,
    now_ms: i64,
) -> Result<String, BackendError> {
    let revision_id = format!("brev-{}", Uuid::new_v4());
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO bot_revisions (
                    revision_id, agent_id, runtime_agent, model, default_reasoning_effort,
                    system_prompt, display_name, workspace_path, created_at_ms
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&revision_id)
            .bind(&agent.agent_id)
            .bind(&agent.runtime_agent)
            .bind(&agent.model)
            .bind(&agent.default_reasoning_effort)
            .bind(&agent.system_prompt)
            .bind(&agent.display_name)
            .bind(&agent.workspace_path)
            .bind(now_ms)
            .execute(pool)
            .await
            .map_err(store_err("social::insert_bot_revision"))?;
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO bot_revisions (
                    revision_id, agent_id, runtime_agent, model, default_reasoning_effort,
                    system_prompt, display_name, workspace_path, created_at_ms
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(&revision_id)
            .bind(&agent.agent_id)
            .bind(&agent.runtime_agent)
            .bind(&agent.model)
            .bind(&agent.default_reasoning_effort)
            .bind(&agent.system_prompt)
            .bind(&agent.display_name)
            .bind(&agent.workspace_path)
            .bind(now_ms)
            .execute(pool)
            .await
            .map_err(store_err("social::insert_bot_revision"))?;
        }
    }
    Ok(revision_id)
}

/// Upsert active deployment of a bot onto a host installation.
pub async fn upsert_bot_deployment(
    store: &impl AsStorePool,
    agent_id: &str,
    host_installation_id: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO bot_deployments (agent_id, host_installation_id, status, updated_at_ms)
                 VALUES (?, ?, 'active', ?)
                 ON CONFLICT(agent_id, host_installation_id) DO UPDATE SET
                   status = 'active', updated_at_ms = excluded.updated_at_ms",
            )
            .bind(agent_id)
            .bind(host_installation_id)
            .bind(now_ms)
            .execute(pool)
            .await
            .map_err(store_err("social::upsert_bot_deployment"))?;
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO bot_deployments (agent_id, host_installation_id, status, updated_at_ms)
                 VALUES ($1, $2, 'active', $3)
                 ON CONFLICT(agent_id, host_installation_id) DO UPDATE SET
                   status = 'active', updated_at_ms = EXCLUDED.updated_at_ms",
            )
            .bind(agent_id)
            .bind(host_installation_id)
            .bind(now_ms)
            .execute(pool)
            .await
            .map_err(store_err("social::upsert_bot_deployment"))?;
        }
    }
    Ok(())
}
