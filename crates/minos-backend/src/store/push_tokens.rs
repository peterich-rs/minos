//! `push_tokens` table CRUD. Supports both SQLite and Postgres via
//! `AsStorePool` dispatch. Tokens are hashed (SHA-256) before storage
//! so the raw device token never persists.

use sha2::{Digest, Sha256};
use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PushTokenRow {
    pub token_hash: String,
    pub account_id: String,
    pub installation_id: String,
    pub kind: String,
    pub locale: Option<String>,
    pub created_at_ms: i64,
    pub last_used_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}

/// Hash a raw push token for storage. Uses SHA-256 so the same token
/// always maps to the same row (enabling upsert on re-register).
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Upsert a push token. If the token_hash already exists, refreshes
/// `last_used_at_ms` and `installation_id` (device may have reinstalled).
pub async fn upsert<S>(
    store: &S,
    account_id: &str,
    installation_id: &str,
    kind: &str,
    token: &str,
    locale: Option<&str>,
    at_ms: i64,
) -> Result<PushTokenRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let token_hash = hash_token(token);
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            upsert_sqlite(
                pool,
                &token_hash,
                account_id,
                installation_id,
                kind,
                locale,
                at_ms,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            upsert_postgres(
                pool,
                &token_hash,
                account_id,
                installation_id,
                kind,
                locale,
                at_ms,
            )
            .await
        }
    }
}

/// Revoke a push token by its hash. Returns `true` if a row was updated.
pub async fn revoke<S>(store: &S, token_hash: &str, at_ms: i64) -> Result<bool, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => revoke_sqlite(pool, token_hash, at_ms).await,
        StorePoolRef::Postgres(pool) => revoke_postgres(pool, token_hash, at_ms).await,
    }
}

/// List all active (non-revoked) push tokens for an account.
pub async fn list_for_account<S>(
    store: &S,
    account_id: &str,
) -> Result<Vec<PushTokenRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => list_for_account_sqlite(pool, account_id).await,
        StorePoolRef::Postgres(pool) => list_for_account_postgres(pool, account_id).await,
    }
}

/// List all active push tokens for a set of account IDs (batch for fanout).
pub async fn list_for_accounts<S>(
    store: &S,
    account_ids: &[&str],
) -> Result<Vec<PushTokenRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => list_for_accounts_sqlite(pool, account_ids).await,
        StorePoolRef::Postgres(pool) => list_for_accounts_postgres(pool, account_ids).await,
    }
}

// ── SQLite implementations ─────────────────────────────────────────────

async fn upsert_sqlite(
    pool: &SqlitePool,
    token_hash: &str,
    account_id: &str,
    installation_id: &str,
    kind: &str,
    locale: Option<&str>,
    at_ms: i64,
) -> Result<PushTokenRow, BackendError> {
    sqlx::query_as::<_, PushTokenRow>(
        "INSERT INTO push_tokens (token_hash, account_id, installation_id, kind::text, locale, created_at_ms, last_used_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(token_hash) DO UPDATE SET
             installation_id = excluded.installation_id,
             last_used_at_ms = excluded.last_used_at_ms,
             revoked_at_ms = NULL
         RETURNING *",
    )
    .bind(token_hash)
    .bind(account_id)
    .bind(installation_id)
    .bind(kind)
    .bind(locale)
    .bind(at_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_tokens.upsert".into(),
        message: e.to_string(),
    })
}

async fn revoke_sqlite(
    pool: &SqlitePool,
    token_hash: &str,
    at_ms: i64,
) -> Result<bool, BackendError> {
    let result = sqlx::query(
        "UPDATE push_tokens SET revoked_at_ms = ?1 WHERE token_hash = ?2 AND revoked_at_ms IS NULL",
    )
    .bind(at_ms)
    .bind(token_hash)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_tokens.revoke".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected() > 0)
}

async fn list_for_account_sqlite(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<PushTokenRow>, BackendError> {
    sqlx::query_as::<_, PushTokenRow>(
        "SELECT * FROM push_tokens WHERE account_id = ?1 AND revoked_at_ms IS NULL",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_tokens.list_for_account".into(),
        message: e.to_string(),
    })
}

async fn list_for_accounts_sqlite(
    pool: &SqlitePool,
    account_ids: &[&str],
) -> Result<Vec<PushTokenRow>, BackendError> {
    // Build a parameterized query with the right number of placeholders.
    let placeholders: Vec<String> = account_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect();
    let sql = format!(
        "SELECT * FROM push_tokens WHERE account_id IN ({}) AND revoked_at_ms IS NULL",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, PushTokenRow>(&sql);
    for id in account_ids {
        query = query.bind(*id);
    }
    query
        .fetch_all(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "push_tokens.list_for_accounts".into(),
            message: e.to_string(),
        })
}

// ── Postgres implementations ───────────────────────────────────────────

async fn upsert_postgres(
    pool: &PgPool,
    token_hash: &str,
    account_id: &str,
    installation_id: &str,
    kind: &str,
    locale: Option<&str>,
    at_ms: i64,
) -> Result<PushTokenRow, BackendError> {
    sqlx::query_as::<_, PushTokenRow>(
        "INSERT INTO push_tokens (token_hash, account_id, installation_id, kind::text, locale, created_at_ms, last_used_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $6)
         ON CONFLICT(token_hash) DO UPDATE SET
             installation_id = EXCLUDED.installation_id,
             last_used_at_ms = EXCLUDED.last_used_at_ms,
             revoked_at_ms = NULL
         RETURNING *",
    )
    .bind(token_hash)
    .bind(account_id)
    .bind(installation_id)
    .bind(kind)
    .bind(locale)
    .bind(at_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_tokens.upsert".into(),
        message: e.to_string(),
    })
}

async fn revoke_postgres(
    pool: &PgPool,
    token_hash: &str,
    at_ms: i64,
) -> Result<bool, BackendError> {
    let result = sqlx::query(
        "UPDATE push_tokens SET revoked_at_ms = $1 WHERE token_hash = $2 AND revoked_at_ms IS NULL",
    )
    .bind(at_ms)
    .bind(token_hash)
    .execute(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_tokens.revoke".into(),
        message: e.to_string(),
    })?;
    Ok(result.rows_affected() > 0)
}

async fn list_for_account_postgres(
    pool: &PgPool,
    account_id: &str,
) -> Result<Vec<PushTokenRow>, BackendError> {
    sqlx::query_as::<_, PushTokenRow>(
        "SELECT * FROM push_tokens WHERE account_id = $1 AND revoked_at_ms IS NULL",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_tokens.list_for_account".into(),
        message: e.to_string(),
    })
}

async fn list_for_accounts_postgres(
    pool: &PgPool,
    account_ids: &[&str],
) -> Result<Vec<PushTokenRow>, BackendError> {
    sqlx::query_as::<_, PushTokenRow>(
        "SELECT * FROM push_tokens WHERE account_id = ANY($1) AND revoked_at_ms IS NULL",
    )
    .bind(account_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "push_tokens.list_for_accounts".into(),
        message: e.to_string(),
    })
}
