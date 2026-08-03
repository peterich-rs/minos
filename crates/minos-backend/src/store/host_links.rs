//! Persistence for `host_links` (account ↔ host installation).
//!
//! Pair model is `(account_id, host_installation_id)`. The client
//! installation that performed the link is recorded as
//! `linked_via_installation_id` for audit only — it does not participate in
//! routing.
//!
//! Field names on [`PairRow`] keep historical `host_device_id` /
//! `mobile_account_id` vocabulary for call-site stability; SQL uses the
//! Postgres-aligned column names on both backends.

use minos_domain::{DeviceId, DeviceRole};
use sqlx::{Executor, Postgres, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type PairRowTuple = (String, String, String, String, Option<String>, i64);

/// A single host-link row after decoding stringly columns into domain ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRow {
    pub pair_id: String,
    pub host_device_id: DeviceId,
    pub mobile_account_id: String,
    /// Client installation that established the link (audit only).
    pub paired_via_device_id: DeviceId,
    pub link_display_name: Option<String>,
    pub paired_at_ms: i64,
}

/// Upsert a host link and return the resulting row.
///
/// On conflict `host_installation_id` (exclusive ownership) refreshes
/// metadata **only** when the existing row belongs to the same account.
/// Returns [`BackendError::HostLinkedElsewhere`] when the host is already
/// bound to a different account (empty RETURNING after conflict WHERE).
///
/// Callers that need exclusivity under concurrency must run
/// [`assert_host_available_or_same_account_*`] in the **same** write
/// transaction before this upsert.
pub async fn upsert_link(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    link_display_name: Option<&str>,
    now_ms: i64,
) -> Result<PairRow, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            upsert_link_with_executor(
                pool,
                host_device_id,
                mobile_account_id,
                paired_via_device_id,
                link_display_name,
                now_ms,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            upsert_link_with_postgres_executor(
                pool,
                host_device_id,
                mobile_account_id,
                paired_via_device_id,
                link_display_name,
                now_ms,
            )
            .await
        }
    }
}

pub(crate) async fn upsert_link_with_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    link_display_name: Option<&str>,
    now_ms: i64,
) -> Result<PairRow, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let pair_id = Uuid::new_v4().to_string();
    let host_s = host_device_id.to_string();
    let via_s = paired_via_device_id.to_string();
    let row = sqlx::query_as::<_, PairRowTuple>(
        r#"
        INSERT INTO host_links
            (pair_id, account_id, host_installation_id, linked_via_installation_id,
             link_display_name, acl_json, paired_at_ms)
        VALUES (?, ?, ?, ?, ?, '{}', ?)
        ON CONFLICT (host_installation_id) DO UPDATE SET
            linked_via_installation_id = excluded.linked_via_installation_id,
            link_display_name = excluded.link_display_name,
            paired_at_ms = excluded.paired_at_ms
        WHERE host_links.account_id = excluded.account_id
        RETURNING pair_id, host_installation_id, account_id,
                  linked_via_installation_id, link_display_name, paired_at_ms
        "#,
    )
    .bind(&pair_id)
    .bind(mobile_account_id)
    .bind(&host_s)
    .bind(&via_s)
    .bind(link_display_name)
    .bind(now_ms)
    .fetch_optional(executor)
    .await
    .map_err(|e| map_host_link_write_error("host_links::upsert_link", e))?;
    let Some(row) = row else {
        return Err(BackendError::HostLinkedElsewhere {
            host_installation_id: host_s,
        });
    };
    decode_pair_row(row)
}

pub(crate) async fn upsert_link_with_postgres_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    link_display_name: Option<&str>,
    now_ms: i64,
) -> Result<PairRow, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let pair_id = Uuid::new_v4().to_string();
    let host_s = host_device_id.to_string();
    let via_s = paired_via_device_id.to_string();
    let row = sqlx::query_as::<_, PairRowTuple>(
        r#"
        INSERT INTO host_links
            (pair_id, account_id, host_installation_id, linked_via_installation_id,
             link_display_name, acl_json, paired_at_ms)
        VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, $6)
        ON CONFLICT (host_installation_id) DO UPDATE SET
            linked_via_installation_id = EXCLUDED.linked_via_installation_id,
            link_display_name = EXCLUDED.link_display_name,
            paired_at_ms = EXCLUDED.paired_at_ms
        WHERE host_links.account_id = EXCLUDED.account_id
        RETURNING pair_id, host_installation_id, account_id,
                  linked_via_installation_id, link_display_name, paired_at_ms
        "#,
    )
    .bind(&pair_id)
    .bind(mobile_account_id)
    .bind(&host_s)
    .bind(&via_s)
    .bind(link_display_name)
    .bind(now_ms)
    .fetch_optional(executor)
    .await
    .map_err(|e| map_host_link_write_error("host_links::upsert_link", e))?;
    let Some(row) = row else {
        return Err(BackendError::HostLinkedElsewhere {
            host_installation_id: host_s,
        });
    };
    decode_pair_row(row)
}

/// Inside a write transaction: host free or already owned by `account_id`.
pub(crate) async fn assert_host_available_or_same_account_sqlite<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    account_id: &str,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host_s = host_device_id.to_string();
    let existing: Option<String> = sqlx::query_scalar(
        r#"
        SELECT account_id
        FROM host_links
        WHERE host_installation_id = ?
        LIMIT 1
        "#,
    )
    .bind(&host_s)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::assert_host_available".into(),
        message: e.to_string(),
    })?;
    if let Some(owner) = existing {
        if owner != account_id {
            return Err(BackendError::HostLinkedElsewhere {
                host_installation_id: host_s,
            });
        }
    }
    Ok(())
}

/// Inside a write transaction: host free or already owned by `account_id`.
pub(crate) async fn assert_host_available_or_same_account_postgres<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    account_id: &str,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host_s = host_device_id.to_string();
    let existing: Option<String> = sqlx::query_scalar(
        r#"
        SELECT account_id
        FROM host_links
        WHERE host_installation_id = $1
        LIMIT 1
        FOR UPDATE
        "#,
    )
    .bind(&host_s)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::assert_host_available".into(),
        message: e.to_string(),
    })?;
    if let Some(owner) = existing {
        if owner != account_id {
            return Err(BackendError::HostLinkedElsewhere {
                host_installation_id: host_s,
            });
        }
    }
    Ok(())
}

fn map_host_link_write_error(operation: &str, error: sqlx::Error) -> BackendError {
    if let sqlx::Error::Database(db) = &error {
        if db.is_unique_violation() {
            return BackendError::HostLinkedElsewhere {
                host_installation_id: String::new(),
            };
        }
    }
    BackendError::StoreQuery {
        operation: operation.into(),
        message: error.to_string(),
    }
}

/// Insert a new host link. Returns `Ok(false)` when the same account already
/// owns the host; `Ok(true)` on insert. Different-account ownership →
/// [`BackendError::HostLinkedElsewhere`].
///
/// Runs assert + insert under a write transaction so exclusivity is race-safe.
pub async fn insert_pair(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    now_ms: i64,
) -> Result<bool, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx =
                pool.begin_with("BEGIN IMMEDIATE")
                    .await
                    .map_err(|e| BackendError::StoreQuery {
                        operation: "host_links::insert_pair.begin".into(),
                        message: e.to_string(),
                    })?;
            assert_host_available_or_same_account_sqlite(
                &mut *tx,
                host_device_id,
                mobile_account_id,
            )
            .await?;
            let inserted = insert_pair_with_executor(
                &mut *tx,
                host_device_id,
                mobile_account_id,
                paired_via_device_id,
                now_ms,
            )
            .await?;
            tx.commit().await.map_err(|e| BackendError::StoreQuery {
                operation: "host_links::insert_pair.commit".into(),
                message: e.to_string(),
            })?;
            Ok(inserted)
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool.begin().await.map_err(|e| BackendError::StoreQuery {
                operation: "host_links::insert_pair.begin".into(),
                message: e.to_string(),
            })?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
                .execute(&mut *tx)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "host_links::insert_pair.set_isolation".into(),
                    message: e.to_string(),
                })?;
            assert_host_available_or_same_account_postgres(
                &mut *tx,
                host_device_id,
                mobile_account_id,
            )
            .await?;
            let inserted = insert_pair_with_postgres_executor(
                &mut *tx,
                host_device_id,
                mobile_account_id,
                paired_via_device_id,
                now_ms,
            )
            .await?;
            tx.commit().await.map_err(|e| BackendError::StoreQuery {
                operation: "host_links::insert_pair.commit".into(),
                message: e.to_string(),
            })?;
            Ok(inserted)
        }
    }
}

/// Insert after exclusivity assert in the **same** write transaction.
///
/// Same-account re-insert → `Ok(false)`. Caller must have already run
/// [`assert_host_available_or_same_account_*`].
pub(crate) async fn insert_pair_with_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    now_ms: i64,
) -> Result<bool, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let pair_id = Uuid::new_v4().to_string();
    let host_s = host_device_id.to_string();
    let via_s = paired_via_device_id.to_string();
    let res = sqlx::query(
        r#"
        INSERT INTO host_links
            (pair_id, account_id, host_installation_id, linked_via_installation_id,
             link_display_name, acl_json, paired_at_ms)
        VALUES (?, ?, ?, ?, NULL, '{}', ?)
        ON CONFLICT (host_installation_id) DO NOTHING
        "#,
    )
    .bind(&pair_id)
    .bind(mobile_account_id)
    .bind(&host_s)
    .bind(&via_s)
    .bind(now_ms)
    .execute(executor)
    .await
    .map_err(|e| map_host_link_write_error("host_links::insert_pair", e))?;
    Ok(res.rows_affected() == 1)
}

pub(crate) async fn insert_pair_with_postgres_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    now_ms: i64,
) -> Result<bool, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let pair_id = Uuid::new_v4().to_string();
    let host_s = host_device_id.to_string();
    let via_s = paired_via_device_id.to_string();
    let res = sqlx::query(
        r#"
        INSERT INTO host_links
            (pair_id, account_id, host_installation_id, linked_via_installation_id,
             link_display_name, acl_json, paired_at_ms)
        VALUES ($1, $2, $3, $4, NULL, '{}'::jsonb, $5)
        ON CONFLICT (host_installation_id) DO NOTHING
        "#,
    )
    .bind(&pair_id)
    .bind(mobile_account_id)
    .bind(&host_s)
    .bind(&via_s)
    .bind(now_ms)
    .execute(executor)
    .await
    .map_err(|e| map_host_link_write_error("host_links::insert_pair", e))?;
    Ok(res.rows_affected() == 1)
}

/// Return every host linked to the given account, most-recent first.
pub async fn list_hosts_for_account(
    store: &impl AsStorePool,
    mobile_account_id: &str,
) -> Result<Vec<PairRow>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, PairRowTuple>(
                r#"
                SELECT pair_id, host_installation_id, account_id,
                       linked_via_installation_id, link_display_name, paired_at_ms
                FROM host_links
                WHERE account_id = ?
                ORDER BY paired_at_ms DESC
                "#,
            )
            .bind(mobile_account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, PairRowTuple>(
                r#"
                SELECT pair_id, host_installation_id, account_id,
                       linked_via_installation_id, link_display_name, paired_at_ms
                FROM host_links
                WHERE account_id = $1
                ORDER BY paired_at_ms DESC
                "#,
            )
            .bind(mobile_account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::list_hosts_for_account".into(),
        message: e.to_string(),
    })?;
    rows.into_iter().map(decode_pair_row).collect()
}

/// Return every account linked to the given host, most-recent first.
pub async fn list_accounts_for_host(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
) -> Result<Vec<PairRow>, BackendError> {
    let host_s = host_device_id.to_string();
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, PairRowTuple>(
                r#"
                SELECT pair_id, host_installation_id, account_id,
                       linked_via_installation_id, link_display_name, paired_at_ms
                FROM host_links
                WHERE host_installation_id = ?
                ORDER BY paired_at_ms DESC
                "#,
            )
            .bind(&host_s)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, PairRowTuple>(
                r#"
                SELECT pair_id, host_installation_id, account_id,
                       linked_via_installation_id, link_display_name, paired_at_ms
                FROM host_links
                WHERE host_installation_id = $1
                ORDER BY paired_at_ms DESC
                "#,
            )
            .bind(&host_s)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::list_accounts_for_host".into(),
        message: e.to_string(),
    })?;
    rows.into_iter().map(decode_pair_row).collect()
}

/// Flatten linked accounts → client installations for host fan-out.
pub async fn list_account_client_targets_for_host(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
) -> Result<Vec<DeviceId>, BackendError> {
    let host_s = host_device_id.to_string();
    let mobile = DeviceRole::MobileClient.to_installation_kind();
    let browser = DeviceRole::BrowserAdmin.to_installation_kind();
    let desktop = DeviceRole::DesktopConsole.to_installation_kind();
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, String>(
                r#"
                SELECT DISTINCT d.installation_id
                FROM host_links hl
                JOIN device_installations d
                  ON d.account_id = hl.account_id
                WHERE hl.host_installation_id = ?
                  AND d.kind IN (?, ?, ?)
                ORDER BY d.installation_id ASC
                "#,
            )
            .bind(&host_s)
            .bind(mobile)
            .bind(browser)
            .bind(desktop)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, String>(
                r#"
                SELECT DISTINCT d.installation_id
                FROM host_links hl
                JOIN device_installations d
                  ON d.account_id = hl.account_id
                WHERE hl.host_installation_id = $1
                  AND d.kind IN (
                      $2::installation_kind,
                      $3::installation_kind,
                      $4::installation_kind
                  )
                ORDER BY d.installation_id ASC
                "#,
            )
            .bind(&host_s)
            .bind(mobile)
            .bind(browser)
            .bind(desktop)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::list_account_client_targets_for_host".into(),
        message: e.to_string(),
    })?;
    rows.into_iter()
        .map(|raw| parse_device_id(&raw, "installation_id"))
        .collect()
}

/// Load one (host, account) link row when present.
pub async fn get_pair_with_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<Option<PairRow>, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host_s = host_device_id.to_string();
    let row = sqlx::query_as::<_, PairRowTuple>(
        r#"
        SELECT pair_id, host_installation_id, account_id,
               linked_via_installation_id, link_display_name, paired_at_ms
        FROM host_links
        WHERE host_installation_id = ? AND account_id = ?
        "#,
    )
    .bind(&host_s)
    .bind(mobile_account_id)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::get_pair".into(),
        message: e.to_string(),
    })?;
    row.map(decode_pair_row).transpose()
}

pub async fn get_pair_with_postgres_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<Option<PairRow>, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host_s = host_device_id.to_string();
    let row = sqlx::query_as::<_, PairRowTuple>(
        r#"
        SELECT pair_id, host_installation_id, account_id,
               linked_via_installation_id, link_display_name, paired_at_ms
        FROM host_links
        WHERE host_installation_id = $1 AND account_id = $2
        "#,
    )
    .bind(&host_s)
    .bind(mobile_account_id)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::get_pair".into(),
        message: e.to_string(),
    })?;
    row.map(decode_pair_row).transpose()
}

/// Does the (host, account) link exist?
pub async fn exists(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<bool, BackendError> {
    let host_s = host_device_id.to_string();
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, String>(
                r#"
                SELECT pair_id
                FROM host_links
                WHERE host_installation_id = ? AND account_id = ?
                LIMIT 1
                "#,
            )
            .bind(&host_s)
            .bind(mobile_account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, String>(
                r#"
                SELECT pair_id
                FROM host_links
                WHERE host_installation_id = $1 AND account_id = $2
                LIMIT 1
                "#,
            )
            .bind(&host_s)
            .bind(mobile_account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::exists".into(),
        message: e.to_string(),
    })?;
    Ok(row.is_some())
}

/// Delete a specific (host, account) link. Returns rows-deleted (0 or 1).
pub async fn delete_pair(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<u64, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            delete_pair_with_executor(pool, host_device_id, mobile_account_id).await
        }
        StorePoolRef::Postgres(pool) => {
            delete_pair_with_postgres_executor(pool, host_device_id, mobile_account_id).await
        }
    }
}

pub(crate) async fn delete_pair_with_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host_s = host_device_id.to_string();
    let res = sqlx::query(
        r#"
        DELETE FROM host_links
        WHERE host_installation_id = ? AND account_id = ?
        "#,
    )
    .bind(&host_s)
    .bind(mobile_account_id)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::delete_pair".into(),
        message: e.to_string(),
    })?;
    Ok(res.rows_affected())
}

pub(crate) async fn delete_pair_with_postgres_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<u64, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host_s = host_device_id.to_string();
    let res = sqlx::query(
        r#"
        DELETE FROM host_links
        WHERE host_installation_id = $1 AND account_id = $2
        "#,
    )
    .bind(&host_s)
    .bind(mobile_account_id)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_links::delete_pair".into(),
        message: e.to_string(),
    })?;
    Ok(res.rows_affected())
}

fn decode_pair_row(row: PairRowTuple) -> Result<PairRow, BackendError> {
    let (
        pair_id,
        host_device_id,
        mobile_account_id,
        paired_via_device_id,
        link_display_name,
        paired_at_ms,
    ) = row;
    Ok(PairRow {
        pair_id,
        host_device_id: parse_device_id(&host_device_id, "host_installation_id")?,
        mobile_account_id,
        paired_via_device_id: parse_device_id(&paired_via_device_id, "linked_via_installation_id")?,
        link_display_name,
        paired_at_ms,
    })
}

fn parse_device_id(raw: &str, column: &str) -> Result<DeviceId, BackendError> {
    Uuid::parse_str(raw)
        .map(DeviceId)
        .map_err(|e| BackendError::StoreDecode {
            column: format!("host_links.{column}"),
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::device_installations::{insert_device, set_account_id};
    use crate::store::test_support::{insert_account, insert_ios_device, memory_pool, T0};
    use pretty_assertions::assert_eq;

    async fn setup_one_host_one_account() -> (
        sqlx::SqlitePool,
        String,   // account_id
        DeviceId, // host_device_id
        DeviceId, // mobile_device_id
    ) {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "user@example.com").await;
        let host = DeviceId::new();
        insert_device(&pool, host, "Mac-mini", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        let mobile = insert_ios_device(&pool, &account_id).await;
        (pool, account_id, host, mobile)
    }

    #[tokio::test]
    async fn insert_and_list_round_trip() {
        let (pool, account, host, mobile) = setup_one_host_one_account().await;
        let inserted = insert_pair(&pool, host, &account, mobile, 100)
            .await
            .unwrap();
        assert!(inserted);
        let hosts = list_hosts_for_account(&pool, &account).await.unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].host_device_id, host);
        assert_eq!(hosts[0].paired_via_device_id, mobile);
        assert_eq!(hosts[0].mobile_account_id, account);
        assert_eq!(hosts[0].paired_at_ms, 100);
    }

    #[tokio::test]
    async fn unique_violation_returns_false() {
        let (pool, account, host, mobile) = setup_one_host_one_account().await;
        assert!(insert_pair(&pool, host, &account, mobile, 100)
            .await
            .unwrap());
        assert!(!insert_pair(&pool, host, &account, mobile, 200)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn delete_pair_removes_row() {
        let (pool, account, host, mobile) = setup_one_host_one_account().await;
        insert_pair(&pool, host, &account, mobile, 100)
            .await
            .unwrap();
        let n = delete_pair(&pool, host, &account).await.unwrap();
        assert_eq!(n, 1);
        assert!(!exists(&pool, host, &account).await.unwrap());
    }

    #[tokio::test]
    async fn delete_pair_on_missing_returns_zero() {
        let (pool, account, host, _mobile) = setup_one_host_one_account().await;
        let n = delete_pair(&pool, host, &account).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn one_mac_rejects_second_account() {
        let (pool, account_a, host, mobile_a) = setup_one_host_one_account().await;
        let account_b = insert_account(&pool, "b@example.com").await;
        let mobile_b = insert_ios_device(&pool, &account_b).await;
        insert_pair(&pool, host, &account_a, mobile_a, 100)
            .await
            .unwrap();
        let err = insert_pair(&pool, host, &account_b, mobile_b, 200)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::HostLinkedElsewhere { .. }));
        let accounts = list_accounts_for_host(&pool, host).await.unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].mobile_account_id, account_a);
    }

    #[tokio::test]
    async fn exclusivity_assert_inside_tx_blocks_elsewhere() {
        let (pool, account_a, host, mobile_a) = setup_one_host_one_account().await;
        let account_b = insert_account(&pool, "tx-b@example.com").await;
        insert_pair(&pool, host, &account_a, mobile_a, 100)
            .await
            .unwrap();

        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let err = assert_host_available_or_same_account_sqlite(&mut *tx, host, &account_b)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::HostLinkedElsewhere { .. }));
        // Same account is fine inside the TX.
        assert_host_available_or_same_account_sqlite(&mut *tx, host, &account_a)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn exists_returns_false_when_missing() {
        let (pool, account, host, _mobile) = setup_one_host_one_account().await;
        assert!(!exists(&pool, host, &account).await.unwrap());
    }

    #[tokio::test]
    async fn list_account_client_targets_for_host_flattens_joined_targets() {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "joined-targets@example.com").await;
        let host = DeviceId::new();
        insert_device(&pool, host, "Mac-mini", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        let mobile = insert_ios_device(&pool, &account_id).await;
        let browser = DeviceId::new();
        insert_device(&pool, browser, "browser", DeviceRole::BrowserAdmin, T0)
            .await
            .unwrap();
        set_account_id(&pool, &browser, &account_id).await.unwrap();
        insert_pair(&pool, host, &account_id, mobile, T0)
            .await
            .unwrap();

        let targets = list_account_client_targets_for_host(&pool, host)
            .await
            .unwrap();

        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&mobile));
        assert!(targets.contains(&browser));
    }
}
