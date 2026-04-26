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
use sqlx::SqlitePool;

use crate::error::BackendError;

pub const REFRESH_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RefreshTokenRow {
    pub token_hash: String,
    pub account_id: String,
    pub device_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

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

pub async fn insert(
    pool: &SqlitePool,
    plaintext: &str,
    account_id: &str,
    device_id: &str,
) -> Result<RefreshTokenRow, BackendError> {
    let now = Utc::now().timestamp_millis();
    let row = RefreshTokenRow {
        token_hash: hash_plaintext(plaintext),
        account_id: account_id.into(),
        device_id: device_id.into(),
        issued_at: now,
        expires_at: now + REFRESH_TTL_MS,
        revoked_at: None,
    };
    sqlx::query(
        "INSERT INTO refresh_tokens (token_hash, account_id, device_id, issued_at, expires_at)
           VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&row.token_hash)
    .bind(&row.account_id)
    .bind(&row.device_id)
    .bind(row.issued_at)
    .bind(row.expires_at)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::insert".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn find_active(
    pool: &SqlitePool,
    plaintext: &str,
) -> Result<Option<RefreshTokenRow>, BackendError> {
    let hash = hash_plaintext(plaintext);
    let now = Utc::now().timestamp_millis();
    let row = sqlx::query_as::<_, RefreshTokenRow>(
        "SELECT token_hash, account_id, device_id, issued_at, expires_at, revoked_at
           FROM refresh_tokens
           WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
    )
    .bind(&hash)
    .bind(now)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::find_active".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn revoke_one(pool: &SqlitePool, plaintext: &str) -> Result<(), BackendError> {
    let hash = hash_plaintext(plaintext);
    let now = Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(&hash)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::revoke_one".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Revoke every active refresh token for an account. Used on login to
/// enforce single-active-iPhone (spec §5.2).
pub async fn revoke_all_for_account(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<u64, BackendError> {
    let now = Utc::now().timestamp_millis();
    let result = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = ? WHERE account_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(account_id)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "refresh_tokens::revoke_all_for_account".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::accounts;
    use crate::store::devices::insert_device;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::{DeviceId, DeviceRole};
    use pretty_assertions::assert_eq;

    async fn setup_account_and_device(pool: &SqlitePool) -> (String, String) {
        let account = accounts::create(pool, "alice@example.com", "phc")
            .await
            .unwrap();
        let device_id = DeviceId::new();
        insert_device(pool, device_id, "iphone", DeviceRole::IosClient, T0)
            .await
            .unwrap();
        (account.account_id, device_id.to_string())
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
    async fn find_active_with_unknown_token_returns_none() {
        let pool = memory_pool().await;
        let (_account_id, _device_id) = setup_account_and_device(&pool).await;
        assert!(find_active(&pool, "not-a-real-token").await.unwrap().is_none());
    }
}
