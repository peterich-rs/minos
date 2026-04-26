//! `accounts` table CRUD. Account ids are UUIDv4 strings; emails are
//! lowercased before lookup (the table is `COLLATE NOCASE` for defence).

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::BackendError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRow {
    pub account_id: String,
    pub email: String,
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
    sqlx::query(
        r#"INSERT INTO accounts (account_id, email, password_hash, created_at)
           VALUES (?, ?, ?, ?)"#,
    )
    .bind(&account_id)
    .bind(&email_norm)
    .bind(password_hash)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => BackendError::EmailTaken,
        _ => BackendError::StoreQuery {
            operation: "accounts::create".into(),
            message: e.to_string(),
        },
    })?;
    Ok(AccountRow {
        account_id,
        email: email_norm,
        password_hash: password_hash.into(),
        created_at: now,
        last_login_at: None,
    })
}

pub async fn find_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<AccountRow>, BackendError> {
    let email_norm = email.to_lowercase();
    let row = sqlx::query_as::<_, AccountRow>(
        r#"SELECT account_id, email, password_hash, created_at, last_login_at
           FROM accounts WHERE email = ?"#,
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

pub async fn touch_last_login(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<(), BackendError> {
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
        let row = create(&pool, "alice@example.com", "phc")
            .await
            .unwrap();
        touch_last_login(&pool, &row.account_id).await.unwrap();
        let got = find_by_email(&pool, "alice@example.com")
            .await
            .unwrap()
            .unwrap();
        assert!(got.last_login_at.is_some());
    }
}
