//! `accounts` table CRUD. Account ids are UUIDv4 strings; emails are
//! lowercased before lookup (the table is `COLLATE NOCASE` for defence).
//!
//! Uses runtime `sqlx::query` / `sqlx::query_as` rather than the macro
//! form deliberately. The macro variants need a populated dev DB during
//! `cargo build` and add a `cargo sqlx prepare` step to every CI run per
//! migration. The schema here is small (one table, four columns) and is
//! exercised by integration tests in `tests/auth_endpoints.rs`. If this
//! file ever grows complex queries that benefit from compile-time
//! checking, migrate to the macro form alongside the existing
//! `devices.rs` / `tokens.rs` / `pairings.rs` callers.

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::BackendError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRow {
    pub account_id: String,
    pub email: String,
    pub minos_id: String,
    pub display_name: Option<String>,
    pub password_hash: String,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

pub async fn create(
    pool: &SqlitePool,
    email: &str,
    password_hash: &str,
) -> Result<AccountRow, BackendError> {
    let account_id = Uuid::new_v4().to_string();
    let email_norm = email.to_lowercase();
    let now = Utc::now().timestamp_millis();
    for _ in 0..4 {
        let minos_id = Uuid::new_v4().simple().to_string()[..12].to_string();
        let result = sqlx::query(
            "INSERT INTO accounts (account_id, email, minos_id, display_name, password_hash, created_at)
               VALUES (?, ?, ?, NULL, ?, ?)",
        )
        .bind(&account_id)
        .bind(&email_norm)
        .bind(&minos_id)
        .bind(password_hash)
        .bind(now)
        .execute(pool)
        .await;

        match result {
            Ok(_) => {
                return Ok(AccountRow {
                    account_id,
                    email: email_norm,
                    minos_id,
                    display_name: None,
                    password_hash: password_hash.into(),
                    created_at: now,
                    last_login_at: None,
                });
            }
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                if db.message().contains("accounts.email")
                    || db.message().contains("idx_accounts_email")
                {
                    return Err(BackendError::EmailTaken);
                }
            }
            Err(e) => {
                return Err(BackendError::StoreQuery {
                    operation: "accounts::create".into(),
                    message: e.to_string(),
                });
            }
        }
    }

    Err(BackendError::StoreQuery {
        operation: "accounts::create".into(),
        message: "failed to allocate unique minos_id".into(),
    })
}

pub async fn find_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<AccountRow>, BackendError> {
    let email_norm = email.to_lowercase();
    let row = sqlx::query_as::<_, AccountRow>(
        "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
           FROM accounts WHERE email = ?",
    )
    .bind(&email_norm)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::find_by_email".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn find_by_id(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Option<AccountRow>, BackendError> {
    let row = sqlx::query_as::<_, AccountRow>(
        "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
           FROM accounts WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::find_by_id".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn touch_last_login(pool: &SqlitePool, account_id: &str) -> Result<(), BackendError> {
    let now = Utc::now().timestamp_millis();
    sqlx::query("UPDATE accounts SET last_login_at = ? WHERE account_id = ?")
        .bind(now)
        .bind(account_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "accounts::touch_last_login".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

pub async fn find_by_minos_id(
    pool: &SqlitePool,
    minos_id: &str,
) -> Result<Option<AccountRow>, BackendError> {
    let row = sqlx::query_as::<_, AccountRow>(
        "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
           FROM accounts WHERE minos_id = ? COLLATE BINARY",
    )
    .bind(minos_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::find_by_minos_id".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn create_then_find_by_email_round_trips() {
        let pool = memory_pool().await;
        let row = create(&pool, "Alice@Example.com", "phc-string")
            .await
            .unwrap();
        assert_eq!(row.email, "alice@example.com");
        assert_eq!(row.password_hash, "phc-string");
        assert!(row.last_login_at.is_none());

        let got = find_by_email(&pool, "ALICE@example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.account_id, row.account_id);
        assert_eq!(got.email, "alice@example.com");
    }

    #[tokio::test]
    async fn create_with_duplicate_email_returns_email_taken() {
        let pool = memory_pool().await;
        create(&pool, "alice@example.com", "phc1").await.unwrap();
        let err = create(&pool, "ALICE@example.com", "phc2")
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::EmailTaken));
    }

    #[tokio::test]
    async fn find_by_email_missing_returns_none() {
        let pool = memory_pool().await;
        let got = find_by_email(&pool, "missing@example.com").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn touch_last_login_updates_timestamp() {
        let pool = memory_pool().await;
        let row = create(&pool, "alice@example.com", "phc").await.unwrap();
        touch_last_login(&pool, &row.account_id).await.unwrap();
        let got = find_by_email(&pool, "alice@example.com")
            .await
            .unwrap()
            .unwrap();
        assert!(got.last_login_at.is_some());
    }
}
