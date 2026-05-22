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
use sqlx::{error::DatabaseError, PgPool, SqlitePool};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

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

pub async fn create<S>(
    store: &S,
    email: &str,
    password_hash: &str,
) -> Result<AccountRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let account_id = Uuid::new_v4().to_string();
    let email_norm = email.to_lowercase();
    let now = Utc::now().timestamp_millis();
    for _ in 0..4 {
        let minos_id = Uuid::new_v4().simple().to_string()[..12].to_string();
        let result = match store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                create_sqlite(
                    pool,
                    &account_id,
                    &email_norm,
                    &minos_id,
                    password_hash,
                    now,
                )
                .await
            }
            StorePoolRef::Postgres(pool) => {
                create_postgres(
                    pool,
                    &account_id,
                    &email_norm,
                    &minos_id,
                    password_hash,
                    now,
                )
                .await
            }
        };

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
                if is_email_unique_violation(db.as_ref()) {
                    return Err(BackendError::EmailTaken);
                }
                // minos_id collision — retry with a new random id.
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

pub async fn find_by_email<S>(store: &S, email: &str) -> Result<Option<AccountRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let email_norm = email.to_lowercase();
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AccountRow>(
                "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
                   FROM accounts WHERE email = ?",
            )
            .bind(&email_norm)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AccountRow>(
                "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
                   FROM accounts WHERE email = $1",
            )
            .bind(&email_norm)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::find_by_email".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn find_by_id<S>(store: &S, account_id: &str) -> Result<Option<AccountRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AccountRow>(
                "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
                   FROM accounts WHERE account_id = ?",
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AccountRow>(
                "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
                   FROM accounts WHERE account_id = $1",
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::find_by_id".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

pub async fn touch_last_login<S>(store: &S, account_id: &str) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    let now = Utc::now().timestamp_millis();
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query("UPDATE accounts SET last_login_at = ? WHERE account_id = ?")
                .bind(now)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query("UPDATE accounts SET last_login_at = $1 WHERE account_id = $2")
                .bind(now)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::touch_last_login".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub async fn set_password_hash<S>(
    store: &S,
    account_id: &str,
    password_hash: &str,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query("UPDATE accounts SET password_hash = ? WHERE account_id = ?")
                .bind(password_hash)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query("UPDATE accounts SET password_hash = $1 WHERE account_id = $2")
                .bind(password_hash)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::set_password_hash".into(),
        message: e.to_string(),
    })?;
    Ok(())
}

pub async fn find_by_minos_id<S>(
    store: &S,
    minos_id: &str,
) -> Result<Option<AccountRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AccountRow>(
                "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
                   FROM accounts WHERE minos_id = ? COLLATE BINARY",
            )
            .bind(minos_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AccountRow>(
                "SELECT account_id, email, minos_id, display_name, password_hash, created_at, last_login_at
                   FROM accounts WHERE minos_id = $1",
            )
            .bind(minos_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::find_by_minos_id".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

async fn create_sqlite(
    pool: &SqlitePool,
    account_id: &str,
    email_norm: &str,
    minos_id: &str,
    password_hash: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO accounts (account_id, email, minos_id, display_name, password_hash, created_at)
           VALUES (?, ?, ?, NULL, ?, ?)",
    )
    .bind(account_id)
    .bind(email_norm)
    .bind(minos_id)
    .bind(password_hash)
    .bind(now)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn create_postgres(
    pool: &PgPool,
    account_id: &str,
    email_norm: &str,
    minos_id: &str,
    password_hash: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO accounts (account_id, email, minos_id, display_name, password_hash, created_at)
           VALUES ($1, $2, $3, NULL, $4, $5)",
    )
    .bind(account_id)
    .bind(email_norm)
    .bind(minos_id)
    .bind(password_hash)
    .bind(now)
    .execute(pool)
    .await
    .map(|_| ())
}

fn is_email_unique_violation(db: &dyn DatabaseError) -> bool {
    matches!(db.constraint(), Some(name) if name.eq_ignore_ascii_case("idx_accounts_email")
        || name.eq_ignore_ascii_case("accounts_email_key")
        || name.eq_ignore_ascii_case("accounts_email"))
        || db.message().contains("accounts.email")
        || db.message().contains("idx_accounts_email")
        || db.message().contains("accounts_email_key")
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
