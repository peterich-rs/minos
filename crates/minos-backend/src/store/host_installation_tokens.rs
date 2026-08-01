//! Formal host installation token persistence.
//!
//! The plaintext token is returned once to the host during pairing redeem.
//! The database stores only a SHA-256 digest and validates steady-state host
//! rail requests from that digest.

use minos_domain::DeviceId;
use sqlx::{Executor, Postgres, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type HostInstallationTokenRowTuple = (String, String, i64, Option<i64>, Option<i64>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInstallationTokenRow {
    pub token_hash: String,
    pub host_installation_id: DeviceId,
    pub issued_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

pub(crate) async fn insert_token_with_executor<'e, E>(
    executor: E,
    token_hash: &str,
    host_installation_id: DeviceId,
    issued_at_ms: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host = host_installation_id.to_string();
    sqlx::query(
        r#"
        INSERT INTO host_installation_tokens
            (token_hash, host_installation_id, issued_at_ms, last_used_at_ms, revoked_at_ms)
        VALUES (?, ?, ?, NULL, NULL)
        "#,
    )
    .bind(token_hash)
    .bind(&host)
    .bind(issued_at_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_installation_tokens::insert_token".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub(crate) async fn insert_token_with_postgres_executor<'e, E>(
    executor: E,
    token_hash: &str,
    host_installation_id: DeviceId,
    issued_at_ms: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host = host_installation_id.to_string();
    sqlx::query(
        r#"
        INSERT INTO host_installation_tokens
            (token_hash, host_installation_id, issued_at_ms, last_used_at_ms, revoked_at_ms)
        VALUES ($1, $2, $3, NULL, NULL)
        "#,
    )
    .bind(token_hash)
    .bind(&host)
    .bind(issued_at_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_installation_tokens::insert_token".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub async fn verify_active_token(
    store: &impl AsStorePool,
    token_hash: &str,
    now_ms: i64,
) -> Result<Option<HostInstallationTokenRow>, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, HostInstallationTokenRowTuple>(
                r#"
                UPDATE host_installation_tokens
                SET last_used_at_ms = ?
                WHERE token_hash = ?
                  AND revoked_at_ms IS NULL
                RETURNING token_hash, host_installation_id, issued_at_ms, last_used_at_ms, revoked_at_ms
                "#,
            )
            .bind(now_ms)
            .bind(token_hash)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, HostInstallationTokenRowTuple>(
                r#"
                UPDATE host_installation_tokens
                SET last_used_at_ms = $1
                WHERE token_hash = $2
                  AND revoked_at_ms IS NULL
                RETURNING token_hash, host_installation_id, issued_at_ms, last_used_at_ms, revoked_at_ms
                "#,
            )
            .bind(now_ms)
            .bind(token_hash)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_installation_tokens::verify_active_token".into(),
        message: e.to_string(),
    })?;
    row.map(decode_host_installation_token_row).transpose()
}

pub async fn revoke_all_for_host(
    store: &impl AsStorePool,
    host_installation_id: DeviceId,
    now_ms: i64,
) -> Result<u64, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            revoke_all_for_host_with_executor(pool, host_installation_id, now_ms).await
        }
        StorePoolRef::Postgres(pool) => {
            revoke_all_for_host_with_postgres_executor(pool, host_installation_id, now_ms).await
        }
    }
}

pub(crate) async fn revoke_all_for_host_with_executor<'e, E>(
    executor: E,
    host_installation_id: DeviceId,
    now_ms: i64,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host = host_installation_id.to_string();
    let result = sqlx::query(
        r#"
        UPDATE host_installation_tokens
        SET revoked_at_ms = ?
        WHERE host_installation_id = ?
          AND revoked_at_ms IS NULL
        "#,
    )
    .bind(now_ms)
    .bind(&host)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_installation_tokens::revoke_all_for_host".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected())
}

pub(crate) async fn revoke_all_for_host_with_postgres_executor<'e, E>(
    executor: E,
    host_installation_id: DeviceId,
    now_ms: i64,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host = host_installation_id.to_string();
    let result = sqlx::query(
        r#"
        UPDATE host_installation_tokens
        SET revoked_at_ms = $1
        WHERE host_installation_id = $2
          AND revoked_at_ms IS NULL
        "#,
    )
    .bind(now_ms)
    .bind(&host)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_installation_tokens::revoke_all_for_host".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected())
}

fn decode_host_installation_token_row(
    row: HostInstallationTokenRowTuple,
) -> Result<HostInstallationTokenRow, BackendError> {
    let (token_hash, host_installation_id, issued_at_ms, last_used_at_ms, revoked_at_ms) = row;
    Ok(HostInstallationTokenRow {
        token_hash,
        host_installation_id: Uuid::parse_str(&host_installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_installation_tokens.host_installation_id".into(),
                message: e.to_string(),
            })?,
        issued_at_ms,
        last_used_at_ms,
        revoked_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::device_installations;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::DeviceRole;

    #[tokio::test]
    async fn verify_active_token_updates_last_used() {
        let pool = memory_pool().await;
        let host = DeviceId::new();
        device_installations::insert_device(&pool, host, "host", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        let mut tx = pool.begin().await.unwrap();
        insert_token_with_executor(&mut *tx, "hash", host, T0)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let row = verify_active_token(&pool, "hash", T0 + 42)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(row.host_installation_id, host);
        assert_eq!(row.last_used_at_ms, Some(T0 + 42));
    }
}
