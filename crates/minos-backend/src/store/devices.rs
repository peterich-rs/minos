//! `devices` table CRUD.
//!
//! ## Type strategy
//!
//! SQLite (via sqlx 0.8) stores `DeviceId` and `DeviceRole` as `TEXT`. Rather
//! than add crate-crossing `sqlx::Type` / `Encode` / `Decode` impls on the
//! `minos-domain` newtypes, we read/write the raw `String` columns and
//! parse on the Rust side:
//!
//! - Writes: `DeviceId::to_string()` and `DeviceRole::to_string()` (both use
//!   `Display`, which for `DeviceRole` is kebab-case — matches the CHECK
//!   constraint in `migrations/0001_devices.sql`).
//! - Reads: `String.parse::<Uuid>()` and `DeviceRole::from_str`.
//!
//! This keeps sqlx type plumbing contained in the backend crate and avoids
//! cross-crate scope creep. If future code paths need `sqlx::Type` on the
//! newtypes, we can revisit in `minos-domain`.

use minos_domain::{DeviceId, DeviceRole};
use sqlx::{Executor, Sqlite, SqlitePool};
use std::str::FromStr;
use uuid::Uuid;

use crate::error::BackendError;

/// A single row of the `devices` table after decoding into domain types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub device_id: DeviceId,
    pub display_name: String,
    pub role: DeviceRole,
    /// `None` until the device completes pairing and a bearer secret has
    /// been minted.
    pub secret_hash: Option<String>,
    /// Unix epoch milliseconds.
    pub created_at: i64,
    /// Unix epoch milliseconds.
    pub last_seen_at: i64,
    /// Account that owns this device, set when an iOS client logs in or
    /// the Mac side adopts the linked account through pairing. `None`
    /// while a device is unauthenticated to any account.
    pub account_id: Option<String>,
}

/// Insert a new device row.
///
/// Both `created_at` and `last_seen_at` are set to `now` (unix epoch ms).
/// `now` is injected from the caller so tests can use fixed-epoch literals.
/// The row is inserted with `secret_hash = NULL`; pair-time completion
/// happens via [`upsert_secret_hash`] and unpair-time revocation via
/// [`clear_secret_hash`].
pub async fn insert_device(
    pool: &SqlitePool,
    id: DeviceId,
    name: &str,
    role: DeviceRole,
    now: i64,
) -> Result<(), BackendError> {
    insert_device_with_executor(pool, id, name, role, now).await
}

pub(crate) async fn insert_device_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
    name: &str,
    role: DeviceRole,
    now: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();
    let role_str = role.to_string();

    sqlx::query!(
        r#"
        INSERT INTO devices (device_id, display_name, role, secret_hash, created_at, last_seen_at)
        VALUES (?, ?, ?, NULL, ?, ?)
        "#,
        id_str,
        name,
        role_str,
        now,
        now,
    )
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "insert_device".to_string(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Set (or overwrite) a device's argon2id `secret_hash`.
///
/// Returns [`BackendError::DeviceNotFound`] if no row matches `id`.
pub async fn upsert_secret_hash(
    pool: &SqlitePool,
    id: DeviceId,
    hash: &str,
) -> Result<(), BackendError> {
    upsert_secret_hash_with_executor(pool, id, hash).await
}

pub(crate) async fn upsert_secret_hash_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
    hash: &str,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();

    let result = sqlx::query!(
        r#"UPDATE devices SET secret_hash = ? WHERE device_id = ?"#,
        hash,
        id_str,
    )
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "upsert_secret_hash".to_string(),
        message: e.to_string(),
    })?;

    if result.rows_affected() == 0 {
        return Err(BackendError::DeviceNotFound { device_id: id_str });
    }

    Ok(())
}

/// Clear a device's stored `secret_hash`.
///
/// Returns [`BackendError::DeviceNotFound`] if no row matches `id`.
pub async fn clear_secret_hash(pool: &SqlitePool, id: DeviceId) -> Result<(), BackendError> {
    clear_secret_hash_with_executor(pool, id).await
}

pub(crate) async fn clear_secret_hash_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();

    let result = sqlx::query("UPDATE devices SET secret_hash = NULL WHERE device_id = ?")
        .bind(&id_str)
        .execute(executor)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "clear_secret_hash".to_string(),
            message: e.to_string(),
        })?;

    if result.rows_affected() == 0 {
        return Err(BackendError::DeviceNotFound { device_id: id_str });
    }

    Ok(())
}

/// Set the `account_id` on an existing device row.
///
/// Used at login time (iOS side) and at pairing-consume time (Mac side
/// inherits its peer's account). `account_id` is a UUIDv4 string; the
/// foreign-key reference to `accounts(account_id)` is enforced by SQLite.
pub async fn set_account_id(
    pool: &SqlitePool,
    device_id: &DeviceId,
    account_id: &str,
) -> Result<(), BackendError> {
    let id_str = device_id.to_string();
    sqlx::query("UPDATE devices SET account_id = ? WHERE device_id = ?")
        .bind(account_id)
        .bind(&id_str)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "set_account_id".to_string(),
            message: e.to_string(),
        })?;
    Ok(())
}

/// Look up a device by id.
///
/// Returns `Ok(None)` if the row does not exist.
pub async fn get_device(
    pool: &SqlitePool,
    id: DeviceId,
) -> Result<Option<DeviceRow>, BackendError> {
    get_device_with_executor(pool, id).await
}

pub(crate) async fn get_device_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
) -> Result<Option<DeviceRow>, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();

    let row = sqlx::query!(
        r#"
        SELECT device_id, display_name, role, secret_hash, created_at, last_seen_at, account_id
        FROM devices
        WHERE device_id = ?
        "#,
        id_str,
    )
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "get_device".to_string(),
        message: e.to_string(),
    })?;

    let Some(r) = row else {
        return Ok(None);
    };

    let device_id =
        Uuid::parse_str(&r.device_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "device_id".to_string(),
                message: e.to_string(),
            })?;
    let role = DeviceRole::from_str(&r.role).map_err(|e| BackendError::StoreDecode {
        column: "role".to_string(),
        message: e,
    })?;

    Ok(Some(DeviceRow {
        device_id,
        display_name: r.display_name,
        role,
        secret_hash: r.secret_hash,
        created_at: r.created_at,
        last_seen_at: r.last_seen_at,
        account_id: r.account_id,
    }))
}

/// List all device rows owned by `account_id`.
///
/// Used by the ingest fan-out (`broadcast_to_peers_of`) to find every
/// iOS recipient under a given Mac's paired account. Ordered by
/// `created_at ASC` so iteration order is stable across calls — the
/// caller filters by `role` so order between roles doesn't matter, but
/// stability still helps tests.
pub async fn list_by_account(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<DeviceRow>, BackendError> {
    let rows = sqlx::query!(
        r#"
        SELECT device_id, display_name, role, secret_hash, created_at, last_seen_at, account_id
        FROM devices
        WHERE account_id = ?
        ORDER BY created_at ASC
        "#,
        account_id,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "list_by_account".to_string(),
        message: e.to_string(),
    })?;
    rows.into_iter()
        .map(|r| {
            let device_id = Uuid::parse_str(&r.device_id).map(DeviceId).map_err(|e| {
                BackendError::StoreDecode {
                    column: "devices.device_id".to_string(),
                    message: e.to_string(),
                }
            })?;
            let role = DeviceRole::from_str(&r.role).map_err(|e| BackendError::StoreDecode {
                column: "devices.role".to_string(),
                message: e,
            })?;
            Ok(DeviceRow {
                device_id,
                display_name: r.display_name,
                role,
                secret_hash: r.secret_hash,
                created_at: r.created_at,
                last_seen_at: r.last_seen_at,
                account_id: r.account_id,
            })
        })
        .collect()
}

/// Return the argon2id `secret_hash` for a device, or `None` if the device
/// exists but has not completed pairing (hash column NULL) or does not
/// exist at all.
///
/// Callers that need to distinguish "unknown device" from "paired-but-no-
/// hash-yet" (the latter should never happen post-pair) can follow up with
/// [`get_device`].
pub async fn get_secret_hash(
    pool: &SqlitePool,
    id: DeviceId,
) -> Result<Option<String>, BackendError> {
    let id_str = id.to_string();

    // query_scalar! returns Option<Option<String>> for a nullable column on
    // fetch_optional: outer Option = row-present, inner = NULL-vs-set. We
    // flatten since we don't distinguish "no row" from "row with NULL hash"
    // at this API surface.
    let hash: Option<Option<String>> = sqlx::query_scalar!(
        r#"SELECT secret_hash FROM devices WHERE device_id = ?"#,
        id_str,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "get_secret_hash".to_string(),
        message: e.to_string(),
    })?;

    Ok(hash.flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{memory_pool, T0};
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn insert_then_get_round_trips_all_columns() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "alice's mac", DeviceRole::AgentHost, T0)
            .await
            .unwrap();

        let got = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.device_id, id);
        assert_eq!(got.display_name, "alice's mac");
        assert_eq!(got.role, DeviceRole::AgentHost);
        assert_eq!(got.secret_hash, None);
        assert_eq!(got.created_at, T0);
        assert_eq!(got.last_seen_at, T0);
        assert_eq!(got.account_id, None);
    }

    #[tokio::test]
    async fn set_account_id_links_existing_device_to_account() {
        let pool = memory_pool().await;
        // Seed an account so the FK constraint is satisfied.
        let account = crate::store::accounts::create(&pool, "alice@example.com", "phc")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_device(&pool, id, "iphone", DeviceRole::IosClient, T0)
            .await
            .unwrap();
        set_account_id(&pool, &id, &account.account_id)
            .await
            .unwrap();
        let got = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.account_id.as_deref(), Some(account.account_id.as_str()));
    }

    #[tokio::test]
    async fn get_device_missing_returns_none() {
        let pool = memory_pool().await;
        let missing = DeviceId::new();
        assert_eq!(get_device(&pool, missing).await.unwrap(), None);
    }

    #[tokio::test]
    async fn upsert_secret_hash_sets_hash_visible_to_get() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "ipad", DeviceRole::IosClient, T0)
            .await
            .unwrap();

        upsert_secret_hash(&pool, id, "$argon2id$v=19$m=19456,t=2,p=1$salt$hash")
            .await
            .unwrap();

        assert_eq!(
            get_secret_hash(&pool, id).await.unwrap(),
            Some("$argon2id$v=19$m=19456,t=2,p=1$salt$hash".to_string()),
        );
        let row = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.secret_hash,
            Some("$argon2id$v=19$m=19456,t=2,p=1$salt$hash".to_string()),
        );
    }

    #[tokio::test]
    async fn clear_secret_hash_removes_hash_visible_to_get() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "ipad", DeviceRole::IosClient, T0)
            .await
            .unwrap();
        upsert_secret_hash(&pool, id, "$argon2id$v=19$m=19456,t=2,p=1$salt$hash")
            .await
            .unwrap();

        clear_secret_hash(&pool, id).await.unwrap();

        assert_eq!(get_secret_hash(&pool, id).await.unwrap(), None);
        let row = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.secret_hash, None);
    }

    #[tokio::test]
    async fn upsert_secret_hash_on_missing_device_errors() {
        let pool = memory_pool().await;
        let missing = DeviceId::new();
        let err = upsert_secret_hash(&pool, missing, "hash")
            .await
            .unwrap_err();
        match err {
            BackendError::DeviceNotFound { device_id } => {
                assert_eq!(device_id, missing.to_string());
            }
            other => panic!("expected DeviceNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_secret_hash_on_missing_device_returns_none() {
        let pool = memory_pool().await;
        let missing = DeviceId::new();
        assert_eq!(get_secret_hash(&pool, missing).await.unwrap(), None);
    }

    #[tokio::test]
    async fn get_secret_hash_on_device_without_hash_returns_none() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "web", DeviceRole::BrowserAdmin, T0)
            .await
            .unwrap();
        assert_eq!(get_secret_hash(&pool, id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn insert_device_stores_role_as_kebab_case() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "admin", DeviceRole::BrowserAdmin, T0)
            .await
            .unwrap();

        let id_str = id.to_string();
        let raw_role: String =
            sqlx::query_scalar!("SELECT role FROM devices WHERE device_id = ?", id_str)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(raw_role, "browser-admin");
    }

    #[tokio::test]
    async fn ios_row_can_be_created_with_null_secret_hash() {
        // ADR-0020 regression: the iOS rail is bearer-only and the
        // device row's secret_hash must remain NULL for the lifetime
        // of the row. This test pins the insert default so a future
        // change can't silently start populating the column on iOS.
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "iPhone", DeviceRole::IosClient, 0)
            .await
            .unwrap();
        let row = get_device(&pool, id).await.unwrap().unwrap();
        assert!(row.secret_hash.is_none());
    }
}
