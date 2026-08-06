//! `device_installations` table CRUD.
//!
//! ## Storage vs wire
//!
//! Rows store `installation_kind` (`mobile`/`browser`/`desktop`/`host`).
//! The Rust API continues to speak wire [`DeviceRole`]
//! (`mobile-client`/…/`agent-host`) and maps at the boundary via
//! [`DeviceRole::to_installation_kind`] / [`DeviceRole::from_installation_kind`].
//!
//! Column names match Postgres; SQLite uses the same names so both backends
//! share SQL shape (placeholder dialect only differs).

use minos_domain::{DeviceId, DeviceRole};
use sqlx::{Executor, PgPool, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type InstallationRowTuple = (
    String,         // installation_id
    String,         // kind
    Option<String>, // display_name
    Option<String>, // public_key
    i64,            // created_at_ms
    i64,            // last_seen_at_ms
    Option<String>, // account_id
);

/// A single installation row after decoding into domain types.
///
/// Field names keep the historical `device_*` vocabulary used across HTTP
/// auth and pairing so call sites stay stable; the underlying table is
/// `device_installations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub device_id: DeviceId,
    pub display_name: String,
    pub role: DeviceRole,
    /// Host installation Ed25519 public key, stored as `ed25519:<base64url>`.
    /// Present only for host rows that have completed bootstrap TOFU.
    pub public_key: Option<String>,
    /// Unix epoch milliseconds.
    pub created_at: i64,
    /// Unix epoch milliseconds.
    pub last_seen_at: i64,
    /// Account that owns this client installation. `None` for hosts (CHECK)
    /// and for client rows not yet bound via login/exchange.
    pub account_id: Option<String>,
}

/// SQLite-tx host insert with an explicit public_key (host_link / tx paths).
#[allow(dead_code)] // reserved for in-tx host bootstrap paths
pub(crate) async fn insert_host_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
    name: &str,
    public_key: &str,
    now: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();
    let kind = DeviceRole::AgentHost.to_installation_kind();

    sqlx::query(
        r#"
        INSERT INTO device_installations
            (installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
        VALUES (?, ?, ?, ?, ?, ?, NULL)
        "#,
    )
    .bind(&id_str)
    .bind(kind)
    .bind(name)
    .bind(public_key)
    .bind(now)
    .bind(now)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "insert_host_with_executor".to_string(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Insert a host installation with its TOFU public key in one step.
///
/// Preferred for Postgres (strict CHECK requires `public_key IS NOT NULL` for
/// `kind = host`). SQLite also accepts this path.
pub async fn insert_host_with_public_key(
    store: &impl AsStorePool,
    id: DeviceId,
    name: &str,
    public_key: &str,
    now: i64,
) -> Result<(), BackendError> {
    let id_str = id.to_string();
    let kind = DeviceRole::AgentHost.to_installation_kind();
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                r#"
                INSERT INTO device_installations
                    (installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
                VALUES (?, ?, ?, ?, ?, ?, NULL)
                "#,
            )
            .bind(&id_str)
            .bind(kind)
            .bind(name)
            .bind(public_key)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                r#"
                INSERT INTO device_installations
                    (installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
                VALUES ($1, $2::installation_kind, $3, $4, $5, $6, NULL)
                "#,
            )
            .bind(&id_str)
            .bind(kind)
            .bind(name)
            .bind(public_key)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map(|_| ())
        }
    };
    result.map_err(|e| BackendError::StoreQuery {
        operation: "insert_host_with_public_key".to_string(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Insert a client installation already bound to an account.
///
/// Preferred for Postgres (strict CHECK requires `account_id IS NOT NULL` for
/// client kinds).
pub async fn insert_client_for_account(
    store: &impl AsStorePool,
    id: DeviceId,
    name: &str,
    role: DeviceRole,
    account_id: &str,
    now: i64,
) -> Result<(), BackendError> {
    if !role.is_account_client() {
        return Err(BackendError::StoreQuery {
            operation: "insert_client_for_account".into(),
            message: format!("role {role} is not an account client"),
        });
    }
    let id_str = id.to_string();
    let kind = role.to_installation_kind();
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                r#"
                INSERT INTO device_installations
                    (installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
                VALUES (?, ?, ?, NULL, ?, ?, ?)
                "#,
            )
            .bind(&id_str)
            .bind(kind)
            .bind(name)
            .bind(now)
            .bind(now)
            .bind(account_id)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                r#"
                INSERT INTO device_installations
                    (installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
                VALUES ($1, $2::installation_kind, $3, NULL, $4, $5, $6)
                "#,
            )
            .bind(&id_str)
            .bind(kind)
            .bind(name)
            .bind(now)
            .bind(now)
            .bind(account_id)
            .execute(pool)
            .await
            .map(|_| ())
        }
    };
    result.map_err(|e| BackendError::StoreQuery {
        operation: "insert_client_for_account".to_string(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Set the `account_id` on an existing client installation.
///
/// Used at login time and at pairing-consume time (mobile side). Hosts must
/// keep `account_id` NULL (CHECK); pair them via `host_links` instead.
pub async fn set_account_id(
    store: &impl AsStorePool,
    device_id: &DeviceId,
    account_id: &str,
) -> Result<(), BackendError> {
    let id_str = device_id.to_string();
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query("UPDATE device_installations SET account_id = ? WHERE installation_id = ?")
                .bind(account_id)
                .bind(&id_str)
                .execute(pool)
                .await
                .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE device_installations SET account_id = $1 WHERE installation_id = $2",
        )
        .bind(account_id)
        .bind(&id_str)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "set_account_id".to_string(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Update the display name on an existing installation row.
pub async fn set_display_name(
    store: &impl AsStorePool,
    device_id: &DeviceId,
    display_name: &str,
) -> Result<(), BackendError> {
    let id_str = device_id.to_string();
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE device_installations SET display_name = ? WHERE installation_id = ?",
        )
        .bind(display_name)
        .bind(&id_str)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE device_installations SET display_name = $1 WHERE installation_id = $2",
        )
        .bind(display_name)
        .bind(&id_str)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "set_display_name".to_string(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Update an installation's `last_seen_at_ms` timestamp.
///
/// Returns [`BackendError::DeviceNotFound`] if no row matches `device_id`.
pub async fn touch_last_seen(
    store: &impl AsStorePool,
    device_id: &DeviceId,
    at_ms: i64,
) -> Result<(), BackendError> {
    let id_str = device_id.to_string();
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE device_installations SET last_seen_at_ms = ? WHERE installation_id = ?",
        )
        .bind(at_ms)
        .bind(&id_str)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE device_installations SET last_seen_at_ms = $1 WHERE installation_id = $2",
        )
        .bind(at_ms)
        .bind(&id_str)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "touch_last_seen".to_string(),
        message: e.to_string(),
    })?;

    if rows_affected == 0 {
        return Err(BackendError::DeviceNotFound { device_id: id_str });
    }

    Ok(())
}

/// Look up an installation by id.
///
/// Returns `Ok(None)` if the row does not exist.
pub async fn get_device(
    store: &impl AsStorePool,
    id: DeviceId,
) -> Result<Option<DeviceRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => get_device_with_executor(pool, id).await,
        StorePoolRef::Postgres(pool) => get_device_postgres(pool, id).await,
    }
}

pub(crate) async fn get_device_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
) -> Result<Option<DeviceRow>, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();

    let row = sqlx::query_as::<_, InstallationRowTuple>(
        r#"
        SELECT installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id
        FROM device_installations
        WHERE installation_id = ?
        "#,
    )
    .bind(&id_str)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "get_device".to_string(),
        message: e.to_string(),
    })?;

    let Some(row) = row else {
        return Ok(None);
    };

    decode_installation_row(row).map(Some)
}

/// List all installation rows owned by `account_id`.
pub async fn list_by_account(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Vec<DeviceRow>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, InstallationRowTuple>(
                r#"
                SELECT installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id
                FROM device_installations
                WHERE account_id = ?
                ORDER BY created_at_ms ASC
                "#,
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, InstallationRowTuple>(
                r#"
                SELECT installation_id, kind::text, display_name, public_key, created_at_ms, last_seen_at_ms, account_id
                FROM device_installations
                WHERE account_id = $1
                ORDER BY created_at_ms ASC
                "#,
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "list_by_account".to_string(),
        message: e.to_string(),
    })?;
    rows.into_iter().map(decode_installation_row).collect()
}

/// Return the latest mobile-client installation for an account.
pub async fn latest_mobile_for_account(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Option<DeviceRow>, BackendError> {
    let kind = DeviceRole::MobileClient.to_installation_kind();
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, InstallationRowTuple>(
                r#"
                SELECT installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id
                FROM device_installations
                WHERE account_id = ? AND kind = ?
                ORDER BY last_seen_at_ms DESC, created_at_ms DESC
                LIMIT 1
                "#,
            )
            .bind(account_id)
            .bind(kind)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, InstallationRowTuple>(
                r#"
                SELECT installation_id, kind::text, display_name, public_key, created_at_ms, last_seen_at_ms, account_id
                FROM device_installations
                WHERE account_id = $1 AND kind = $2::installation_kind
                ORDER BY last_seen_at_ms DESC, created_at_ms DESC
                LIMIT 1
                "#,
            )
            .bind(account_id)
            .bind(kind)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "latest_mobile_for_account".to_string(),
        message: e.to_string(),
    })?;

    row.map(decode_installation_row).transpose()
}

/// Set the host bootstrap public key when it is not already recorded.
///
/// Returns `true` when this call performed the TOFU registration, `false`
/// when a key was already present.
pub async fn set_public_key_if_absent(
    store: &impl AsStorePool,
    device_id: &DeviceId,
    public_key: &str,
) -> Result<bool, BackendError> {
    let id_str = device_id.to_string();
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE device_installations SET public_key = ? \
             WHERE installation_id = ? AND public_key IS NULL",
        )
        .bind(public_key)
        .bind(&id_str)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE device_installations SET public_key = $1 \
             WHERE installation_id = $2 AND public_key IS NULL",
        )
        .bind(public_key)
        .bind(&id_str)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "set_public_key_if_absent".to_string(),
        message: e.to_string(),
    })?;

    Ok(result == 1)
}

fn decode_installation_row(row: InstallationRowTuple) -> Result<DeviceRow, BackendError> {
    let (installation_id, kind, display_name, public_key, created_at, last_seen_at, account_id) =
        row;
    let device_id = Uuid::parse_str(&installation_id)
        .map(DeviceId)
        .map_err(|e| BackendError::StoreDecode {
            column: "device_installations.installation_id".to_string(),
            message: e.to_string(),
        })?;
    let role =
        DeviceRole::from_installation_kind(&kind).map_err(|e| BackendError::StoreDecode {
            column: "device_installations.kind".to_string(),
            message: e,
        })?;

    Ok(DeviceRow {
        device_id,
        display_name: display_name.unwrap_or_default(),
        role,
        public_key,
        created_at,
        last_seen_at,
        account_id,
    })
}

async fn get_device_postgres(
    pool: &PgPool,
    id: DeviceId,
) -> Result<Option<DeviceRow>, BackendError> {
    let id_str = id.to_string();

    let row = sqlx::query_as::<_, InstallationRowTuple>(
        r#"
        SELECT installation_id, kind::text, display_name, public_key, created_at_ms, last_seen_at_ms, account_id
        FROM device_installations
        WHERE installation_id = $1
        "#,
    )
    .bind(&id_str)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "get_device".to_string(),
        message: e.to_string(),
    })?;

    let Some(row) = row else {
        return Ok(None);
    };

    decode_installation_row(row).map(Some)
}

// Silence unused import when SqlitePool only appears in type paths elsewhere.
#[allow(dead_code)]
fn _sqlite_pool_type_anchor(_: &SqlitePool) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_test_host, memory_pool, TEST_HOST_PUBLIC_KEY, T0};
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn insert_then_get_round_trips_all_columns() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_test_host(&pool, id, "alice's mac", T0).await;

        let got = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.device_id, id);
        assert_eq!(got.display_name, "alice's mac");
        assert_eq!(got.role, DeviceRole::AgentHost);
        assert_eq!(got.created_at, T0);
        assert_eq!(got.last_seen_at, T0);
        assert_eq!(got.account_id, None);
        assert_eq!(got.public_key.as_deref(), Some(TEST_HOST_PUBLIC_KEY));
    }

    #[tokio::test]
    async fn set_account_id_links_existing_device_to_account() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "alice@example.com")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_client_for_account(
            &pool,
            id,
            "iphone",
            DeviceRole::MobileClient,
            &account.account_id,
            T0,
        )
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
    async fn set_display_name_overwrites_existing_name() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "name@example.com")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_client_for_account(
            &pool,
            id,
            "unnamed",
            DeviceRole::MobileClient,
            &account.account_id,
            T0,
        )
        .await
        .unwrap();

        set_display_name(&pool, &id, "Fan's iPhone").await.unwrap();

        let got = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.display_name, "Fan's iPhone");
    }

    #[tokio::test]
    async fn touch_last_seen_updates_timestamp() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "touch@example.com")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_client_for_account(
            &pool,
            id,
            "iphone",
            DeviceRole::MobileClient,
            &account.account_id,
            T0,
        )
        .await
        .unwrap();

        touch_last_seen(&pool, &id, T0 + 500).await.unwrap();

        let got = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.last_seen_at, T0 + 500);
    }

    #[tokio::test]
    async fn latest_mobile_for_account_ignores_browser_admin() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "mobile-latest@example.com")
            .await
            .unwrap();
        let older_mobile = DeviceId::new();
        let latest_mobile = DeviceId::new();
        let browser = DeviceId::new();
        insert_client_for_account(
            &pool,
            older_mobile,
            "old phone",
            DeviceRole::MobileClient,
            &account.account_id,
            100,
        )
        .await
        .unwrap();
        insert_client_for_account(
            &pool,
            latest_mobile,
            "current phone",
            DeviceRole::MobileClient,
            &account.account_id,
            200,
        )
        .await
        .unwrap();
        insert_client_for_account(
            &pool,
            browser,
            "web",
            DeviceRole::BrowserAdmin,
            &account.account_id,
            300,
        )
        .await
        .unwrap();

        let got = latest_mobile_for_account(&pool, &account.account_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(got.device_id, latest_mobile);
        assert_eq!(got.display_name, "current phone");
    }

    #[tokio::test]
    async fn insert_device_stores_kind_as_installation_kind() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "kind@example.com")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_client_for_account(
            &pool,
            id,
            "admin",
            DeviceRole::BrowserAdmin,
            &account.account_id,
            T0,
        )
        .await
        .unwrap();

        let id_str = id.to_string();
        let raw_kind: String =
            sqlx::query_scalar("SELECT kind FROM device_installations WHERE installation_id = ?")
                .bind(&id_str)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(raw_kind, "browser");
    }

    #[tokio::test]
    async fn host_row_rejects_account_id_by_check() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "host-bind@example.com")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_test_host(&pool, id, "mac", T0).await;
        let err = set_account_id(&pool, &id, &account.account_id)
            .await
            .unwrap_err();
        match err {
            BackendError::StoreQuery { message, .. } => {
                assert!(
                    message.to_lowercase().contains("check")
                        || message.to_lowercase().contains("constraint"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected StoreQuery, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn insert_client_for_account_binds_account_atomically() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "client-insert@example.com")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_client_for_account(
            &pool,
            id,
            "iPhone",
            DeviceRole::MobileClient,
            &account.account_id,
            T0,
        )
        .await
        .unwrap();
        let row = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.role, DeviceRole::MobileClient);
        assert_eq!(row.account_id.as_deref(), Some(account.account_id.as_str()));
        assert!(row.public_key.is_none());
    }

    #[tokio::test]
    async fn insert_client_for_account_rejects_host_role() {
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "host-as-client@example.com")
            .await
            .unwrap();
        let err = insert_client_for_account(
            &pool,
            DeviceId::new(),
            "mac",
            DeviceRole::AgentHost,
            &account.account_id,
            T0,
        )
        .await
        .unwrap_err();
        match err {
            BackendError::StoreQuery { message, .. } => {
                assert!(
                    message.contains("not an account client"),
                    "unexpected: {message}"
                );
            }
            other => panic!("expected StoreQuery, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn insert_host_with_public_key_sets_key_atomically() {
        let pool = memory_pool().await;
        let id = DeviceId::new();
        insert_host_with_public_key(&pool, id, "Mac", "ed25519:testkey", T0)
            .await
            .unwrap();
        let row = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.role, DeviceRole::AgentHost);
        assert_eq!(row.public_key.as_deref(), Some("ed25519:testkey"));
        assert!(row.account_id.is_none());
    }

    #[tokio::test]
    async fn insert_client_for_account_is_required_shape_for_exchange_style_bind() {
        // Regression: exchange/login must never create a client with null
        // account_id (Postgres CHECK). This asserts the CHECK-compliant
        // helper writes account_id in the same INSERT.
        let pool = memory_pool().await;
        let account = crate::store::accounts::create(&pool, "exchange-shape@example.com")
            .await
            .unwrap();
        let id = DeviceId::new();
        insert_client_for_account(
            &pool,
            id,
            "Desktop",
            DeviceRole::DesktopConsole,
            &account.account_id,
            T0,
        )
        .await
        .unwrap();
        let (kind, account_id, public_key): (String, Option<String>, Option<String>) =
            sqlx::query_as(
                "SELECT kind, account_id, public_key FROM device_installations WHERE installation_id = ?",
            )
            .bind(id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kind, "desktop");
        assert_eq!(account_id.as_deref(), Some(account.account_id.as_str()));
        assert!(public_key.is_none());
    }
}
