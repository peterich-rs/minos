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
//! This keeps sqlx type plumbing contained in the relay crate and avoids
//! cross-crate scope creep. If future code paths need `sqlx::Type` on the
//! newtypes, we can revisit in `minos-domain`.

use minos_domain::{DeviceId, DeviceRole};
use sqlx::SqlitePool;
use std::str::FromStr;
use uuid::Uuid;

use crate::error::RelayError;

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
}

/// Insert a new device row.
///
/// Both `created_at` and `last_seen_at` are set to `now` (unix epoch ms).
/// `now` is injected from the caller so tests can use fixed-epoch literals.
/// The row is inserted with `secret_hash = NULL`; pair-time completion
/// happens via [`upsert_secret_hash`].
pub async fn insert_device(
    pool: &SqlitePool,
    id: DeviceId,
    name: &str,
    role: DeviceRole,
    now: i64,
) -> Result<(), RelayError> {
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
    .execute(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "insert_device".to_string(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Set (or overwrite) a device's argon2id `secret_hash`.
///
/// Returns [`RelayError::DeviceNotFound`] if no row matches `id`.
pub async fn upsert_secret_hash(
    pool: &SqlitePool,
    id: DeviceId,
    hash: &str,
) -> Result<(), RelayError> {
    let id_str = id.to_string();

    let result = sqlx::query!(
        r#"UPDATE devices SET secret_hash = ? WHERE device_id = ?"#,
        hash,
        id_str,
    )
    .execute(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "upsert_secret_hash".to_string(),
        message: e.to_string(),
    })?;

    if result.rows_affected() == 0 {
        return Err(RelayError::DeviceNotFound { device_id: id_str });
    }

    Ok(())
}

/// Look up a device by id.
///
/// Returns `Ok(None)` if the row does not exist.
pub async fn get_device(pool: &SqlitePool, id: DeviceId) -> Result<Option<DeviceRow>, RelayError> {
    let id_str = id.to_string();

    let row = sqlx::query!(
        r#"
        SELECT device_id, display_name, role, secret_hash, created_at, last_seen_at
        FROM devices
        WHERE device_id = ?
        "#,
        id_str,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "get_device".to_string(),
        message: e.to_string(),
    })?;

    let Some(r) = row else {
        return Ok(None);
    };

    let device_id =
        Uuid::parse_str(&r.device_id)
            .map(DeviceId)
            .map_err(|e| RelayError::StoreDecode {
                column: "device_id".to_string(),
                message: e.to_string(),
            })?;
    let role = DeviceRole::from_str(&r.role).map_err(|e| RelayError::StoreDecode {
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
    }))
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
) -> Result<Option<String>, RelayError> {
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
    .map_err(|e| RelayError::StoreQuery {
        operation: "get_secret_hash".to_string(),
        message: e.to_string(),
    })?;

    Ok(hash.flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    /// Fixed unix-epoch ms used as `now` in tests.
    const T0: i64 = 1_700_000_000_000;

    async fn setup() -> SqlitePool {
        let opts: SqliteConnectOptions = "sqlite::memory:".parse().unwrap();
        let opts = opts.create_if_missing(true).foreign_keys(true);
        // `sqlite::memory:` is per-connection — each connection gets a
        // fresh DB. Cap at 1 so tests see a consistent store.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_then_get_round_trips_all_columns() {
        let pool = setup().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "alice's mac", DeviceRole::MacHost, T0)
            .await
            .unwrap();

        let got = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.device_id, id);
        assert_eq!(got.display_name, "alice's mac");
        assert_eq!(got.role, DeviceRole::MacHost);
        assert_eq!(got.secret_hash, None);
        assert_eq!(got.created_at, T0);
        assert_eq!(got.last_seen_at, T0);
    }

    #[tokio::test]
    async fn get_device_missing_returns_none() {
        let pool = setup().await;
        let missing = DeviceId::new();
        assert_eq!(get_device(&pool, missing).await.unwrap(), None);
    }

    #[tokio::test]
    async fn upsert_secret_hash_sets_hash_visible_to_get() {
        let pool = setup().await;
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
    async fn upsert_secret_hash_on_missing_device_errors() {
        let pool = setup().await;
        let missing = DeviceId::new();
        let err = upsert_secret_hash(&pool, missing, "hash")
            .await
            .unwrap_err();
        match err {
            RelayError::DeviceNotFound { device_id } => {
                assert_eq!(device_id, missing.to_string());
            }
            other => panic!("expected DeviceNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_secret_hash_on_missing_device_returns_none() {
        let pool = setup().await;
        let missing = DeviceId::new();
        assert_eq!(get_secret_hash(&pool, missing).await.unwrap(), None);
    }

    #[tokio::test]
    async fn get_secret_hash_on_device_without_hash_returns_none() {
        let pool = setup().await;
        let id = DeviceId::new();
        insert_device(&pool, id, "web", DeviceRole::BrowserAdmin, T0)
            .await
            .unwrap();
        assert_eq!(get_secret_hash(&pool, id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn insert_device_stores_role_as_kebab_case() {
        let pool = setup().await;
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
}
