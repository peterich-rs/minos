use minos_domain::DeviceId;
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequestState {
    Pending,
    Decided,
    Timeout,
    Disconnected,
}

impl ApprovalRequestState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Decided => "decided",
            Self::Timeout => "timeout",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequestRow {
    pub request_id: String,
    pub agent_session_id: String,
    pub turn_id: Option<String>,
    pub host_device_id: DeviceId,
    pub method: String,
    pub params_json: Value,
    pub state: ApprovalRequestState,
    pub deadline_at_ms: i64,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
    pub resolution_json: Option<Value>,
    /// Client Intent Outbox id when respond carried `client_request_id` (C5.3).
    pub client_request_id: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_pending(
    store: &impl AsStorePool,
    request_id: &str,
    agent_session_id: &str,
    turn_id: Option<&str>,
    method: &str,
    params_json: &Value,
    created_at_ms: i64,
    deadline_at_ms: i64,
) -> Result<(), BackendError> {
    let params_json =
        serde_json::to_string(params_json).map_err(|error| BackendError::StoreQuery {
            operation: "approval_requests::insert_pending.serialize".into(),
            message: error.to_string(),
        })?;
    let turn_id = normalize_optional_text(turn_id);

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT OR IGNORE INTO approval_requests
                    (request_id, agent_session_id, turn_id, method, params_json, state, deadline_at_ms, created_at_ms, resolved_at_ms, resolution_json)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL)",
            )
            .bind(request_id)
            .bind(agent_session_id)
            .bind(turn_id)
            .bind(method)
            .bind(params_json.as_str())
            .bind(ApprovalRequestState::Pending.as_str())
            .bind(deadline_at_ms)
            .bind(created_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO approval_requests
                    (request_id, agent_session_id, turn_id, method, params_json, state::text, deadline_at_ms, created_at_ms, resolved_at_ms, resolution_json)
                 VALUES ($1, $2, $3, $4, $5::jsonb, $6::approval_state, $7, $8, NULL, NULL)
                 ON CONFLICT (request_id) DO NOTHING",
            )
            .bind(request_id)
            .bind(agent_session_id)
            .bind(turn_id)
            .bind(method)
            .bind(params_json.as_str())
            .bind(ApprovalRequestState::Pending.as_str())
            .bind(deadline_at_ms)
            .bind(created_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("approval_requests::insert_pending"))?;
    Ok(())
}

pub async fn get(
    store: &impl AsStorePool,
    request_id: &str,
) -> Result<Option<ApprovalRequestRow>, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_as::<_, ApprovalRequestDbRow>(
            "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, COALESCE(s.host_installation_id, t.owner_device_id), ar.method,
                        ar.params_json, ar.state, ar.deadline_at_ms, ar.created_at_ms,
                        ar.resolved_at_ms, ar.resolution_json, ar.client_request_id
                   FROM approval_requests ar
                   LEFT JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                   LEFT JOIN sessions t
                     ON t.session_id = ar.agent_session_id
                  WHERE ar.request_id = ?",
        )
        .bind(request_id)
        .fetch_optional(pool)
        .await,
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ApprovalRequestDbRow>(
                "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, s.host_installation_id,
                        ar.method, ar.params_json::text, ar.state::text, ar.deadline_at_ms,
                        ar.created_at_ms, ar.resolved_at_ms, ar.resolution_json::text,
                        ar.client_request_id
                   FROM approval_requests ar
                   JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                  WHERE ar.request_id = $1",
            )
            .bind(request_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("approval_requests::get"))?;

    row.map(decode_row).transpose()
}

/// Lookup by client Intent Outbox id (C5.3 respond idempotency).
pub async fn get_by_client_request_id(
    store: &impl AsStorePool,
    client_request_id: &str,
) -> Result<Option<ApprovalRequestRow>, BackendError> {
    let client_request_id = client_request_id.trim();
    if client_request_id.is_empty() {
        return Ok(None);
    }
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_as::<_, ApprovalRequestDbRow>(
            "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, COALESCE(s.host_installation_id, t.owner_device_id), ar.method,
                        ar.params_json, ar.state, ar.deadline_at_ms, ar.created_at_ms,
                        ar.resolved_at_ms, ar.resolution_json, ar.client_request_id
                   FROM approval_requests ar
                   LEFT JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                   LEFT JOIN sessions t
                     ON t.session_id = ar.agent_session_id
                  WHERE ar.client_request_id = ?",
        )
        .bind(client_request_id)
        .fetch_optional(pool)
        .await,
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ApprovalRequestDbRow>(
                "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, s.host_installation_id,
                        ar.method, ar.params_json::text, ar.state::text, ar.deadline_at_ms,
                        ar.created_at_ms, ar.resolved_at_ms, ar.resolution_json::text,
                        ar.client_request_id
                   FROM approval_requests ar
                   JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                  WHERE ar.client_request_id = $1",
            )
            .bind(client_request_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("approval_requests::get_by_client_request_id"))?;

    row.map(decode_row).transpose()
}

pub async fn resolve(
    store: &impl AsStorePool,
    request_id: &str,
    state: ApprovalRequestState,
    resolved_at_ms: i64,
    resolution_json: Option<&Value>,
) -> Result<bool, BackendError> {
    resolve_with_client_request_id(
        store,
        request_id,
        state,
        resolved_at_ms,
        resolution_json,
        None,
    )
    .await
}

/// Resolve a pending approval, optionally stamping `client_request_id` for
/// respond idempotency (C5.3).
pub async fn resolve_with_client_request_id(
    store: &impl AsStorePool,
    request_id: &str,
    state: ApprovalRequestState,
    resolved_at_ms: i64,
    resolution_json: Option<&Value>,
    client_request_id: Option<&str>,
) -> Result<bool, BackendError> {
    let resolution_json = resolution_json
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| BackendError::StoreQuery {
            operation: "approval_requests::resolve.serialize".into(),
            message: error.to_string(),
        })?;
    let client_request_id = client_request_id.map(str::trim).filter(|s| !s.is_empty());

    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE approval_requests
                    SET state = ?, resolved_at_ms = ?, resolution_json = ?,
                        client_request_id = COALESCE(?, client_request_id)
                  WHERE request_id = ?
                    AND state = ?",
        )
        .bind(state.as_str())
        .bind(resolved_at_ms)
        .bind(resolution_json.as_deref())
        .bind(client_request_id)
        .bind(request_id)
        .bind(ApprovalRequestState::Pending.as_str())
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE approval_requests
                    SET state = $1::approval_state, resolved_at_ms = $2, resolution_json = $3::jsonb,
                        client_request_id = COALESCE($4, client_request_id)
                  WHERE request_id = $5
                    AND state = 'pending'::approval_state",
        )
        .bind(state.as_str())
        .bind(resolved_at_ms)
        .bind(resolution_json.as_deref())
        .bind(client_request_id)
        .bind(request_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("approval_requests::resolve"))?;

    Ok(rows_affected == 1)
}

pub async fn list_expired_pending(
    store: &impl AsStorePool,
    now_ms: i64,
    limit: u32,
) -> Result<Vec<ApprovalRequestRow>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_as::<_, ApprovalRequestDbRow>(
            "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, COALESCE(s.host_installation_id, t.owner_device_id), ar.method,
                        ar.params_json, ar.state, ar.deadline_at_ms, ar.created_at_ms,
                        ar.resolved_at_ms, ar.resolution_json, ar.client_request_id
                   FROM approval_requests ar
                   LEFT JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                   LEFT JOIN sessions t
                     ON t.session_id = ar.agent_session_id
                  WHERE ar.state = ?
                    AND ar.deadline_at_ms <= ?
                  ORDER BY ar.deadline_at_ms ASC
                  LIMIT ?",
        )
        .bind(ApprovalRequestState::Pending.as_str())
        .bind(now_ms)
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await,
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ApprovalRequestDbRow>(
                "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, s.host_installation_id,
                        ar.method, ar.params_json::text, ar.state::text, ar.deadline_at_ms,
                        ar.created_at_ms, ar.resolved_at_ms, ar.resolution_json::text,
                        ar.client_request_id
                   FROM approval_requests ar
                   JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                  WHERE ar.state = 'pending'::approval_state
                    AND ar.deadline_at_ms <= $1
                  ORDER BY ar.deadline_at_ms ASC
                  LIMIT $2",
            )
            .bind(now_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("approval_requests::list_expired_pending"))?;

    rows.into_iter().map(decode_row).collect()
}

pub async fn next_pending_deadline_at_ms(
    store: &impl AsStorePool,
) -> Result<Option<i64>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar(
                "SELECT MIN(deadline_at_ms)
                   FROM approval_requests
                  WHERE state = ?",
            )
            .bind(ApprovalRequestState::Pending.as_str())
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar(
                "SELECT MIN(deadline_at_ms)
                   FROM approval_requests
                  WHERE state = 'pending'::approval_state",
            )
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("approval_requests::next_pending_deadline_at_ms"))
}

pub async fn list_pending_for_hosts(
    store: &impl AsStorePool,
    host_device_ids: &[DeviceId],
) -> Result<Vec<ApprovalRequestRow>, BackendError> {
    if host_device_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, COALESCE(s.host_installation_id, t.owner_device_id), ar.method,
                        ar.params_json, ar.state, ar.deadline_at_ms, ar.created_at_ms,
                        ar.resolved_at_ms, ar.resolution_json, ar.client_request_id
                   FROM approval_requests ar
                   LEFT JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                   LEFT JOIN sessions t
                     ON t.session_id = ar.agent_session_id
                  WHERE ar.state = ",
            );
            builder.push_bind(ApprovalRequestState::Pending.as_str());
            builder.push(" AND COALESCE(s.host_installation_id, t.owner_device_id) IN (");
            {
                let mut separated = builder.separated(", ");
                for host_device_id in host_device_ids {
                    separated.push_bind(host_device_id.to_string());
                }
            }
            builder.push(')');

            builder
                .build_query_as::<ApprovalRequestDbRow>()
                .fetch_all(pool)
                .await
        }
        StorePoolRef::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(
                "SELECT ar.request_id, ar.agent_session_id, ar.turn_id, s.host_installation_id,
                        ar.method, ar.params_json::text, ar.state::text, ar.deadline_at_ms,
                        ar.created_at_ms, ar.resolved_at_ms, ar.resolution_json::text,
                        ar.client_request_id
                   FROM approval_requests ar
                   JOIN agent_sessions s
                     ON s.session_id = ar.agent_session_id
                  WHERE ar.state = 'pending'::approval_state
                    AND s.host_installation_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for host_device_id in host_device_ids {
                    separated.push_bind(host_device_id.to_string());
                }
            }
            builder.push(')');

            builder
                .build_query_as::<ApprovalRequestDbRow>()
                .fetch_all(pool)
                .await
        }
    }
    .map_err(store_err("approval_requests::list_pending_for_hosts"))?;

    rows.into_iter().map(decode_row).collect()
}

type ApprovalRequestDbRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
);

fn decode_row(row: ApprovalRequestDbRow) -> Result<ApprovalRequestRow, BackendError> {
    let (
        request_id,
        agent_session_id,
        turn_id,
        host_device_id,
        method,
        params_json,
        state,
        deadline_at_ms,
        created_at_ms,
        resolved_at_ms,
        resolution_json,
        client_request_id,
    ) = row;

    let host_device_id = host_device_id.ok_or_else(|| BackendError::StoreDecode {
        column: "approval_requests.host_installation_id".into(),
        message: "NULL host installation on approval session".into(),
    })?;

    Ok(ApprovalRequestRow {
        request_id,
        agent_session_id,
        turn_id,
        host_device_id: Uuid::parse_str(&host_device_id)
            .map(DeviceId)
            .map_err(|error| BackendError::StoreDecode {
                column: "approval_requests.host_installation_id".into(),
                message: error.to_string(),
            })?,
        method,
        params_json: serde_json::from_str(&params_json).map_err(|error| {
            BackendError::StoreDecode {
                column: "approval_requests.params_json".into(),
                message: error.to_string(),
            }
        })?,
        state: decode_state(&state)?,
        deadline_at_ms,
        created_at_ms,
        resolved_at_ms,
        resolution_json: resolution_json
            .map(|json| {
                serde_json::from_str(&json).map_err(|error| BackendError::StoreDecode {
                    column: "approval_requests.resolution_json".into(),
                    message: error.to_string(),
                })
            })
            .transpose()?,
        client_request_id,
    })
}

fn decode_state(state: &str) -> Result<ApprovalRequestState, BackendError> {
    match state {
        "pending" => Ok(ApprovalRequestState::Pending),
        "decided" => Ok(ApprovalRequestState::Decided),
        "timeout" => Ok(ApprovalRequestState::Timeout),
        "disconnected" => Ok(ApprovalRequestState::Disconnected),
        _ => Err(BackendError::StoreDecode {
            column: "approval_requests.state".into(),
            message: format!("unknown approval state: {state}"),
        }),
    }
}

fn normalize_optional_text(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

fn store_err(operation: &'static str) -> impl FnOnce(sqlx::Error) -> BackendError {
    move |error| BackendError::StoreQuery {
        operation: operation.into(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, memory_pool, T0};
    use serde_json::json;

    async fn seed_pending(pool: &sqlx::SqlitePool, request_id: &str, session_id: &str) {
        let account = insert_account(pool, &format!("approval-{request_id}@example.com")).await;
        let conv = crate::store::social::create_group_conversation(
            pool,
            &account,
            &format!("approval-{request_id}"),
            &[],
            T0,
        )
        .await
        .unwrap();
        let host = crate::store::test_support::insert_ios_device(pool, &account).await;
        sqlx::query(
            "INSERT INTO agent_sessions
                (session_id, conversation_id, project_id, host_installation_id, agent_id, status, started_at_ms, ended_at_ms)
             VALUES (?, ?, NULL, ?, NULL, 'running', ?, NULL)",
        )
        .bind(session_id)
        .bind(&conv.conversation_id)
        .bind(host.to_string())
        .bind(T0)
        .execute(pool)
        .await
        .unwrap();
        insert_pending(
            pool,
            request_id,
            session_id,
            None,
            "approval/request",
            &json!({ "tool": "shell" }),
            T0,
            T0 + 60_000,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn resolve_stamps_client_request_id_and_lookup_is_idempotent() {
        let pool = memory_pool().await;
        seed_pending(&pool, "req-idem-1", "sess-idem-1").await;

        let decision = json!({ "decision": "accept" });
        let first = resolve_with_client_request_id(
            &pool,
            "req-idem-1",
            ApprovalRequestState::Decided,
            T0 + 1,
            Some(&decision),
            Some("client-op-same"),
        )
        .await
        .unwrap();
        assert!(first);

        let by_cid = get_by_client_request_id(&pool, "client-op-same")
            .await
            .unwrap()
            .expect("client_request_id stamped");
        assert_eq!(by_cid.request_id, "req-idem-1");
        assert_eq!(by_cid.state, ApprovalRequestState::Decided);
        assert_eq!(by_cid.client_request_id.as_deref(), Some("client-op-same"));

        // Second resolve on already-decided is a no-op (0 rows).
        let second = resolve_with_client_request_id(
            &pool,
            "req-idem-1",
            ApprovalRequestState::Decided,
            T0 + 2,
            Some(&decision),
            Some("client-op-same"),
        )
        .await
        .unwrap();
        assert!(!second);

        // Lookup still returns the same logical result (idempotent read).
        let again = get_by_client_request_id(&pool, "client-op-same")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again.request_id, "req-idem-1");
        assert_eq!(again.resolved_at_ms, Some(T0 + 1));
    }

    #[tokio::test]
    async fn different_requests_cannot_reuse_same_client_request_id() {
        let pool = memory_pool().await;
        seed_pending(&pool, "req-a", "sess-a").await;
        seed_pending(&pool, "req-b", "sess-b").await;

        let ok = resolve_with_client_request_id(
            &pool,
            "req-a",
            ApprovalRequestState::Decided,
            T0 + 1,
            Some(&json!({"decision":"accept"})),
            Some("shared-op"),
        )
        .await
        .unwrap();
        assert!(ok);

        // Unique index on client_request_id: second stamp of same id fails.
        let err = resolve_with_client_request_id(
            &pool,
            "req-b",
            ApprovalRequestState::Decided,
            T0 + 2,
            Some(&json!({"decision":"accept"})),
            Some("shared-op"),
        )
        .await;
        assert!(err.is_err());
    }
}
