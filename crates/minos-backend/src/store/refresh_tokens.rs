//! `refresh_tokens` table. Tokens are stored as SHA-256 hex of the
//! 32-byte random plaintext; plaintext is only ever in transit. Same
//! pattern as `pairing_tokens`.
//!
//! Uses runtime `sqlx::query` / `sqlx::query_as` rather than the macro
//! form deliberately. The macro variants require a populated dev DB
//! during `cargo build` and an extra `cargo sqlx prepare` step in CI per
//! migration. The schema is small and the contract here is covered by
//! integration tests in `tests/auth_endpoints.rs`. If this file grows
//! complex queries that benefit from compile-time checking, migrate to
//! the macro form alongside `devices.rs` / `tokens.rs` / `pairings.rs`.

use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

pub const REFRESH_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RefreshTokenRow {
    pub token_hash: String,
    pub account_id: String,
    pub device_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
    pub rotated_to_hash: Option<String>,
}

const REFRESH_SELECT_SQLITE: &str = "SELECT token_hash, account_id, installation_id AS device_id, \
     issued_at_ms AS issued_at, expires_at_ms AS expires_at, revoked_at_ms AS revoked_at, \
     rotated_to_hash \
     FROM refresh_tokens";

const REFRESH_SELECT_POSTGRES: &str = "SELECT token_hash, account_id, installation_id AS device_id, \
     issued_at_ms AS issued_at, expires_at_ms AS expires_at, revoked_at_ms AS revoked_at, \
     rotated_to_hash \
     FROM refresh_tokens";

/// 32 random bytes from the OS CSPRNG, hex-encoded (64 chars).
///
/// Mirrors `DeviceSecret::generate` style by going through `getrandom`
/// directly so we don't pull in the `rand` crate.
#[must_use]
pub fn generate_plaintext() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    hex_encode(&bytes)
}

#[must_use]
pub fn hash_plaintext(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.as_bytes());
    hex_encode(&digest)
}

/// Hand-rolled `{:02x}` encoder; matches the helper in `pairing/mod.rs` so
/// we don't pull in the `hex` crate for a single output.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{b:02x}").expect("String write never fails");
    }
    out
}

pub async fn insert<S>(
    store: &S,
    plaintext: &str,
    account_id: &str,
    device_id: &str,
) -> Result<RefreshTokenRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let now = Utc::now().timestamp_millis();
    let row = RefreshTokenRow {
        token_hash: hash_plaintext(plaintext),
        account_id: account_id.into(),
        device_id: device_id.into(),
        issued_at: now,
        expires_at: now + REFRESH_TTL_MS,
        revoked_at: None,
        rotated_to_hash: None,
    };
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => insert_sqlite(pool, &row).await,
        StorePoolRef::Postgres(pool) => insert_postgres(pool, &row).await,
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::insert".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn find_active<S>(
    store: &S,
    plaintext: &str,
) -> Result<Option<RefreshTokenRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let hash = hash_plaintext(plaintext);
    let now = Utc::now().timestamp_millis();
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, RefreshTokenRow>(&format!(
                "{REFRESH_SELECT_SQLITE}
                   WHERE token_hash = ? AND revoked_at_ms IS NULL AND expires_at_ms > ?"
            ))
            .bind(&hash)
            .bind(now)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, RefreshTokenRow>(&format!(
                "{REFRESH_SELECT_POSTGRES}
                   WHERE token_hash = $1 AND revoked_at_ms IS NULL AND expires_at_ms > $2"
            ))
            .bind(&hash)
            .bind(now)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::find_active".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn find_any<S>(
    store: &S,
    plaintext: &str,
) -> Result<Option<RefreshTokenRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let hash = hash_plaintext(plaintext);
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, RefreshTokenRow>(&format!(
                "{REFRESH_SELECT_SQLITE}
                   WHERE token_hash = ?"
            ))
            .bind(&hash)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, RefreshTokenRow>(&format!(
                "{REFRESH_SELECT_POSTGRES}
                   WHERE token_hash = $1"
            ))
            .bind(&hash)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::find_any".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn revoke_one<S>(store: &S, plaintext: &str) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    let hash = hash_plaintext(plaintext);
    let now = Utc::now().timestamp_millis();
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at_ms = ? WHERE token_hash = ? AND revoked_at_ms IS NULL",
            )
            .bind(now)
            .bind(&hash)
            .execute(pool)
            .await
            .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at_ms = $1 WHERE token_hash = $2 AND revoked_at_ms IS NULL",
            )
            .bind(now)
            .bind(&hash)
            .execute(pool)
            .await
            .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::revoke_one".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Atomically revoke the old refresh token and insert a new one in a
/// single transaction. Returns `None` if the old token was already
/// revoked (CAS failure — another concurrent request won the race).
/// Returns `Some(new_row)` on success.
pub async fn rotate<S>(
    store: &S,
    old_plaintext: &str,
    new_plaintext: &str,
    account_id: &str,
    device_id: &str,
) -> Result<Option<RefreshTokenRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let old_hash = hash_plaintext(old_plaintext);
    let now = Utc::now().timestamp_millis();

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            rotate_sqlite(pool, &old_hash, new_plaintext, account_id, device_id, now).await
        }
        StorePoolRef::Postgres(pool) => {
            rotate_postgres(pool, &old_hash, new_plaintext, account_id, device_id, now).await
        }
    }
}

/// Revoke every active refresh token for an account. Used on login to
/// invalidate all devices for that account when an administrative flow needs
/// it. Normal login uses [`revoke_all_for_device`] so multiple iOS devices can
/// remain signed in at once.
pub async fn revoke_all_for_account(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<u64, BackendError> {
    let now = Utc::now().timestamp_millis();
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at_ms = ? WHERE account_id = ? AND revoked_at_ms IS NULL",
            )
            .bind(now)
            .bind(account_id)
            .execute(pool)
            .await
            .map(|result| result.rows_affected())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at_ms = $1 WHERE account_id = $2 AND revoked_at_ms IS NULL",
            )
            .bind(now)
            .bind(account_id)
            .execute(pool)
            .await
            .map(|result| result.rows_affected())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::revoke_all_for_account".into(),
        message: e.to_string(),
    })?;
    Ok(result)
}

/// Revoke every active refresh token for a single device.
///
/// Login mints a fresh token for the current device, but it must not evict
/// other devices on the same account now that multi-device sessions are
/// supported.
pub async fn revoke_all_for_device(
    store: &impl AsStorePool,
    device_id: &str,
) -> Result<u64, BackendError> {
    let now = Utc::now().timestamp_millis();
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE refresh_tokens SET revoked_at_ms = ? WHERE installation_id = ? AND revoked_at_ms IS NULL",
        )
        .bind(now)
        .bind(device_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE refresh_tokens SET revoked_at_ms = $1 WHERE installation_id = $2 AND revoked_at_ms IS NULL",
        )
        .bind(now)
        .bind(device_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::revoke_all_for_device".into(),
        message: e.to_string(),
    })?;
    Ok(result)
}

/// Delete expired or already-revoked refresh token rows.
pub async fn gc_expired(store: &impl AsStorePool, now_ms: i64) -> Result<u64, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "DELETE FROM refresh_tokens WHERE expires_at_ms <= ? OR revoked_at_ms IS NOT NULL",
        )
        .bind(now_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "DELETE FROM refresh_tokens WHERE expires_at_ms <= $1 OR revoked_at_ms IS NOT NULL",
        )
        .bind(now_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::gc_expired".into(),
        message: e.to_string(),
    })?;
    Ok(result)
}

async fn insert_sqlite(pool: &SqlitePool, row: &RefreshTokenRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO refresh_tokens (token_hash, account_id, installation_id, issued_at_ms, expires_at_ms)
           VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&row.token_hash)
    .bind(&row.account_id)
    .bind(&row.device_id)
    .bind(row.issued_at)
    .bind(row.expires_at)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn insert_postgres(pool: &PgPool, row: &RefreshTokenRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO refresh_tokens (token_hash, account_id, installation_id, issued_at_ms, expires_at_ms)
           VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&row.token_hash)
    .bind(&row.account_id)
    .bind(&row.device_id)
    .bind(row.issued_at)
    .bind(row.expires_at)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn rotate_sqlite(
    pool: &SqlitePool,
    old_hash: &str,
    new_plaintext: &str,
    account_id: &str,
    device_id: &str,
    now: i64,
) -> Result<Option<RefreshTokenRow>, BackendError> {
    let mut tx = pool.begin().await.map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.begin".into(),
        message: e.to_string(),
    })?;

    // Insert new first so rotated_to_hash FK (self-ref) is satisfiable, then CAS-revoke old.
    let new_hash = hash_plaintext(new_plaintext);
    let expires_at = now + REFRESH_TTL_MS;
    sqlx::query(
        "INSERT INTO refresh_tokens (token_hash, account_id, installation_id, issued_at_ms, expires_at_ms)
           VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&new_hash)
    .bind(account_id)
    .bind(device_id)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.insert".into(),
        message: e.to_string(),
    })?;

    let result = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at_ms = ?, rotated_to_hash = ?
          WHERE token_hash = ? AND revoked_at_ms IS NULL",
    )
    .bind(now)
    .bind(&new_hash)
    .bind(old_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.revoke".into(),
        message: e.to_string(),
    })?;

    if result.rows_affected() == 0 {
        tx.rollback().await.ok();
        return Ok(None);
    }

    tx.commit().await.map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.commit".into(),
        message: e.to_string(),
    })?;

    Ok(Some(RefreshTokenRow {
        token_hash: new_hash,
        account_id: account_id.into(),
        device_id: device_id.into(),
        issued_at: now,
        expires_at,
        revoked_at: None,
        rotated_to_hash: None,
    }))
}

async fn rotate_postgres(
    pool: &PgPool,
    old_hash: &str,
    new_plaintext: &str,
    account_id: &str,
    device_id: &str,
    now: i64,
) -> Result<Option<RefreshTokenRow>, BackendError> {
    let mut tx = pool.begin().await.map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.begin".into(),
        message: e.to_string(),
    })?;

    // Insert new first so rotated_to_hash FK (self-ref) is satisfiable, then CAS-revoke old.
    let new_hash = hash_plaintext(new_plaintext);
    let expires_at = now + REFRESH_TTL_MS;
    sqlx::query(
        "INSERT INTO refresh_tokens (token_hash, account_id, installation_id, issued_at_ms, expires_at_ms)
           VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&new_hash)
    .bind(account_id)
    .bind(device_id)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.insert".into(),
        message: e.to_string(),
    })?;

    let result = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at_ms = $1, rotated_to_hash = $2
          WHERE token_hash = $3 AND revoked_at_ms IS NULL",
    )
    .bind(now)
    .bind(&new_hash)
    .bind(old_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.revoke".into(),
        message: e.to_string(),
    })?;

    if result.rows_affected() == 0 {
        tx.rollback().await.ok();
        return Ok(None);
    }

    tx.commit().await.map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::rotate.commit".into(),
        message: e.to_string(),
    })?;

    Ok(Some(RefreshTokenRow {
        token_hash: new_hash,
        account_id: account_id.into(),
        device_id: device_id.into(),
        issued_at: now,
        expires_at,
        revoked_at: None,
        rotated_to_hash: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_test_client, memory_pool, T0};
    use minos_domain::{DeviceId, DeviceRole};
    use pretty_assertions::assert_eq;

    async fn setup_account_and_device(pool: &SqlitePool) -> (String, String) {
        let account_id = crate::store::test_support::insert_account(pool, "alice@example.com").await;
        let device_id = DeviceId::new();
        insert_test_client(
            pool,
            device_id,
            DeviceRole::MobileClient,
            &account_id,
            "iphone",
            T0,
        )
        .await;
        (account_id, device_id.to_string())
    }

    #[test]
    fn generate_plaintext_is_64_hex_chars_and_unique() {
        let a = generate_plaintext();
        let b = generate_plaintext();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_plaintext_is_deterministic_and_64_chars() {
        let plain = "abc";
        let h1 = hash_plaintext(plain);
        let h2 = hash_plaintext(plain);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        // SHA-256("abc") known vector
        assert_eq!(
            h1,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn insert_then_find_active_round_trips() {
        let pool = memory_pool().await;
        let (account_id, device_id) = setup_account_and_device(&pool).await;
        let plain = generate_plaintext();
        let row = insert(&pool, &plain, &account_id, &device_id)
            .await
            .unwrap();
        assert_eq!(row.account_id, account_id);
        assert_eq!(row.device_id, device_id);

        let got = find_active(&pool, &plain).await.unwrap().unwrap();
        assert_eq!(got.token_hash, row.token_hash);
        assert_eq!(got.account_id, account_id);
        assert_eq!(got.device_id, device_id);
    }

    #[tokio::test]
    async fn revoke_one_makes_token_invisible_to_find_active() {
        let pool = memory_pool().await;
        let (account_id, device_id) = setup_account_and_device(&pool).await;
        let plain = generate_plaintext();
        insert(&pool, &plain, &account_id, &device_id)
            .await
            .unwrap();
        revoke_one(&pool, &plain).await.unwrap();
        assert!(find_active(&pool, &plain).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_any_returns_revoked_row() {
        let pool = memory_pool().await;
        let (account_id, device_id) = setup_account_and_device(&pool).await;
        let plain = generate_plaintext();
        insert(&pool, &plain, &account_id, &device_id)
            .await
            .unwrap();
        revoke_one(&pool, &plain).await.unwrap();

        let row = find_any(&pool, &plain).await.unwrap().unwrap();
        assert!(row.revoked_at.is_some());
    }

    #[tokio::test]
    async fn revoke_all_for_account_revokes_only_unrevoked() {
        let pool = memory_pool().await;
        let (account_id, device_id) = setup_account_and_device(&pool).await;
        let p1 = generate_plaintext();
        let p2 = generate_plaintext();
        insert(&pool, &p1, &account_id, &device_id).await.unwrap();
        insert(&pool, &p2, &account_id, &device_id).await.unwrap();

        let revoked = revoke_all_for_account(&pool, &account_id).await.unwrap();
        assert_eq!(revoked, 2);
        assert!(find_active(&pool, &p1).await.unwrap().is_none());
        assert!(find_active(&pool, &p2).await.unwrap().is_none());

        // Idempotent: a second call revokes 0.
        let revoked2 = revoke_all_for_account(&pool, &account_id).await.unwrap();
        assert_eq!(revoked2, 0);
    }

    #[tokio::test]
    async fn revoke_all_for_device_leaves_other_devices_on_account_active() {
        let pool = memory_pool().await;
        let (account_id, device_a) = setup_account_and_device(&pool).await;
        let device_b = DeviceId::new();
        insert_test_client(
            &pool,
            device_b,
            DeviceRole::MobileClient,
            &account_id,
            "ipad",
            T0,
        )
        .await;
        let device_b = device_b.to_string();
        let p1 = generate_plaintext();
        let p2 = generate_plaintext();
        insert(&pool, &p1, &account_id, &device_a).await.unwrap();
        insert(&pool, &p2, &account_id, &device_b).await.unwrap();

        let revoked = revoke_all_for_device(&pool, &device_a).await.unwrap();
        assert_eq!(revoked, 1);
        assert!(find_active(&pool, &p1).await.unwrap().is_none());
        assert!(find_active(&pool, &p2).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn find_active_with_unknown_token_returns_none() {
        let pool = memory_pool().await;
        let (_account_id, _device_id) = setup_account_and_device(&pool).await;
        assert!(find_active(&pool, "not-a-real-token")
            .await
            .unwrap()
            .is_none());
    }
}
