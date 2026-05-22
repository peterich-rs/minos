//! Formal `pairing_codes` table access.
//!
//! A pairing code is the host-bootstrap credential that moves through the
//! fixed `pending -> confirmed -> redeemed` state machine. The store keeps
//! only the SHA-256 digest of the plaintext code.

use minos_domain::DeviceId;
use sqlx::{Executor, Postgres, Sqlite};
use std::str::FromStr;
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type PairingCodeRowTuple = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingCodeStatus {
    Pending,
    Confirmed,
    Redeemed,
    Expired,
}

impl PairingCodeStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Redeemed => "redeemed",
            Self::Expired => "expired",
        }
    }
}

impl FromStr for PairingCodeStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "confirmed" => Ok(Self::Confirmed),
            "redeemed" => Ok(Self::Redeemed),
            "expired" => Ok(Self::Expired),
            other => Err(format!("unknown pairing code status: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCodeRow {
    pub code_hash: String,
    pub host_installation_id: DeviceId,
    pub account_id: Option<String>,
    pub linked_via_installation_id: Option<DeviceId>,
    pub status: PairingCodeStatus,
    pub client_request_id: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub confirmed_at_ms: Option<i64>,
    pub redeemed_at_ms: Option<i64>,
}

pub async fn insert_code(
    store: &impl AsStorePool,
    code_hash: &str,
    host_installation_id: DeviceId,
    created_at_ms: i64,
    expires_at_ms: i64,
) -> Result<(), BackendError> {
    let host = host_installation_id.to_string();
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            r#"
                INSERT INTO pairing_codes
                    (code_hash, host_installation_id, status, created_at_ms, expires_at_ms)
                VALUES (?, ?, 'pending', ?, ?)
                "#,
        )
        .bind(code_hash)
        .bind(&host)
        .bind(created_at_ms)
        .bind(expires_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            r#"
                INSERT INTO pairing_codes
                    (code_hash, host_installation_id, status, created_at_ms, expires_at_ms)
                VALUES ($1, $2, 'pending', $3, $4)
                "#,
        )
        .bind(code_hash)
        .bind(&host)
        .bind(created_at_ms)
        .bind(expires_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "pairing_codes::insert_code".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub(crate) async fn get_code_with_executor<'e, E>(
    executor: E,
    code_hash: &str,
) -> Result<Option<PairingCodeRow>, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query_as::<_, PairingCodeRowTuple>(
        r#"
        SELECT
            code_hash,
            host_installation_id,
            account_id,
            linked_via_installation_id,
            status,
            client_request_id,
            created_at_ms,
            expires_at_ms,
            confirmed_at_ms,
            redeemed_at_ms
        FROM pairing_codes
        WHERE code_hash = ?
        "#,
    )
    .bind(code_hash)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "pairing_codes::get_code".into(),
        message: e.to_string(),
    })?;

    row.map(decode_pairing_code_row).transpose()
}

pub(crate) async fn get_code_with_postgres_executor<'e, E>(
    executor: E,
    code_hash: &str,
) -> Result<Option<PairingCodeRow>, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, PairingCodeRowTuple>(
        r#"
        SELECT
            code_hash,
            host_installation_id,
            account_id,
            linked_via_installation_id,
            status,
            client_request_id,
            created_at_ms,
            expires_at_ms,
            confirmed_at_ms,
            redeemed_at_ms
        FROM pairing_codes
        WHERE code_hash = $1
        FOR UPDATE
        "#,
    )
    .bind(code_hash)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "pairing_codes::get_code".into(),
        message: e.to_string(),
    })?;

    row.map(decode_pairing_code_row).transpose()
}

pub(crate) async fn confirm_code_with_executor<'e, E>(
    executor: E,
    code_hash: &str,
    account_id: &str,
    linked_via_installation_id: DeviceId,
    client_request_id: Option<&str>,
    now_ms: i64,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let linked_via = linked_via_installation_id.to_string();
    let result = sqlx::query(
        r#"
        UPDATE pairing_codes
        SET
            status = 'confirmed',
            account_id = ?,
            linked_via_installation_id = ?,
            client_request_id = ?,
            confirmed_at_ms = COALESCE(confirmed_at_ms, ?)
        WHERE code_hash = ?
          AND status = 'pending'
          AND expires_at_ms > ?
        "#,
    )
    .bind(account_id)
    .bind(&linked_via)
    .bind(client_request_id)
    .bind(now_ms)
    .bind(code_hash)
    .bind(now_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "pairing_codes::confirm_code".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected())
}

pub(crate) async fn confirm_code_with_postgres_executor<'e, E>(
    executor: E,
    code_hash: &str,
    account_id: &str,
    linked_via_installation_id: DeviceId,
    client_request_id: Option<&str>,
    now_ms: i64,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let linked_via = linked_via_installation_id.to_string();
    let result = sqlx::query(
        r#"
        UPDATE pairing_codes
        SET
            status = 'confirmed',
            account_id = $1,
            linked_via_installation_id = $2,
            client_request_id = $3,
            confirmed_at_ms = COALESCE(confirmed_at_ms, $4)
        WHERE code_hash = $5
          AND status = 'pending'
          AND expires_at_ms > $6
        "#,
    )
    .bind(account_id)
    .bind(&linked_via)
    .bind(client_request_id)
    .bind(now_ms)
    .bind(code_hash)
    .bind(now_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "pairing_codes::confirm_code".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected())
}

pub(crate) async fn redeem_code_with_executor<'e, E>(
    executor: E,
    code_hash: &str,
    host_installation_id: DeviceId,
    now_ms: i64,
) -> Result<Option<String>, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host = host_installation_id.to_string();
    let account_id = sqlx::query_scalar::<_, String>(
        r#"
        UPDATE pairing_codes
        SET
            status = 'redeemed',
            redeemed_at_ms = ?
        WHERE code_hash = ?
          AND host_installation_id = ?
          AND status = 'confirmed'
          AND expires_at_ms > ?
        RETURNING account_id
        "#,
    )
    .bind(now_ms)
    .bind(code_hash)
    .bind(&host)
    .bind(now_ms)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "pairing_codes::redeem_code".into(),
        message: e.to_string(),
    })?;
    Ok(account_id)
}

pub(crate) async fn redeem_code_with_postgres_executor<'e, E>(
    executor: E,
    code_hash: &str,
    host_installation_id: DeviceId,
    now_ms: i64,
) -> Result<Option<String>, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host = host_installation_id.to_string();
    let account_id = sqlx::query_scalar::<_, String>(
        r#"
        UPDATE pairing_codes
        SET
            status = 'redeemed',
            redeemed_at_ms = $1
        WHERE code_hash = $2
          AND host_installation_id = $3
          AND status = 'confirmed'
          AND expires_at_ms > $4
        RETURNING account_id
        "#,
    )
    .bind(now_ms)
    .bind(code_hash)
    .bind(&host)
    .bind(now_ms)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "pairing_codes::redeem_code".into(),
        message: e.to_string(),
    })?;
    Ok(account_id)
}

fn decode_pairing_code_row(row: PairingCodeRowTuple) -> Result<PairingCodeRow, BackendError> {
    let (
        code_hash,
        host_installation_id,
        account_id,
        linked_via_installation_id,
        status,
        client_request_id,
        created_at_ms,
        expires_at_ms,
        confirmed_at_ms,
        redeemed_at_ms,
    ) = row;

    Ok(PairingCodeRow {
        code_hash,
        host_installation_id: parse_device_id(&host_installation_id, "host_installation_id")?,
        account_id,
        linked_via_installation_id: linked_via_installation_id
            .as_deref()
            .map(|raw| parse_device_id(raw, "linked_via_installation_id"))
            .transpose()?,
        status: status
            .parse()
            .map_err(|message| BackendError::StoreDecode {
                column: "pairing_codes.status".into(),
                message,
            })?,
        client_request_id,
        created_at_ms,
        expires_at_ms,
        confirmed_at_ms,
        redeemed_at_ms,
    })
}

fn parse_device_id(raw: &str, column: &str) -> Result<DeviceId, BackendError> {
    Uuid::parse_str(raw)
        .map(DeviceId)
        .map_err(|e| BackendError::StoreDecode {
            column: format!("pairing_codes.{column}"),
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::devices;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn insert_and_confirm_round_trip() {
        let pool = memory_pool().await;
        let host = DeviceId::new();
        devices::insert_device(&pool, host, "host", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        let account = crate::store::accounts::create(&pool, "pairing-code@example.com", "phc")
            .await
            .unwrap();
        insert_code(&pool, "hash", host, T0, T0 + 60_000)
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let rows = confirm_code_with_executor(
            &mut *tx,
            "hash",
            &account.account_id,
            host,
            Some("client-request"),
            T0 + 1,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(rows, 1);

        let mut tx = pool.begin().await.unwrap();
        let row = get_code_with_executor(&mut *tx, "hash")
            .await
            .unwrap()
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(row.status, PairingCodeStatus::Confirmed);
        assert_eq!(row.account_id.as_deref(), Some(account.account_id.as_str()));
        assert_eq!(row.linked_via_installation_id, Some(host));
    }
}
