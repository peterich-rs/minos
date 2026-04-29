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

use crate::error::BackendError;

/// Legacy SQLite trigger message used by migrations before multi-device
/// pairing. Kept so older DB errors can still be normalized during upgrades.
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
/// `ON CONFLICT DO NOTHING`. Pairing a device with itself is rejected by the
/// `CHECK (device_a < device_b)` constraint and surfaces as a `StoreQuery`
/// error.
pub async fn insert_pairing(
    pool: &SqlitePool,
    a: DeviceId,
    b: DeviceId,
    now: i64,
) -> Result<(), BackendError> {
    insert_pairing_with_executor(pool, a, b, now).await
}

pub(crate) async fn insert_pairing_with_executor<'e, E>(
    executor: E,
    a: DeviceId,
    b: DeviceId,
    now: i64,
) -> Result<(), BackendError>
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
    .map_err(|e| BackendError::StoreQuery {
        operation: "insert_pairing".to_string(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Look up the peer device paired with `id`.
///
/// Returns `Ok(None)` if `id` has no pairing row. If the device has multiple
/// pairings, returns the most recently-created peer. This preserves the old
/// single-peer call sites as an "active/latest peer" fallback while newer
/// multi-device flows use [`get_peers`].
pub async fn get_pair(pool: &SqlitePool, id: DeviceId) -> Result<Option<DeviceId>, BackendError> {
    get_pair_with_executor(pool, id).await
}

/// Look up every peer paired with `id`.
///
/// This differs from [`get_pair`] for agent-host devices: one Mac can now
/// have multiple iOS clients paired, and ingest fan-out must reach all live
/// peers. For single-homed devices the returned vector has at most one item.
pub async fn get_peers(pool: &SqlitePool, id: DeviceId) -> Result<Vec<DeviceId>, BackendError> {
    let id_str = id.to_string();
    let rows = sqlx::query_as::<_, (String, String)>(
        r"
        SELECT device_a, device_b
        FROM pairings
        WHERE device_a = ? OR device_b = ?
        ORDER BY created_at ASC
        ",
    )
    .bind(&id_str)
    .bind(&id_str)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "get_peers".to_string(),
        message: e.to_string(),
    })?;

    rows.into_iter()
        .map(|(device_a, device_b)| {
            let peer_str = if device_a == id_str {
                device_b
            } else {
                device_a
            };
            Uuid::parse_str(&peer_str)
                .map(DeviceId)
                .map_err(|e| BackendError::StoreDecode {
                    column: "pairings.device_a/b".to_string(),
                    message: e.to_string(),
                })
        })
        .collect()
}

/// Look up the peer device paired with `id` together with the pairing's
/// `created_at` timestamp (epoch ms).
///
/// Returns `Ok(None)` when the device has no pairing row, mirroring
/// [`get_pair`]. Used by `/v1/me/peer` to populate `paired_at_ms`
/// without forcing the caller to re-query the row.
pub async fn get_pair_with_created_at(
    pool: &SqlitePool,
    id: DeviceId,
) -> Result<Option<(DeviceId, i64)>, BackendError> {
    let id_str = id.to_string();

    let row = sqlx::query_as::<_, (String, String, i64)>(
        r"
        SELECT device_a, device_b, created_at
        FROM pairings
        WHERE device_a = ? OR device_b = ?
        ORDER BY created_at DESC
        LIMIT 1
        ",
    )
    .bind(&id_str)
    .bind(&id_str)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "get_pair_with_created_at".to_string(),
        message: e.to_string(),
    })?;

    let Some(r) = row else {
        return Ok(None);
    };

    let peer_str = if r.0 == id_str { r.1 } else { r.0 };
    let peer = Uuid::parse_str(&peer_str)
        .map(DeviceId)
        .map_err(|e| BackendError::StoreDecode {
            column: "pairings.device_a/b".to_string(),
            message: e.to_string(),
        })?;
    Ok(Some((peer, r.2)))
}

pub(crate) async fn get_pair_with_executor<'e, E>(
    executor: E,
    id: DeviceId,
) -> Result<Option<DeviceId>, BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id_str = id.to_string();

    let row = sqlx::query_as::<_, (String, String)>(
        r"
        SELECT device_a, device_b
        FROM pairings
        WHERE device_a = ? OR device_b = ?
        ORDER BY created_at DESC
        LIMIT 1
        ",
    )
    .bind(&id_str)
    .bind(&id_str)
    .fetch_optional(executor)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "get_pair".to_string(),
        message: e.to_string(),
    })?;

    let Some(r) = row else {
        return Ok(None);
    };

    let peer_str = if r.0 == id_str { r.1 } else { r.0 };

    let peer = Uuid::parse_str(&peer_str)
        .map(DeviceId)
        .map_err(|e| BackendError::StoreDecode {
            column: "pairings.device_a/b".to_string(),
            message: e.to_string(),
        })?;
    Ok(Some(peer))
}

/// Delete the pairing between `a` and `b` (in either order). No-op if no
/// such row exists; callers that need strict semantics can check via
/// [`get_pair`] first.
pub async fn delete_pair(pool: &SqlitePool, a: DeviceId, b: DeviceId) -> Result<(), BackendError> {
    delete_pair_with_executor(pool, a, b).await
}

pub(crate) async fn delete_pair_with_executor<'e, E>(
    executor: E,
    a: DeviceId,
    b: DeviceId,
) -> Result<(), BackendError>
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
    .map_err(|e| BackendError::StoreQuery {
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
        insert_device(pool, a, "mac", DeviceRole::AgentHost, T0)
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
    async fn insert_pairing_allows_second_distinct_pair_for_agent_host() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;
        let c = DeviceId::new();
        insert_device(&pool, c, "ios-2", DeviceRole::IosClient, T0)
            .await
            .unwrap();

        insert_pairing(&pool, a, b, T0).await.unwrap();
        insert_pairing(&pool, a, c, T0 + 1).await.unwrap();

        let peers = get_peers(&pool, a).await.unwrap();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&b));
        assert!(peers.contains(&c));
    }

    #[tokio::test]
    async fn insert_pairing_allows_second_distinct_pair_for_ios_device() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;
        let other_mac = DeviceId::new();
        insert_device(&pool, other_mac, "mac-2", DeviceRole::AgentHost, T0)
            .await
            .unwrap();

        insert_pairing(&pool, a, b, T0).await.unwrap();
        insert_pairing(&pool, other_mac, b, T0 + 1).await.unwrap();

        let peers = get_peers(&pool, b).await.unwrap();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&a));
        assert!(peers.contains(&other_mac));
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
    async fn get_pair_returns_most_recent_peer_when_multiple_exist() {
        let pool = memory_pool().await;
        let (ios, mac_a) = two_devices(&pool).await;
        let mac_b = DeviceId::new();
        insert_device(&pool, mac_b, "mac-b", DeviceRole::AgentHost, T0)
            .await
            .unwrap();

        insert_pairing(&pool, ios, mac_a, T0).await.unwrap();
        insert_pairing(&pool, ios, mac_b, T0 + 1).await.unwrap();

        assert_eq!(get_pair(&pool, ios).await.unwrap(), Some(mac_b));
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

    #[tokio::test]
    async fn get_pair_with_created_at_round_trips_peer_and_timestamp() {
        let pool = memory_pool().await;
        let (a, b) = two_devices(&pool).await;
        let now = T0 + 1_234;
        insert_pairing(&pool, a, b, now).await.unwrap();

        let from_a = get_pair_with_created_at(&pool, a).await.unwrap();
        assert_eq!(from_a, Some((b, now)));
        let from_b = get_pair_with_created_at(&pool, b).await.unwrap();
        assert_eq!(from_b, Some((a, now)));
    }

    #[tokio::test]
    async fn get_pair_with_created_at_unpaired_returns_none() {
        let pool = memory_pool().await;
        let (a, _b) = two_devices(&pool).await;
        assert_eq!(get_pair_with_created_at(&pool, a).await.unwrap(), None);
    }
}
