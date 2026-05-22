use minos_domain::DeviceId;
use serde_json::Value;
#[cfg(test)]
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type HostCommandRowTuple = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCommandStatus {
    Pending,
    Acked,
    Succeeded,
    Failed,
}

impl HostCommandStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Acked => "acked",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCommandTerminalStatus {
    Succeeded,
    Failed,
}

impl HostCommandTerminalStatus {
    fn as_status(self) -> HostCommandStatus {
        match self {
            Self::Succeeded => HostCommandStatus::Succeeded,
            Self::Failed => HostCommandStatus::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostCommandRow {
    pub command_id: String,
    pub host_installation_id: DeviceId,
    pub agent_session_id: Option<String>,
    pub method: String,
    pub params_json: Value,
    pub requested_by_account_id: Option<String>,
    pub status: HostCommandStatus,
    pub response_json: Option<Value>,
    pub error_json: Option<Value>,
    pub deadline_at_ms: i64,
    pub created_at_ms: i64,
    pub ack_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
pub async fn enqueue(
    store: &impl AsStorePool,
    command_id: &str,
    host_installation_id: DeviceId,
    agent_session_id: Option<&str>,
    method: &str,
    params_json: &Value,
    requested_by_account_id: Option<&str>,
    deadline_at_ms: i64,
    created_at_ms: i64,
) -> Result<(), BackendError> {
    let params_json = serialize_json(params_json, "host_commands::enqueue.params_json")?;

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO host_commands
                    (command_id, host_installation_id, agent_session_id, method, params_json, requested_by_account_id, status, deadline_at_ms, created_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
            )
            .bind(command_id)
            .bind(host_installation_id.to_string())
            .bind(agent_session_id)
            .bind(method)
            .bind(params_json.as_str())
            .bind(requested_by_account_id)
            .bind(deadline_at_ms)
            .bind(created_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO host_commands
                    (command_id, host_installation_id, agent_session_id, method, params_json, requested_by_account_id, status, deadline_at_ms, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8)",
            )
            .bind(command_id)
            .bind(host_installation_id.to_string())
            .bind(agent_session_id)
            .bind(method)
            .bind(params_json.as_str())
            .bind(requested_by_account_id)
            .bind(deadline_at_ms)
            .bind(created_at_ms)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(store_err("host_commands::enqueue"))?;
    Ok(())
}

pub async fn get(
    store: &impl AsStorePool,
    command_id: &str,
) -> Result<Option<HostCommandRow>, BackendError> {
    let row =
        match store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => sqlx::query_as::<_, HostCommandRowTuple>(
                "SELECT command_id, host_installation_id, agent_session_id, method, params_json,
                        requested_by_account_id, status, response_json, error_json,
                        deadline_at_ms, created_at_ms, ack_at_ms, finished_at_ms
                   FROM host_commands
                  WHERE command_id = ?",
            )
            .bind(command_id)
            .fetch_optional(pool)
            .await,
            StorePoolRef::Postgres(pool) => sqlx::query_as::<_, HostCommandRowTuple>(
                "SELECT command_id, host_installation_id, agent_session_id, method, params_json,
                        requested_by_account_id, status, response_json, error_json,
                        deadline_at_ms, created_at_ms, ack_at_ms, finished_at_ms
                   FROM host_commands
                  WHERE command_id = $1",
            )
            .bind(command_id)
            .fetch_optional(pool)
            .await,
        }
        .map_err(store_err("host_commands::get"))?;

    row.map(decode_row).transpose()
}

pub async fn ack(
    store: &impl AsStorePool,
    command_id: &str,
    ack_at_ms: i64,
) -> Result<bool, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE host_commands
                    SET status = 'acked', ack_at_ms = ?
                  WHERE command_id = ?
                    AND status = 'pending'
                    AND ack_at_ms IS NULL
                    AND finished_at_ms IS NULL",
        )
        .bind(ack_at_ms)
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE host_commands
                    SET status = 'acked', ack_at_ms = $1
                  WHERE command_id = $2
                    AND status = 'pending'
                    AND ack_at_ms IS NULL
                    AND finished_at_ms IS NULL",
        )
        .bind(ack_at_ms)
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("host_commands::ack"))?;
    Ok(result == 1)
}

#[allow(clippy::too_many_arguments)]
pub async fn finish(
    store: &impl AsStorePool,
    command_id: &str,
    status: HostCommandTerminalStatus,
    response_json: Option<&Value>,
    error_json: Option<&Value>,
    finished_at_ms: i64,
) -> Result<bool, BackendError> {
    let response_json = response_json
        .map(|value| serialize_json(value, "host_commands::finish.response_json"))
        .transpose()?;
    let error_json = error_json
        .map(|value| serialize_json(value, "host_commands::finish.error_json"))
        .transpose()?;

    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE host_commands
                    SET status = ?, response_json = ?, error_json = ?, finished_at_ms = ?
                  WHERE command_id = ?
                    AND finished_at_ms IS NULL
                    AND status IN ('pending', 'acked')",
        )
        .bind(status.as_status().as_str())
        .bind(response_json.as_deref())
        .bind(error_json.as_deref())
        .bind(finished_at_ms)
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE host_commands
                    SET status = $1, response_json = $2, error_json = $3, finished_at_ms = $4
                  WHERE command_id = $5
                    AND finished_at_ms IS NULL
                    AND status IN ('pending', 'acked')",
        )
        .bind(status.as_status().as_str())
        .bind(response_json.as_deref())
        .bind(error_json.as_deref())
        .bind(finished_at_ms)
        .bind(command_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("host_commands::finish"))?;
    Ok(result == 1)
}

pub async fn list_timed_out_open(
    store: &impl AsStorePool,
    now_ms: i64,
    limit: u32,
) -> Result<Vec<HostCommandRow>, BackendError> {
    let rows =
        match store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => sqlx::query_as::<_, HostCommandRowTuple>(
                "SELECT command_id, host_installation_id, agent_session_id, method, params_json,
                        requested_by_account_id, status, response_json, error_json,
                        deadline_at_ms, created_at_ms, ack_at_ms, finished_at_ms
                   FROM host_commands
                  WHERE deadline_at_ms <= ?
                    AND finished_at_ms IS NULL
                    AND status IN ('pending', 'acked')
                  ORDER BY deadline_at_ms ASC
                  LIMIT ?",
            )
            .bind(now_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await,
            StorePoolRef::Postgres(pool) => sqlx::query_as::<_, HostCommandRowTuple>(
                "SELECT command_id, host_installation_id, agent_session_id, method, params_json,
                        requested_by_account_id, status, response_json, error_json,
                        deadline_at_ms, created_at_ms, ack_at_ms, finished_at_ms
                   FROM host_commands
                  WHERE deadline_at_ms <= $1
                    AND finished_at_ms IS NULL
                    AND status IN ('pending', 'acked')
                  ORDER BY deadline_at_ms ASC
                  LIMIT $2",
            )
            .bind(now_ms)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await,
        }
        .map_err(store_err("host_commands::list_timed_out_open"))?;

    rows.into_iter().map(decode_row).collect()
}

pub async fn mark_timed_out(
    store: &impl AsStorePool,
    command_id: &str,
    error_json: &Value,
    finished_at_ms: i64,
) -> Result<bool, BackendError> {
    let error_json = serialize_json(error_json, "host_commands::mark_timed_out.error_json")?;

    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE host_commands
                    SET status = 'failed', error_json = ?, finished_at_ms = ?
                  WHERE command_id = ?
                    AND deadline_at_ms <= ?
                    AND finished_at_ms IS NULL
                    AND status IN ('pending', 'acked')",
        )
        .bind(error_json.as_str())
        .bind(finished_at_ms)
        .bind(command_id)
        .bind(finished_at_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE host_commands
                    SET status = 'failed', error_json = $1, finished_at_ms = $2
                  WHERE command_id = $3
                    AND deadline_at_ms <= $4
                    AND finished_at_ms IS NULL
                    AND status IN ('pending', 'acked')",
        )
        .bind(error_json.as_str())
        .bind(finished_at_ms)
        .bind(command_id)
        .bind(finished_at_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("host_commands::mark_timed_out"))?;
    Ok(result == 1)
}

fn decode_row(row: HostCommandRowTuple) -> Result<HostCommandRow, BackendError> {
    let (
        command_id,
        host_installation_id,
        agent_session_id,
        method,
        params_json,
        requested_by_account_id,
        status,
        response_json,
        error_json,
        deadline_at_ms,
        created_at_ms,
        ack_at_ms,
        finished_at_ms,
    ) = row;

    Ok(HostCommandRow {
        command_id,
        host_installation_id: Uuid::parse_str(&host_installation_id)
            .map(DeviceId)
            .map_err(|error| BackendError::StoreDecode {
                column: "host_commands.host_installation_id".into(),
                message: error.to_string(),
            })?,
        agent_session_id,
        method,
        params_json: decode_json("host_commands.params_json", &params_json)?,
        requested_by_account_id,
        status: parse_status(&status)?,
        response_json: response_json
            .as_deref()
            .map(|value| decode_json("host_commands.response_json", value))
            .transpose()?,
        error_json: error_json
            .as_deref()
            .map(|value| decode_json("host_commands.error_json", value))
            .transpose()?,
        deadline_at_ms,
        created_at_ms,
        ack_at_ms,
        finished_at_ms,
    })
}

fn parse_status(status: &str) -> Result<HostCommandStatus, BackendError> {
    match status {
        "pending" => Ok(HostCommandStatus::Pending),
        "acked" => Ok(HostCommandStatus::Acked),
        "succeeded" => Ok(HostCommandStatus::Succeeded),
        "failed" => Ok(HostCommandStatus::Failed),
        other => Err(BackendError::StoreDecode {
            column: "host_commands.status".into(),
            message: other.to_string(),
        }),
    }
}

fn serialize_json(value: &Value, operation: &'static str) -> Result<String, BackendError> {
    serde_json::to_string(value).map_err(|error| BackendError::StoreQuery {
        operation: operation.into(),
        message: error.to_string(),
    })
}

fn decode_json(column: &'static str, value: &str) -> Result<Value, BackendError> {
    serde_json::from_str(value).map_err(|error| BackendError::StoreDecode {
        column: column.into(),
        message: error.to_string(),
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

    async fn seed_host(pool: &SqlitePool) -> DeviceId {
        let host = DeviceId::new();
        insert_device(pool, host, "host", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        host
    }

    #[tokio::test]
    async fn enqueue_ack_finish_round_trip() {
        let pool = memory_pool().await;
        let host = seed_host(&pool).await;

        enqueue(
            &pool,
            "cmd-1",
            host,
            Some("session-1"),
            "approval/respond",
            &serde_json::json!({ "decision": "approve" }),
            None,
            T0 + 5_000,
            T0,
        )
        .await
        .unwrap();

        let row = get(&pool, "cmd-1").await.unwrap().unwrap();
        assert_eq!(row.status, HostCommandStatus::Pending);
        assert_eq!(row.agent_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            row.params_json,
            serde_json::json!({ "decision": "approve" })
        );

        assert!(ack(&pool, "cmd-1", T0 + 100).await.unwrap());
        assert!(!ack(&pool, "cmd-1", T0 + 200).await.unwrap());

        assert!(finish(
            &pool,
            "cmd-1",
            HostCommandTerminalStatus::Succeeded,
            Some(&serde_json::json!({ "ok": true })),
            None,
            T0 + 300,
        )
        .await
        .unwrap());

        let row = get(&pool, "cmd-1").await.unwrap().unwrap();
        assert_eq!(row.status, HostCommandStatus::Succeeded);
        assert_eq!(row.ack_at_ms, Some(T0 + 100));
        assert_eq!(row.finished_at_ms, Some(T0 + 300));
        assert_eq!(row.response_json, Some(serde_json::json!({ "ok": true })));
    }

    #[tokio::test]
    async fn list_and_mark_timed_out_commands() {
        let pool = memory_pool().await;
        let host = seed_host(&pool).await;

        enqueue(
            &pool,
            "cmd-timeout",
            host,
            None,
            "tool/call",
            &serde_json::json!({ "name": "apply_patch" }),
            None,
            T0 + 10,
            T0,
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            "cmd-future",
            host,
            None,
            "tool/call",
            &serde_json::json!({ "name": "read_file" }),
            None,
            T0 + 1_000,
            T0,
        )
        .await
        .unwrap();

        let rows = list_timed_out_open(&pool, T0 + 100, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].command_id, "cmd-timeout");

        assert!(mark_timed_out(
            &pool,
            "cmd-timeout",
            &serde_json::json!({ "kind": "timeout" }),
            T0 + 100,
        )
        .await
        .unwrap());

        let row = get(&pool, "cmd-timeout").await.unwrap().unwrap();
        assert_eq!(row.status, HostCommandStatus::Failed);
        assert_eq!(row.finished_at_ms, Some(T0 + 100));
        assert_eq!(
            row.error_json,
            Some(serde_json::json!({ "kind": "timeout" }))
        );
        assert!(list_timed_out_open(&pool, T0 + 100, 10)
            .await
            .unwrap()
            .is_empty());
    }
}
