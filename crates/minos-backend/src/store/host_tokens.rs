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

type HostInstallationTokenRowTuple = (
    String,
    String,
    Option<String>,
    i64,
    Option<i64>,
    Option<i64>,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInstallationTokenRow {
    pub token_hash: String,
    pub host_device_id: DeviceId,
    pub account_id: Option<String>,
    pub issued_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

pub(crate) async fn insert_token_with_executor<'e, E>(
    executor: E,
    token_hash: &str,
    host_device_id: DeviceId,
    account_id: Option<&str>,
    issued_at_ms: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host = host_device_id.to_string();
    sqlx::query(
        r#"
        INSERT INTO host_tokens
            (token_hash, host_device_id, account_id, issued_at_ms, last_used_at_ms, revoked_at_ms)
        VALUES (?, ?, ?, ?, NULL, NULL)
        "#,
    )
    .bind(token_hash)
    .bind(&host)
    .bind(account_id)
    .bind(issued_at_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_tokens::insert_token".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub(crate) async fn insert_token_with_postgres_executor<'e, E>(
    executor: E,
    token_hash: &str,
    host_device_id: DeviceId,
    account_id: Option<&str>,
    issued_at_ms: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host = host_device_id.to_string();
    sqlx::query(
        r#"
        INSERT INTO host_tokens
            (token_hash, host_device_id, account_id, issued_at_ms, last_used_at_ms, revoked_at_ms)
        VALUES ($1, $2, $3, $4, NULL, NULL)
        "#,
    )
    .bind(token_hash)
    .bind(&host)
    .bind(account_id)
    .bind(issued_at_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_tokens::insert_token".into(),
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
                UPDATE host_tokens
                SET last_used_at_ms = ?
                WHERE token_hash = ?
                  AND revoked_at_ms IS NULL
                RETURNING token_hash, host_device_id, account_id, issued_at_ms, last_used_at_ms, revoked_at_ms
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
                UPDATE host_tokens
                SET last_used_at_ms = $1
                WHERE token_hash = $2
                  AND revoked_at_ms IS NULL
                RETURNING token_hash, host_device_id, account_id, issued_at_ms, last_used_at_ms, revoked_at_ms
                "#,
            )
            .bind(now_ms)
            .bind(token_hash)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_tokens::verify_active_token".into(),
        message: e.to_string(),
    })?;
    row.map(decode_host_installation_token_row).transpose()
}

pub async fn revoke_all_for_host(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
    now_ms: i64,
) -> Result<u64, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            revoke_all_for_host_with_executor(pool, host_device_id, now_ms).await
        }
        StorePoolRef::Postgres(pool) => {
            revoke_all_for_host_with_postgres_executor(pool, host_device_id, now_ms).await
        }
    }
}

pub(crate) async fn revoke_all_for_host_with_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    now_ms: i64,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host = host_device_id.to_string();
    let result = sqlx::query(
        r#"
        UPDATE host_tokens
        SET revoked_at_ms = ?
        WHERE host_device_id = ?
          AND revoked_at_ms IS NULL
        "#,
    )
    .bind(now_ms)
    .bind(&host)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_tokens::revoke_all_for_host".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected())
}

pub(crate) async fn revoke_all_for_host_with_postgres_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    now_ms: i64,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host = host_device_id.to_string();
    let result = sqlx::query(
        r#"
        UPDATE host_tokens
        SET revoked_at_ms = $1
        WHERE host_device_id = $2
          AND revoked_at_ms IS NULL
        "#,
    )
    .bind(now_ms)
    .bind(&host)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_tokens::revoke_all_for_host".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected())
}

fn decode_host_installation_token_row(
    row: HostInstallationTokenRowTuple,
) -> Result<HostInstallationTokenRow, BackendError> {
    let (token_hash, host_device_id, account_id, issued_at_ms, last_used_at_ms, revoked_at_ms) =
        row;
    Ok(HostInstallationTokenRow {
        token_hash,
        host_device_id: Uuid::parse_str(&host_device_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_tokens.host_device_id".into(),
                message: e.to_string(),
            })?,
        account_id: account_id.filter(|id| !id.is_empty()),
        issued_at_ms,
        last_used_at_ms,
        revoked_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{memory_pool, T0};

    #[tokio::test]
    async fn verify_active_token_updates_last_used() {
        let pool = memory_pool().await;
        let host = DeviceId::new();
        crate::store::test_support::insert_test_host(&pool, host, "host", T0).await;
        let mut tx = pool.begin().await.unwrap();
        insert_token_with_executor(&mut *tx, "hash", host, None, T0)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let row = verify_active_token(&pool, "hash", T0 + 42)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(row.host_device_id, host);
        assert_eq!(row.last_used_at_ms, Some(T0 + 42));
    }
}
