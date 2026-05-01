//! Persistence for `account_mac_pairings`. Pair model is
//! `(mac_device_id, mobile_account_id)` post ADR-0020. The mobile
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
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::BackendError;

/// A single row of the `account_mac_pairings` table after decoding the
/// stringly-typed columns back into the domain `DeviceId` newtypes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRow {
    pub pair_id: String,
    pub mac_device_id: DeviceId,
    pub mobile_account_id: String,
    /// The mobile device that scanned the pairing QR. Recorded for
    /// audit only; routing keys off `mac_device_id` and account.
    pub paired_via_device_id: DeviceId,
    pub paired_at_ms: i64,
}

/// Insert a new pair. Returns `Ok(false)` on UNIQUE conflict
/// (account already paired to this Mac); `Ok(true)` on insert.
///
/// The `ON CONFLICT DO NOTHING` clause makes the call idempotent for
/// the (mac, account) couple while still letting the caller
/// distinguish "newly created" from "already present" via the bool
/// return — used by the pairing handler to decide whether to emit the
/// `Paired` event.
pub async fn insert_pair(
    pool: &SqlitePool,
    mac_device_id: DeviceId,
    mobile_account_id: &str,
    paired_via_device_id: DeviceId,
    now_ms: i64,
) -> Result<bool, BackendError> {
    let pair_id = Uuid::new_v4().to_string();
    let mac_s = mac_device_id.to_string();
    let via_s = paired_via_device_id.to_string();
    let res = sqlx::query!(
        r#"
        INSERT INTO account_mac_pairings
            (pair_id, mac_device_id, mobile_account_id, paired_via_device_id, paired_at_ms)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT (mac_device_id, mobile_account_id) DO NOTHING
        "#,
        pair_id,
        mac_s,
        mobile_account_id,
        via_s,
        now_ms,
    )
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_mac_pairings::insert_pair".into(),
        message: e.to_string(),
    })?;
    Ok(res.rows_affected() == 1)
}

/// Return every Mac paired to the given account, ordered most-recent
/// first by `paired_at_ms`.
pub async fn list_macs_for_account(
    pool: &SqlitePool,
    mobile_account_id: &str,
) -> Result<Vec<PairRow>, BackendError> {
    let rows = sqlx::query!(
        r#"
        SELECT pair_id, mac_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
        FROM account_mac_pairings
        WHERE mobile_account_id = ?
        ORDER BY paired_at_ms DESC
        "#,
        mobile_account_id,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_mac_pairings::list_macs_for_account".into(),
        message: e.to_string(),
    })?;
    rows.into_iter()
        .map(|r| {
            Ok(PairRow {
                pair_id: r.pair_id,
                mac_device_id: parse_device_id(&r.mac_device_id, "mac_device_id")?,
                mobile_account_id: r.mobile_account_id,
                paired_via_device_id: parse_device_id(
                    &r.paired_via_device_id,
                    "paired_via_device_id",
                )?,
                paired_at_ms: r.paired_at_ms,
            })
        })
        .collect()
}

/// Return every account paired to the given Mac, ordered most-recent
/// first by `paired_at_ms`.
pub async fn list_accounts_for_mac(
    pool: &SqlitePool,
    mac_device_id: DeviceId,
) -> Result<Vec<PairRow>, BackendError> {
    let mac_s = mac_device_id.to_string();
    let rows = sqlx::query!(
        r#"
        SELECT pair_id, mac_device_id, mobile_account_id, paired_via_device_id, paired_at_ms
        FROM account_mac_pairings
        WHERE mac_device_id = ?
        ORDER BY paired_at_ms DESC
        "#,
        mac_s,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_mac_pairings::list_accounts_for_mac".into(),
        message: e.to_string(),
    })?;
    rows.into_iter()
        .map(|r| {
            Ok(PairRow {
                pair_id: r.pair_id,
                mac_device_id: parse_device_id(&r.mac_device_id, "mac_device_id")?,
                mobile_account_id: r.mobile_account_id,
                paired_via_device_id: parse_device_id(
                    &r.paired_via_device_id,
                    "paired_via_device_id",
                )?,
                paired_at_ms: r.paired_at_ms,
            })
        })
        .collect()
}

/// Does the (mac, account) pair exist?
pub async fn exists(
    pool: &SqlitePool,
    mac_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<bool, BackendError> {
    let mac_s = mac_device_id.to_string();
    // `SELECT 1` would type-infer to `()` under the `sqlx::query!` macro
    // because sqlite reports `INTEGER` literals as untyped. Selecting
    // `pair_id` instead picks up the column's `TEXT NOT NULL` annotation
    // and gives the macro an `Option<String>` row to work with.
    let row = sqlx::query!(
        r#"
        SELECT pair_id
        FROM account_mac_pairings
        WHERE mac_device_id = ? AND mobile_account_id = ?
        LIMIT 1
        "#,
        mac_s,
        mobile_account_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_mac_pairings::exists".into(),
        message: e.to_string(),
    })?;
    Ok(row.is_some())
}

/// Delete a specific (mac, account) pair. Returns rows-deleted (0 or 1).
pub async fn delete_pair(
    pool: &SqlitePool,
    mac_device_id: DeviceId,
    mobile_account_id: &str,
) -> Result<u64, BackendError> {
    let mac_s = mac_device_id.to_string();
    let res = sqlx::query!(
        r#"
        DELETE FROM account_mac_pairings
        WHERE mac_device_id = ? AND mobile_account_id = ?
        "#,
        mac_s,
        mobile_account_id,
    )
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "account_mac_pairings::delete_pair".into(),
        message: e.to_string(),
    })?;
    Ok(res.rows_affected())
}

fn parse_device_id(raw: &str, column: &str) -> Result<DeviceId, BackendError> {
    Uuid::parse_str(raw)
        .map(DeviceId)
        .map_err(|e| BackendError::StoreDecode {
            column: format!("account_mac_pairings.{column}"),
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::devices::insert_device;
    use crate::store::test_support::{insert_account, insert_ios_device, memory_pool, T0};
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;

    /// Set up a Mac, a mobile account + iOS device, and return the ids.
    /// Mac is inserted via `insert_device` directly (no account_id link
    /// pre-pair); iOS is inserted via `insert_ios_device` which sets
    /// `account_id` during creation. `secret_hash` stays NULL on iOS as
    /// required by the new ADR-0020 rail.
    async fn setup_one_mac_one_account() -> (
        sqlx::SqlitePool,
        String,   // account_id
        DeviceId, // mac_device_id
        DeviceId, // mobile_device_id
    ) {
        let pool = memory_pool().await;
        let account_id = insert_account(&pool, "user@example.com").await;
        let mac = DeviceId::new();
        insert_device(&pool, mac, "Mac-mini", DeviceRole::AgentHost, T0)
            .await
            .unwrap();
        let mobile = insert_ios_device(&pool, &account_id).await;
        (pool, account_id, mac, mobile)
    }

    #[tokio::test]
    async fn insert_and_list_round_trip() {
        let (pool, account, mac, mobile) = setup_one_mac_one_account().await;
        let inserted = insert_pair(&pool, mac, &account, mobile, 100)
            .await
            .unwrap();
        assert!(inserted);
        let macs = list_macs_for_account(&pool, &account).await.unwrap();
        assert_eq!(macs.len(), 1);
        assert_eq!(macs[0].mac_device_id, mac);
        assert_eq!(macs[0].paired_via_device_id, mobile);
        assert_eq!(macs[0].mobile_account_id, account);
        assert_eq!(macs[0].paired_at_ms, 100);
    }

    #[tokio::test]
    async fn unique_violation_returns_false() {
        let (pool, account, mac, mobile) = setup_one_mac_one_account().await;
        assert!(insert_pair(&pool, mac, &account, mobile, 100).await.unwrap());
        assert!(
            !insert_pair(&pool, mac, &account, mobile, 200).await.unwrap()
        );
    }

    #[tokio::test]
    async fn delete_pair_removes_row() {
        let (pool, account, mac, mobile) = setup_one_mac_one_account().await;
        insert_pair(&pool, mac, &account, mobile, 100).await.unwrap();
        let n = delete_pair(&pool, mac, &account).await.unwrap();
        assert_eq!(n, 1);
        assert!(!exists(&pool, mac, &account).await.unwrap());
    }

    #[tokio::test]
    async fn delete_pair_on_missing_returns_zero() {
        let (pool, account, mac, _mobile) = setup_one_mac_one_account().await;
        let n = delete_pair(&pool, mac, &account).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn one_mac_to_many_accounts() {
        let (pool, account_a, mac, mobile_a) = setup_one_mac_one_account().await;
        let account_b = insert_account(&pool, "b@example.com").await;
        let mobile_b = insert_ios_device(&pool, &account_b).await;
        insert_pair(&pool, mac, &account_a, mobile_a, 100)
            .await
            .unwrap();
        insert_pair(&pool, mac, &account_b, mobile_b, 200)
            .await
            .unwrap();
        let accounts = list_accounts_for_mac(&pool, mac).await.unwrap();
        assert_eq!(accounts.len(), 2);
        // ordered most-recent first
        assert_eq!(accounts[0].mobile_account_id, account_b);
        assert_eq!(accounts[1].mobile_account_id, account_a);
    }

    #[tokio::test]
    async fn exists_returns_false_when_missing() {
        let (pool, account, mac, _mobile) = setup_one_mac_one_account().await;
        assert!(!exists(&pool, mac, &account).await.unwrap());
    }
}
