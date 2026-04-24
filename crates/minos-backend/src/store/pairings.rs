//! `pairings` table CRUD.
//!
//! The pairing relation is undirected: "A paired with B" is the same row
//! as "B paired with A". The schema (migration 0002) enforces uniqueness by
//! storing `(device_a, device_b)` with the `CHECK (device_a < device_b)`
//! constraint. SQLite compares the TEXT UUID representations
//! lexicographically, so we canonicalize on the Rust side with the same
//! stringwise ordering.

use minos_domain::DeviceId;
use sqlx::{Executor, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::error::RelayError;

/// SQLite trigger message used when a device is already present in some
/// other pairing row.
pub(crate) const SINGLE_PAIR_VIOLATION_MARKER: &str = "pairings_device_already_paired";

/// Order `(a, b)` so `first < second` using the same string comparison
/// SQLite applies to the CHECK constraint. The inputs must differ; callers
/// should never pair a device with itself.
fn canonical(a: DeviceId, b: DeviceId) -> (String, String) {
    let a_s = a.to_string();
    let b_s = b.to_string();
    if a_s < b_s {
        (a_s, b_s)
    } else {
        (b_s, a_s)
    }
}

/// Insert a pairing between `a` and `b`.
///
/// Canonicalizes the pair so a caller may pass the two device ids in either
/// order. Idempotent: re-inserting an existing pair is a silent no-op via
/// `ON CONFLICT DO NOTHING`. Any different row that reuses either device is
/// rejected by the migration-installed trigger with
/// [`SINGLE_PAIR_VIOLATION_MARKER`]. Pairing a device with itself is rejected
/// by the `CHECK (device_a < device_b)` constraint and surfaces as a
/// `StoreQuery` error.
pub async fn insert_pairing(
    pool: &SqlitePool,
    a: DeviceId,
    b: DeviceId,
    now: i64,
) -> Result<(), RelayError> {
    insert_pairing_with_executor(pool, a, b, now).await
}

pub(crate) async fn insert_pairing_with_executor<'e, E>(
    executor: E,
    a: DeviceId,
    b: DeviceId,
    now: i64,
) -> Result<(), RelayError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let (lo, hi) = canonical(a, b);

    sqlx::query!(
        r#"
        INSERT INTO pairings (device_a, device_b, created_at)
        VALUES (?, ?, ?)
        ON CONFLICT(device_a, device_b) DO NOTHING
        "#,
        lo,
        hi,
        now,
    )
    .execute(executor)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "insert_pairing".to_string(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Look up the peer device paired with `id`.
///
/// Returns `Ok(None)` if `id` has no pairing row. In the MVP a device has
/// at most one pair (spec §7.2), so any row that touches `id` names the
/// peer in its other column.
pub async fn get_pair(pool: &SqlitePool, id: DeviceId) -> Result<Option<DeviceId>, RelayError> {
    get_pair_with_executor(pool, id).await
}

pub(crate) async fn get_pair_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
) -> Result<Option<DeviceId>, RelayError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();

    let row = sqlx::query!(
        r#"
        SELECT device_a, device_b
        FROM pairings
        WHERE device_a = ? OR device_b = ?
        LIMIT 1
        "#,
        id_str,
        id_str,
    )
    .fetch_optional(executor)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "get_pair".to_string(),
        message: e.to_string(),
    })?;

    let Some(r) = row else {
        return Ok(None);
    };

    let peer_str = if r.device_a == id_str {
        r.device_b
    } else {
        r.device_a
    };

    let peer = Uuid::parse_str(&peer_str)
        .map(DeviceId)
        .map_err(|e| RelayError::StoreDecode {
            column: "pairings.device_a/b".to_string(),
            message: e.to_string(),
        })?;
    Ok(Some(peer))
}

/// Delete the pairing between `a` and `b` (in either order). No-op if no
/// such row exists; callers that need strict semantics can check via
/// [`get_pair`] first.
pub async fn delete_pair(pool: &SqlitePool, a: DeviceId, b: DeviceId) -> Result<(), RelayError> {
    delete_pair_with_executor(pool, a, b).await
}

pub(crate) async fn delete_pair_with_executor<'e, E>(
    executor: E,
    a: DeviceId,
    b: DeviceId,
) -> Result<(), RelayError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let (lo, hi) = canonical(a, b);

    sqlx::query!(
        r#"DELETE FROM pairings WHERE device_a = ? AND device_b = ?"#,
        lo,
        hi
    )
    .execute(executor)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "delete_pair".to_string(),
        message: e.to_string(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::devices::insert_device;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;

    /// Insert two devices + return their ids, already ordered so `(low, high)`
    /// matches SQLite's string ordering. Handy for tests that want a fresh
    /// paired set without caring about the ID assignment.
    async fn two_devices(pool: &SqlitePool) -> (DeviceId, DeviceId) {
        let a = DeviceId::new();
        let b = DeviceId::new();
        insert_device(pool, a, "mac", DeviceRole::MacHost, T0)
            .await
            .unwrap();
        insert_device(pool, b, "ios", DeviceRole::IosClient, T0)
            .await
            .unwrap();
        (a, b)
    }

    #[tokio::test]
    async fn insert_pairing_canonicalizes_rows_regardless_of_arg_order() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;

        // Insert in (a, b) order. If a > b stringwise, the row should still
        // land with device_a < device_b.
        insert_pairing(&pool, a, b, T0).await.unwrap();

        let row = sqlx::query!("SELECT device_a, device_b FROM pairings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(row.device_a < row.device_b);
    }

    #[tokio::test]
    async fn insert_pairing_in_reverse_arg_order_produces_identical_row() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;

        insert_pairing(&pool, b, a, T0).await.unwrap(); // flipped
        insert_pairing(&pool, a, b, T0).await.unwrap(); // same pair, ON CONFLICT DO NOTHING

        let count: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM pairings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "repeat insert must be idempotent");
    }

    #[tokio::test]
    async fn insert_pairing_rejects_second_distinct_pair_for_same_device() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;
        let c = DeviceId::new();
        insert_device(&pool, c, "ios-2", DeviceRole::IosClient, T0)
            .await
            .unwrap();

        insert_pairing(&pool, a, b, T0).await.unwrap();

        let err = insert_pairing(&pool, a, c, T0 + 1).await.unwrap_err();
        match err {
            RelayError::StoreQuery { operation, message } => {
                assert_eq!(operation, "insert_pairing");
                assert!(message.contains(SINGLE_PAIR_VIOLATION_MARKER));
            }
            other => panic!("expected StoreQuery, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_pair_returns_peer_from_either_side() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;
        insert_pairing(&pool, a, b, T0).await.unwrap();

        assert_eq!(get_pair(&pool, a).await.unwrap(), Some(b));
        assert_eq!(get_pair(&pool, b).await.unwrap(), Some(a));
    }

    #[tokio::test]
    async fn get_pair_on_unpaired_device_returns_none() {
        let pool = memory_pool().await;
        let (a, _b) = two_devices(&pool).await;
        assert_eq!(get_pair(&pool, a).await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_pair_removes_row_and_followup_get_is_none() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;
        insert_pairing(&pool, a, b, T0).await.unwrap();
        assert_eq!(get_pair(&pool, a).await.unwrap(), Some(b));

        delete_pair(&pool, a, b).await.unwrap();
        assert_eq!(get_pair(&pool, a).await.unwrap(), None);
        assert_eq!(get_pair(&pool, b).await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_pair_on_nonexistent_pair_is_silent_ok() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;
        // Never inserted — must not error.
        delete_pair(&pool, a, b).await.unwrap();
        delete_pair(&pool, b, a).await.unwrap(); // reversed order too
    }
}
