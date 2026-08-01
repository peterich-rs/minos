//! `accounts` table CRUD. Account ids are UUIDv4 strings; emails are
//! lowercased before lookup (the table is `COLLATE NOCASE` for defence).
//!
//! Accounts are IdP-bound via `supabase_sub` (created through
//! `POST /v1/auth/supabase` exchange). There is no local password storage.
//!
//! Uses runtime `sqlx::query` / `sqlx::query_as` rather than the macro
//! form deliberately. The macro variants need a populated dev DB during
//! `cargo build` and add a `cargo sqlx prepare` step to every CI run per
//! migration. The schema here is small and is exercised by integration
//! tests in `tests/auth_endpoints.rs`.

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
    pub supabase_sub: Option<String>,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

const ACCOUNT_SELECT_SQLITE: &str = "SELECT account_id, email, minos_id, display_name, supabase_sub, created_at, last_login_at
                   FROM accounts";

const ACCOUNT_SELECT_POSTGRES: &str = "SELECT account_id, email, minos_id, display_name,
                   supabase_sub,
                   created_at_ms AS created_at,
                   last_login_at_ms AS last_login_at
                   FROM accounts";

/// Create an account without a Supabase subject (test fixtures / rare
/// unbound rows that later bind via exchange).
pub async fn create<S>(store: &S, email: &str) -> Result<AccountRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    create_inner(store, email, None).await
}

/// Create an account bound to a Supabase Auth subject.
pub async fn create_with_supabase_sub<S>(
    store: &S,
    email: &str,
    supabase_sub: &str,
) -> Result<AccountRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    create_inner(store, email, Some(supabase_sub)).await
}

async fn create_inner<S>(
    store: &S,
    email: &str,
    supabase_sub: Option<&str>,
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
                create_sqlite(pool, &account_id, &email_norm, &minos_id, supabase_sub, now).await
            }
            StorePoolRef::Postgres(pool) => {
                create_postgres(pool, &account_id, &email_norm, &minos_id, supabase_sub, now).await
            }
        };

        match result {
            Ok(_) => {
                return Ok(AccountRow {
                    account_id,
                    email: email_norm,
                    minos_id,
                    display_name: None,
                    supabase_sub: supabase_sub.map(str::to_owned),
                    created_at: now,
                    last_login_at: None,
                });
            }
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                if is_email_unique_violation(db.as_ref()) {
                    return Err(BackendError::EmailTaken);
                }
                if is_supabase_sub_unique_violation(db.as_ref()) {
                    return Err(BackendError::StoreQuery {
                        operation: "accounts::create".into(),
                        message: "supabase_sub already bound".into(),
                    });
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
            sqlx::query_as::<_, AccountRow>(&format!("{ACCOUNT_SELECT_SQLITE} WHERE email = ?"))
                .bind(&email_norm)
                .fetch_optional(pool)
                .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AccountRow>(&format!("{ACCOUNT_SELECT_POSTGRES} WHERE email = $1"))
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
            sqlx::query_as::<_, AccountRow>(&format!(
                "{ACCOUNT_SELECT_SQLITE} WHERE account_id = ?"
            ))
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AccountRow>(&format!(
                "{ACCOUNT_SELECT_POSTGRES} WHERE account_id = $1"
            ))
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

pub async fn find_by_supabase_sub<S>(
    store: &S,
    supabase_sub: &str,
) -> Result<Option<AccountRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AccountRow>(&format!(
                "{ACCOUNT_SELECT_SQLITE} WHERE supabase_sub = ?"
            ))
            .bind(supabase_sub)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AccountRow>(&format!(
                "{ACCOUNT_SELECT_POSTGRES} WHERE supabase_sub = $1"
            ))
            .bind(supabase_sub)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "accounts::find_by_supabase_sub".into(),
        message: e.to_string(),
    })?;
    Ok(row)
}

/// Bind a Supabase subject to an existing account. Fails if the account
/// already has a different sub, or if the sub is already bound elsewhere
/// (unique constraint).
pub async fn bind_supabase_sub<S>(
    store: &S,
    account_id: &str,
    supabase_sub: &str,
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    let rows_affected = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE accounts SET supabase_sub = ?
                   WHERE account_id = ?
                     AND (supabase_sub IS NULL OR supabase_sub = ?)",
        )
        .bind(supabase_sub)
        .bind(account_id)
        .bind(supabase_sub)
        .execute(pool)
        .await
        .map(|done| done.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE accounts SET supabase_sub = $1
                   WHERE account_id = $2
                     AND (supabase_sub IS NULL OR supabase_sub = $1)",
        )
        .bind(supabase_sub)
        .bind(account_id)
        .execute(pool)
        .await
        .map(|done| done.rows_affected()),
    };

    match rows_affected {
        Ok(1) => Ok(()),
        Ok(_) => Err(BackendError::StoreQuery {
            operation: "accounts::bind_supabase_sub".into(),
            message: "account missing or already bound to a different supabase_sub".into(),
        }),
        Err(sqlx::Error::Database(db)) if is_supabase_sub_unique_violation(db.as_ref()) => {
            Err(BackendError::StoreQuery {
                operation: "accounts::bind_supabase_sub".into(),
                message: "supabase_sub already bound".into(),
            })
        }
        Err(e) => Err(BackendError::StoreQuery {
            operation: "accounts::bind_supabase_sub".into(),
            message: e.to_string(),
        }),
    }
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
            sqlx::query("UPDATE accounts SET last_login_at_ms = $1 WHERE account_id = $2")
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

pub async fn find_by_minos_id<S>(
    store: &S,
    minos_id: &str,
) -> Result<Option<AccountRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, AccountRow>(&format!(
                "{ACCOUNT_SELECT_SQLITE} WHERE minos_id = ? COLLATE BINARY"
            ))
            .bind(minos_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, AccountRow>(&format!(
                "{ACCOUNT_SELECT_POSTGRES} WHERE minos_id = $1"
            ))
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
    supabase_sub: Option<&str>,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO accounts (account_id, email, minos_id, display_name, supabase_sub, created_at)
           VALUES (?, ?, ?, NULL, ?, ?)",
    )
    .bind(account_id)
    .bind(email_norm)
    .bind(minos_id)
    .bind(supabase_sub)
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
    supabase_sub: Option<&str>,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO accounts (account_id, email, minos_id, display_name, supabase_sub, created_at_ms)
           VALUES ($1, $2, $3, NULL, $4, $5)",
    )
    .bind(account_id)
    .bind(email_norm)
    .bind(minos_id)
    .bind(supabase_sub)
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

fn is_supabase_sub_unique_violation(db: &dyn DatabaseError) -> bool {
    matches!(db.constraint(), Some(name) if name.eq_ignore_ascii_case("idx_accounts_supabase_sub")
        || name.eq_ignore_ascii_case("accounts_supabase_sub_key")
        || name.eq_ignore_ascii_case("accounts_supabase_sub"))
        || db.message().contains("supabase_sub")
        || db.message().contains("idx_accounts_supabase_sub")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn create_then_find_by_email_round_trips() {
        let pool = memory_pool().await;
        let row = create(&pool, "Alice@Example.com").await.unwrap();
        assert_eq!(row.email, "alice@example.com");
        assert!(row.supabase_sub.is_none());
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
        create(&pool, "alice@example.com").await.unwrap();
        let err = create(&pool, "ALICE@example.com").await.unwrap_err();
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
        let row = create(&pool, "alice@example.com").await.unwrap();
        touch_last_login(&pool, &row.account_id).await.unwrap();
        let got = find_by_email(&pool, "alice@example.com")
            .await
            .unwrap()
            .unwrap();
        assert!(got.last_login_at.is_some());
    }

    #[tokio::test]
    async fn create_with_supabase_sub_and_find() {
        let pool = memory_pool().await;
        let row = create_with_supabase_sub(&pool, "oidc@example.com", "sub-abc")
            .await
            .unwrap();
        assert_eq!(row.supabase_sub.as_deref(), Some("sub-abc"));
        let got = find_by_supabase_sub(&pool, "sub-abc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.account_id, row.account_id);
    }

    #[tokio::test]
    async fn bind_supabase_sub_links_unbound_account() {
        let pool = memory_pool().await;
        let row = create(&pool, "merge@example.com").await.unwrap();
        bind_supabase_sub(&pool, &row.account_id, "sub-merge")
            .await
            .unwrap();
        let got = find_by_supabase_sub(&pool, "sub-merge")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.account_id, row.account_id);
    }
}
