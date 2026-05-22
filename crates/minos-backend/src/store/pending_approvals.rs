use minos_domain::DeviceId;
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, PartialEq)]
pub struct PendingApprovalRow {
    pub request_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub host_device_id: DeviceId,
    pub method: String,
    pub params_json: Value,
    pub created_at_ms: i64,
    pub timeout_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
    pub resolution: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    store: &impl AsStorePool,
    request_id: &str,
    thread_id: &str,
    turn_id: &str,
    host_device_id: DeviceId,
    method: &str,
    params_json: &Value,
    created_at_ms: i64,
    timeout_at_ms: i64,
) -> Result<(), BackendError> {
    let params_json =
        serde_json::to_string(params_json).map_err(|error| BackendError::StoreQuery {
            operation: "pending_approvals::insert.serialize".into(),
            message: error.to_string(),
        })?;

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT OR IGNORE INTO pending_approvals
                    (request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(request_id)
            .bind(thread_id)
            .bind(turn_id)
            .bind(host_device_id.to_string())
            .bind(method)
            .bind(params_json.as_str())
            .bind(created_at_ms)
            .bind(timeout_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO pending_approvals
                    (request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (request_id) DO NOTHING",
            )
            .bind(request_id)
            .bind(thread_id)
            .bind(turn_id)
            .bind(host_device_id.to_string())
            .bind(method)
            .bind(params_json.as_str())
            .bind(created_at_ms)
            .bind(timeout_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("pending_approvals::insert"))?;
    Ok(())
}

pub async fn get(
    store: &impl AsStorePool,
    request_id: &str,
) -> Result<Option<PendingApprovalRow>, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64, i64, Option<i64>, Option<String>)>(
                "SELECT request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms, resolved_at_ms, resolution
                   FROM pending_approvals
                  WHERE request_id = ?",
            )
            .bind(request_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64, i64, Option<i64>, Option<String>)>(
                "SELECT request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms, resolved_at_ms, resolution
                   FROM pending_approvals
                  WHERE request_id = $1",
            )
            .bind(request_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("pending_approvals::get"))?;

    row.map(decode_row).transpose()
}

pub async fn resolve(
    store: &impl AsStorePool,
    request_id: &str,
    resolution: &str,
    resolved_at_ms: i64,
) -> Result<bool, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE pending_approvals
                    SET resolved_at_ms = ?, resolution = ?
                  WHERE request_id = ?
                    AND resolved_at_ms IS NULL",
        )
        .bind(resolved_at_ms)
        .bind(resolution)
        .bind(request_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE pending_approvals
                    SET resolved_at_ms = $1, resolution = $2
                  WHERE request_id = $3
                    AND resolved_at_ms IS NULL",
        )
        .bind(resolved_at_ms)
        .bind(resolution)
        .bind(request_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("pending_approvals::resolve"))?;
    Ok(result == 1)
}

pub async fn list_expired_unresolved(
    store: &impl AsStorePool,
    now_ms: i64,
    limit: u32,
) -> Result<Vec<PendingApprovalRow>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64, i64, Option<i64>, Option<String>)>(
                "SELECT request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms, resolved_at_ms, resolution
                   FROM pending_approvals
                  WHERE resolved_at_ms IS NULL
                    AND timeout_at_ms <= ?
                  ORDER BY timeout_at_ms ASC
                  LIMIT ?",
            )
            .bind(now_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, (String, String, String, String, String, String, i64, i64, Option<i64>, Option<String>)>(
                "SELECT request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms, resolved_at_ms, resolution
                   FROM pending_approvals
                  WHERE resolved_at_ms IS NULL
                    AND timeout_at_ms <= $1
                  ORDER BY timeout_at_ms ASC
                  LIMIT $2",
            )
            .bind(now_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("pending_approvals::list_expired_unresolved"))?;

    rows.into_iter().map(decode_row).collect()
}

pub async fn next_unresolved_timeout_at_ms(
    store: &impl AsStorePool,
) -> Result<Option<i64>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar(
                "SELECT MIN(timeout_at_ms)
                   FROM pending_approvals
                  WHERE resolved_at_ms IS NULL",
            )
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar(
                "SELECT MIN(timeout_at_ms)
                   FROM pending_approvals
                  WHERE resolved_at_ms IS NULL",
            )
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err(
        "pending_approvals::next_unresolved_timeout_at_ms",
    ))
}

pub async fn list_unresolved_for_hosts(
    store: &impl AsStorePool,
    host_device_ids: &[DeviceId],
) -> Result<Vec<PendingApprovalRow>, BackendError> {
    if host_device_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms, resolved_at_ms, resolution
                   FROM pending_approvals
                  WHERE resolved_at_ms IS NULL
                    AND host_device_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for host_device_id in host_device_ids {
                    separated.push_bind(host_device_id.to_string());
                }
            }
            builder.push(')');

            builder
                .build_query_as::<(
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    i64,
                    i64,
                    Option<i64>,
                    Option<String>,
                )>()
                .fetch_all(pool)
                .await
        }
        StorePoolRef::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(
                "SELECT request_id, thread_id, turn_id, host_device_id, method, params_json, created_at_ms, timeout_at_ms, resolved_at_ms, resolution
                   FROM pending_approvals
                  WHERE resolved_at_ms IS NULL
                    AND host_device_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for host_device_id in host_device_ids {
                    separated.push_bind(host_device_id.to_string());
                }
            }
            builder.push(')');

            builder
                .build_query_as::<(
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    i64,
                    i64,
                    Option<i64>,
                    Option<String>,
                )>()
                .fetch_all(pool)
                .await
        }
    }
    .map_err(store_err("pending_approvals::list_unresolved_for_hosts"))?;

    rows.into_iter().map(decode_row).collect()
}

#[allow(clippy::type_complexity)]
fn decode_row(
    row: (
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
        Option<i64>,
        Option<String>,
    ),
) -> Result<PendingApprovalRow, BackendError> {
    let (
        request_id,
        thread_id,
        turn_id,
        host_device_id,
        method,
        params_json,
        created_at_ms,
        timeout_at_ms,
        resolved_at_ms,
        resolution,
    ) = row;
    Ok(PendingApprovalRow {
        request_id,
        thread_id,
        turn_id,
        host_device_id: Uuid::parse_str(&host_device_id)
            .map(DeviceId)
            .map_err(|error| BackendError::StoreDecode {
                column: "pending_approvals.host_device_id".into(),
                message: error.to_string(),
            })?,
        method,
        params_json: serde_json::from_str(&params_json).map_err(|error| {
            BackendError::StoreDecode {
                column: "pending_approvals.params_json".into(),
                message: error.to_string(),
            }
        })?,
        created_at_ms,
        timeout_at_ms,
        resolved_at_ms,
        resolution,
    })
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
    use crate::store::devices::insert_device;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::DeviceRole;

    #[tokio::test]
    async fn insert_and_resolve_round_trip() {
        let pool = memory_pool().await;
        let host = DeviceId::new();
        insert_device(&pool, host, "mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();

        insert(
            &pool,
            "req-1",
            "thr-1",
            "turn-1",
            host,
            "item/commandExecution/requestApproval",
            &serde_json::json!({ "threadId": "thr-1" }),
            T0,
            T0 + 100,
        )
        .await
        .unwrap();

        let row = get(&pool, "req-1").await.unwrap().unwrap();
        assert_eq!(row.host_device_id, host);
        assert_eq!(row.resolution, None);

        assert!(resolve(&pool, "req-1", "user_decision", T0 + 1)
            .await
            .unwrap());

        let row = get(&pool, "req-1").await.unwrap().unwrap();
        assert_eq!(row.resolution.as_deref(), Some("user_decision"));
        assert_eq!(row.resolved_at_ms, Some(T0 + 1));
    }

    #[tokio::test]
    async fn next_unresolved_timeout_at_ms_returns_earliest_open_deadline() {
        let pool = memory_pool().await;
        let host = DeviceId::new();
        insert_device(&pool, host, "mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();

        insert(
            &pool,
            "req-late",
            "thr-1",
            "turn-1",
            host,
            "applyPatchApproval",
            &serde_json::json!({}),
            T0,
            T0 + 5_000,
        )
        .await
        .unwrap();
        insert(
            &pool,
            "req-early",
            "thr-1",
            "turn-2",
            host,
            "applyPatchApproval",
            &serde_json::json!({}),
            T0,
            T0 + 500,
        )
        .await
        .unwrap();
        assert!(resolve(&pool, "req-late", "timeout", T0 + 1).await.unwrap());

        assert_eq!(
            next_unresolved_timeout_at_ms(&pool).await.unwrap(),
            Some(T0 + 500)
        );
    }
}
