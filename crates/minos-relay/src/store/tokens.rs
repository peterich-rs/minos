//! `pairing_tokens` table CRUD.
//!
//! A pairing token is a 32-byte random bearer value the Mac host hands to
//! the iOS client (typically via QR). The relay stores only the argon2id
//! hash of the token plus the issuer's `DeviceId` and expiry. Tokens are
//! one-shot: [`consume_token`] atomically marks a row `consumed_at = now`
//! and returns the issuer, or returns `None` for any non-usable state
//! (expired, already consumed, unknown).
//!
//! Naming note: the plan text (§5) says `consume_token` returns
//! `Option<IssuerId>`. There is no `IssuerId` type in the workspace; the
//! issuer is just a `DeviceId`. We return a small typed wrapper
//! ([`ConsumedToken`]) so future callsites don't confuse "a device id that
//! happens to be the issuer" with other device ids in the same scope.

use minos_domain::DeviceId;
use sqlx::{Executor, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::error::RelayError;

/// Successful output of [`consume_token`].
///
/// Kept as a struct (rather than a bare `DeviceId`) to document intent at
/// the call site and leave room for future fields (e.g. `issued_at`)
/// without an API break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedToken {
    pub issuer_device_id: DeviceId,
}

/// Insert a new pairing token row.
///
/// `token_hash` is an argon2id verifier string; since argon2 uses a fresh
/// random salt per call, even identical inputs produce different hash
/// strings — so collision on the PK is not a practical concern. `now` and
/// `expires_at` are both unix epoch ms.
pub async fn issue_token(
    pool: &SqlitePool,
    token_hash: &str,
    issuer: DeviceId,
    expires_at: i64,
    now: i64,
) -> Result<(), RelayError> {
    let issuer_str = issuer.to_string();

    sqlx::query!(
        r#"
        INSERT INTO pairing_tokens (token_hash, issuer_device_id, created_at, expires_at, consumed_at)
        VALUES (?, ?, ?, ?, NULL)
        "#,
        token_hash,
        issuer_str,
        now,
        expires_at,
    )
    .execute(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "issue_token".to_string(),
        message: e.to_string(),
    })?;

    Ok(())
}

/// Atomically consume a pairing token.
///
/// Uses `UPDATE ... WHERE consumed_at IS NULL AND expires_at > now
/// RETURNING issuer_device_id` so the check-and-set is a single statement.
/// SQLite's default rollback-journal isolation guarantees no two callers
/// can both succeed on the same row.
///
/// Returns `Ok(None)` if the row is missing, expired, or already consumed.
/// The caller cannot distinguish between these three cases — that's a
/// feature: leaking "token exists but is expired" vs. "no such token"
/// helps an attacker enumerate issued tokens.
pub(crate) async fn peek_usable_token_with_executor<'e, E>(
    executor: E,
    token_hash_candidate: &str,
    now: i64,
) -> Result<Option<ConsumedToken>, RelayError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let issuer_device_id = sqlx::query_scalar::<_, String>(
        "
        SELECT issuer_device_id
        FROM pairing_tokens
        WHERE token_hash = ?
          AND consumed_at IS NULL
          AND expires_at > ?
        ",
    )
    .bind(token_hash_candidate)
    .bind(now)
    .fetch_optional(executor)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "peek_usable_token".to_string(),
        message: e.to_string(),
    })?;

    decode_consumed_token(issuer_device_id, "peek_usable_token")
}

pub async fn consume_token(
    pool: &SqlitePool,
    token_hash_candidate: &str,
    now: i64,
) -> Result<Option<ConsumedToken>, RelayError> {
    consume_token_with_executor(pool, token_hash_candidate, now).await
}

pub(crate) async fn consume_token_with_executor<'e, E>(
    executor: E,
    token_hash_candidate: &str,
    now: i64,
) -> Result<Option<ConsumedToken>, RelayError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query!(
        r#"
        UPDATE pairing_tokens
        SET consumed_at = ?
        WHERE token_hash = ?
          AND consumed_at IS NULL
          AND expires_at > ?
        RETURNING issuer_device_id
        "#,
        now,
        token_hash_candidate,
        now,
    )
    .fetch_optional(executor)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "consume_token".to_string(),
        message: e.to_string(),
    })?;

    decode_consumed_token(row.map(|r| r.issuer_device_id), "consume_token")
}

fn decode_consumed_token(
    issuer_device_id: Option<String>,
    operation: &str,
) -> Result<Option<ConsumedToken>, RelayError> {
    let Some(issuer_device_id) = issuer_device_id else {
        return Ok(None);
    };

    let issuer_device_id = Uuid::parse_str(&issuer_device_id)
        .map(DeviceId)
        .map_err(|e| RelayError::StoreDecode {
            column: format!("pairing_tokens.issuer_device_id ({operation})"),
            message: e.to_string(),
        })?;
    Ok(Some(ConsumedToken { issuer_device_id }))
}

/// Delete expired, unconsumed tokens. Returns the number of rows removed.
///
/// Consumed tokens are preserved as an audit trail (spec §8.2:
/// `consumed_at` is permanent).
pub async fn gc_expired(pool: &SqlitePool, now: i64) -> Result<u64, RelayError> {
    let result = sqlx::query!(
        r#"DELETE FROM pairing_tokens WHERE expires_at <= ? AND consumed_at IS NULL"#,
        now,
    )
    .execute(pool)
    .await
    .map_err(|e| RelayError::StoreQuery {
        operation: "gc_expired".to_string(),
        message: e.to_string(),
    })?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::devices::insert_device;
    use crate::store::test_support::{memory_pool, T0};
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;

    const HOUR_MS: i64 = 60 * 60 * 1_000;

    async fn issuer_device(pool: &SqlitePool) -> DeviceId {
        let id = DeviceId::new();
        insert_device(pool, id, "mac", DeviceRole::MacHost, T0)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn issue_then_consume_happy_path_returns_issuer() {
        let pool = memory_pool().await;
        let issuer = issuer_device(&pool).await;
        issue_token(&pool, "hash-A", issuer, T0 + HOUR_MS, T0)
            .await
            .unwrap();

        let consumed = consume_token(&pool, "hash-A", T0 + 1).await.unwrap();
        assert_eq!(
            consumed,
            Some(ConsumedToken {
                issuer_device_id: issuer,
            }),
        );
    }

    #[tokio::test]
    async fn consume_expired_token_returns_none() {
        let pool = memory_pool().await;
        let issuer = issuer_device(&pool).await;
        issue_token(&pool, "hash-A", issuer, T0 + HOUR_MS, T0)
            .await
            .unwrap();

        // now equals expires_at → expired (strict `>` in the WHERE clause).
        let consumed = consume_token(&pool, "hash-A", T0 + HOUR_MS).await.unwrap();
        assert_eq!(consumed, None);

        // And any later `now` still fails.
        let consumed = consume_token(&pool, "hash-A", T0 + HOUR_MS + 5)
            .await
            .unwrap();
        assert_eq!(consumed, None);
    }

    #[tokio::test]
    async fn double_consume_returns_none_on_second_call() {
        let pool = memory_pool().await;
        let issuer = issuer_device(&pool).await;
        issue_token(&pool, "hash-A", issuer, T0 + HOUR_MS, T0)
            .await
            .unwrap();

        let first = consume_token(&pool, "hash-A", T0 + 10).await.unwrap();
        assert!(first.is_some());
        let second = consume_token(&pool, "hash-A", T0 + 20).await.unwrap();
        assert_eq!(second, None);
    }

    #[tokio::test]
    async fn consume_unknown_token_returns_none() {
        let pool = memory_pool().await;
        let _issuer = issuer_device(&pool).await;
        let consumed = consume_token(&pool, "not-in-db", T0).await.unwrap();
        assert_eq!(consumed, None);
    }

    #[tokio::test]
    async fn gc_expired_removes_only_unconsumed_expired_rows() {
        let pool = memory_pool().await;
        let issuer = issuer_device(&pool).await;

        // A: expired + unconsumed → should be GC'd.
        issue_token(&pool, "hash-A", issuer, T0 + 100, T0)
            .await
            .unwrap();
        // B: fresh + unconsumed → should stay.
        issue_token(&pool, "hash-B", issuer, T0 + HOUR_MS, T0)
            .await
            .unwrap();
        // C: expired + consumed → stays as audit trail.
        issue_token(&pool, "hash-C", issuer, T0 + 200, T0)
            .await
            .unwrap();
        let _ = consume_token(&pool, "hash-C", T0 + 50).await.unwrap();

        let removed = gc_expired(&pool, T0 + 1_000).await.unwrap();
        assert_eq!(removed, 1, "only hash-A is expired-and-unconsumed");

        let remaining: Vec<String> =
            sqlx::query_scalar!("SELECT token_hash FROM pairing_tokens ORDER BY token_hash")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec!["hash-B".to_string(), "hash-C".to_string()]);
    }

    #[tokio::test]
    async fn gc_expired_with_no_candidates_returns_zero() {
        let pool = memory_pool().await;
        let issuer = issuer_device(&pool).await;
        issue_token(&pool, "hash-A", issuer, T0 + HOUR_MS, T0)
            .await
            .unwrap();
        let removed = gc_expired(&pool, T0).await.unwrap();
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn issue_token_duplicate_hash_surfaces_as_store_query_error() {
        // Argon2 salts make this impossible in practice, but the PK
        // constraint still applies — make sure we translate rather than
        // panic.
        let pool = memory_pool().await;
        let issuer = issuer_device(&pool).await;
        issue_token(&pool, "hash-A", issuer, T0 + HOUR_MS, T0)
            .await
            .unwrap();
        let err = issue_token(&pool, "hash-A", issuer, T0 + HOUR_MS, T0)
            .await
            .unwrap_err();
        match err {
            RelayError::StoreQuery { operation, .. } => {
                assert_eq!(operation, "issue_token");
            }
            other => panic!("expected StoreQuery, got {other:?}"),
        }
    }
}
