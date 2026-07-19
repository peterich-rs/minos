//! Persistence for `account_host_pairings`. Pair model is
//! `(host_device_id, mobile_account_id)` post ADR-0020. The mobile
//! `device_id` that performed the scan is recorded as
//! `paired_via_device_id` for audit only — it does not participate in
//! routing.
//!
//! ## Type strategy
//!
//! Same as `store::devices` and `store::accounts`: we store
//! `DeviceId` as `TEXT` (UUID-string form) and parse on the way back
//! using `Uuid::parse_str`. `mobile_account_id` rides as a plain
//! `String` because the codebase treats account ids as opaque UUID
//! strings (see `accounts::AccountRow::account_id: String`); there is
//! no `AccountId` newtype yet.

use minos_domain::DeviceId;
use sqlx::{Executor, Postgres, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

type PairRowTuple = (String, String, String, String, i64);

/// A single row of the `account_host_pairings` table after decoding the
/// stringly-typed columns back into the domain `DeviceId` newtypes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRow {
    pub pair_id: String,
    pub host_device_id: DeviceId,
    pub mobile_account_id: String,
    /// The mobile device that scanned the pairing QR. Recorded for
    /// audit only; routing keys off `host_device_id` and account.
    pub paired_via_device_id: DeviceId,
    pub paired_at_ms: i64,
}

/// Insert a new pair. Returns `Ok(false)` on UNIQUE conflict
/// (account already paired to this Mac); `Ok(true)` on insert.
///
/// The `ON CONFLICT DO NOTHING` clause makes the call idempotent for
/// the (host, account) couple while still letting the caller
/// distinguish "newly created" from "already present" via the bool
/// return — used by the pairing handler to decide whether to emit the
/// `Paired` event.
pub async fn insert_pair(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    now_ms: i64,
) -> Result<bool, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            insert_pair_with_executor(
                pool,
                host_device_id,
                mobile_account_id,
                paired_via_device_id,
                now_ms,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            insert_pair_with_postgres_executor(
                pool,
                host_device_id,
                mobile_account_id,
                paired_via_device_id,
                now_ms,
            )
            .await
        }
    }
}

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
        INSERT INTO account_host_pairings
            (pair_id, host_device_id, mobile_account_id, paired_via_device_id, paired_at_ms)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT (host_device_id, mobile_account_id) DO NOTHING
        "#,
    )
    .bind(&pair_id)
    .bind(&host_s)
    .bind(mobile_account_id)
    .bind(&via_s)
    .bind(now_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::insert_pair".into(),
        message: e.to_string(),
    })?;
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
        INSERT INTO account_host_pairings
            (pair_id, host_device_id, mobile_account_id, paired_via_device_id, paired_at_ms)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (host_device_id, mobile_account_id) DO NOTHING
        "#,
    )
    .bind(&pair_id)
    .bind(&host_s)
    .bind(mobile_account_id)
    .bind(&via_s)
    .bind(now_ms)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::insert_pair".into(),
        message: e.to_string(),
    })?;
    Ok(res.rows_affected() == 1)
}

/// Return every Mac paired to the given account, ordered most-recent
/// first by `paired_at_ms`.
pub async fn list_hosts_for_account(
    store: &impl AsStorePool,
    mobile_account_id: &str,
) -> Result<Vec<PairRow>, BackendError> {
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, PairRowTuple>(
                r#"
                SELECT pair_id, host_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
                FROM account_host_pairings
                WHERE mobile_account_id = ?
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
                SELECT pair_id, host_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
                FROM account_host_pairings
                WHERE mobile_account_id = $1
                ORDER BY paired_at_ms DESC
                "#,
            )
            .bind(mobile_account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::list_hosts_for_account".into(),
        message: e.to_string(),
    })?;
    rows.into_iter().map(decode_pair_row).collect()
}

/// Return every account paired to the given Mac, ordered most-recent
/// first by `paired_at_ms`.
pub async fn list_accounts_for_host(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
) -> Result<Vec<PairRow>, BackendError> {
    let host_s = host_device_id.to_string();
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, PairRowTuple>(
                r#"
                SELECT pair_id, host_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
                FROM account_host_pairings
                WHERE host_device_id = ?
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
                SELECT pair_id, host_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
                FROM account_host_pairings
                WHERE host_device_id = $1
                ORDER BY paired_at_ms DESC
                "#,
            )
            .bind(&host_s)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::list_accounts_for_host".into(),
        message: e.to_string(),
    })?;
    rows.into_iter().map(decode_pair_row).collect()
}

pub async fn list_account_client_targets_for_host(
    store: &impl AsStorePool,
    host_device_id: DeviceId,
) -> Result<Vec<DeviceId>, BackendError> {
    let host_s = host_device_id.to_string();
    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, String>(
                r#"
                SELECT DISTINCT d.device_id
                FROM account_host_pairings ahp
                JOIN devices d
                  ON d.account_id = ahp.mobile_account_id
                WHERE ahp.host_device_id = ?
                  AND d.role IN ('mobile-client', 'browser-admin')
                ORDER BY d.device_id ASC
                "#,
            )
            .bind(&host_s)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, String>(
                r#"
                SELECT DISTINCT d.device_id
                FROM account_host_pairings ahp
                JOIN devices d
                  ON d.account_id = ahp.mobile_account_id
                WHERE ahp.host_device_id = $1
                  AND d.role IN ('mobile-client', 'browser-admin')
                ORDER BY d.device_id ASC
                "#,
            )
            .bind(&host_s)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::list_account_client_targets_for_host".into(),
        message: e.to_string(),
    })?;
    rows.into_iter()
        .map(|raw| parse_device_id(&raw, "device_id"))
        .collect()
}

/// Does the (host, account) pair exist?
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
                FROM account_host_pairings
                WHERE host_device_id = ? AND mobile_account_id = ?
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
                FROM account_host_pairings
                WHERE host_device_id = $1 AND mobile_account_id = $2
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
        operation: "account_host_pairings::exists".into(),
        message: e.to_string(),
    })?;
    Ok(row.is_some())
}

/// Delete a specific (host, account) pair. Returns rows-deleted (0 or 1).
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
        DELETE FROM account_host_pairings
        WHERE host_device_id = ? AND mobile_account_id = ?
        "#,
    )
    .bind(&host_s)
    .bind(mobile_account_id)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::delete_pair".into(),
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
        DELETE FROM account_host_pairings
        WHERE host_device_id = $1 AND mobile_account_id = $2
        "#,
    )
    .bind(&host_s)
    .bind(mobile_account_id)
    .execute(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::delete_pair".into(),
        message: e.to_string(),
    })?;
    Ok(res.rows_affected())
}

pub(crate) async fn count_accounts_for_host_with_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
) -> Result<i64, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let host_s = host_device_id.to_string();
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM account_host_pairings
        WHERE host_device_id = ?
        "#,
    )
    .bind(&host_s)
    .fetch_one(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::count_accounts_for_host".into(),
        message: e.to_string(),
    })
}

pub(crate) async fn count_accounts_for_host_with_postgres_executor<'e, E>(
    executor: E,
    host_device_id: DeviceId,
) -> Result<i64, BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let host_s = host_device_id.to_string();
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM account_host_pairings
        WHERE host_device_id = $1
        "#,
    )
    .bind(&host_s)
    .fetch_one(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_host_pairings::count_accounts_for_host".into(),
        message: e.to_string(),
    })
}

fn decode_pair_row(row: PairRowTuple) -> Result<PairRow, BackendError> {
    let (pair_id, host_device_id, mobile_account_id, paired_via_device_id, paired_at_ms) = row;
    Ok(PairRow {
        pair_id,
        host_device_id: parse_device_id(&host_device_id, "host_device_id")?,
        mobile_account_id,
        paired_via_device_id: parse_device_id(&paired_via_device_id, "paired_via_device_id")?,
        paired_at_ms,
    })
}

fn parse_device_id(raw: &str, column: &str) -> Result<DeviceId, BackendError> {
    Uuid::parse_str(raw)
        .map(DeviceId)
        .map_err(|e| BackendError::StoreDecode {
            column: format!("account_host_pairings.{column}"),
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::devices::{insert_device, set_account_id};
    use crate::store::test_support::{insert_account, insert_ios_device, memory_pool, T0};
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;

    /// Set up a Mac, a mobile account + iOS device, and return the ids.
    /// Mac is inserted via `insert_device` directly (no account_id link
    /// pre-pair); iOS is inserted via `insert_ios_device` which sets
    /// `account_id` during creation. `secret_hash` stays NULL on iOS as
    /// required by the new ADR-0020 rail.
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
    async fn one_mac_to_many_accounts() {
        let (pool, account_a, host, mobile_a) = setup_one_host_one_account().await;
        let account_b = insert_account(&pool, "b@example.com").await;
        let mobile_b = insert_ios_device(&pool, &account_b).await;
        insert_pair(&pool, host, &account_a, mobile_a, 100)
            .await
            .unwrap();
        insert_pair(&pool, host, &account_b, mobile_b, 200)
            .await
            .unwrap();
        let accounts = list_accounts_for_host(&pool, host).await.unwrap();
        assert_eq!(accounts.len(), 2);
        // ordered most-recent first
        assert_eq!(accounts[0].mobile_account_id, account_b);
        assert_eq!(accounts[1].mobile_account_id, account_a);
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
