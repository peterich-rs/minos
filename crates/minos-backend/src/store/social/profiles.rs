use std::collections::HashMap;

use sqlx::{Postgres, QueryBuilder, Sqlite};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::{store_err, ProfileRow};

pub async fn profile_by_account(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Option<ProfileRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ProfileRow>(
                "SELECT account_id, email, minos_id, display_name
                   FROM accounts
                  WHERE account_id = ?",
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ProfileRow>(
                "SELECT account_id, email, minos_id, display_name
                   FROM accounts
                  WHERE account_id = $1",
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::profile_by_account"))
}

/// Batch-load profiles for multiple account IDs in a single query.
/// Returns a map from account_id to ProfileRow. Missing accounts are
/// silently omitted from the result.
pub async fn profiles_by_accounts(
    store: &impl AsStorePool,
    account_ids: &[String],
) -> Result<HashMap<String, ProfileRow>, BackendError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT account_id, email, minos_id, display_name FROM accounts WHERE account_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for id in account_ids {
                    separated.push_bind(id);
                }
            }
            builder.push(')');
            builder
                .build_query_as::<ProfileRow>()
                .fetch_all(pool)
                .await
        }
        StorePoolRef::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(
                "SELECT account_id, email, minos_id, display_name FROM accounts WHERE account_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for id in account_ids {
                    separated.push_bind(id);
                }
            }
            builder.push(')');
            builder
                .build_query_as::<ProfileRow>()
                .fetch_all(pool)
                .await
        }
    }
    .map_err(store_err("social::profiles_by_accounts"))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.account_id.clone(), r))
        .collect())
}

pub async fn find_by_minos_id(
    store: &impl AsStorePool,
    minos_id: &str,
) -> Result<Option<ProfileRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ProfileRow>(
                "SELECT account_id, email, minos_id, display_name
                   FROM accounts
                  WHERE minos_id = ? COLLATE BINARY",
            )
            .bind(minos_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ProfileRow>(
                "SELECT account_id, email, minos_id, display_name
                   FROM accounts
                  WHERE minos_id = $1",
            )
            .bind(minos_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::find_by_minos_id"))
}

pub async fn search_by_minos_id_prefix(
    store: &impl AsStorePool,
    query: &str,
) -> Result<Vec<ProfileRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ProfileRow>(
                "SELECT account_id, email, minos_id, display_name
                   FROM accounts
                  WHERE substr(minos_id, 1, length(?)) = ?
                  ORDER BY CASE WHEN minos_id = ? THEN 0 ELSE 1 END, minos_id
                  LIMIT 20",
            )
            .bind(query)
            .bind(query)
            .bind(query)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ProfileRow>(
                "SELECT account_id, email, minos_id, display_name
                   FROM accounts
                  WHERE minos_id LIKE ($1 || '%')
                  ORDER BY CASE WHEN minos_id = $1 THEN 0 ELSE 1 END, minos_id
                  LIMIT 20",
            )
            .bind(query)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::search_by_minos_id_prefix"))
}

pub async fn set_minos_id(
    store: &impl AsStorePool,
    account_id: &str,
    minos_id: &str,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query("UPDATE accounts SET minos_id = ? WHERE account_id = ?")
                .bind(minos_id)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query("UPDATE accounts SET minos_id = $1 WHERE account_id = $2")
                .bind(minos_id)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
    }
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => BackendError::StoreQuery {
            operation: "social::set_minos_id".into(),
            message: "minos_id_taken".into(),
        },
        _ => BackendError::StoreQuery {
            operation: "social::set_minos_id".into(),
            message: e.to_string(),
        },
    })?;
    Ok(())
}

pub async fn set_display_name(
    store: &impl AsStorePool,
    account_id: &str,
    display_name: Option<&str>,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query("UPDATE accounts SET display_name = ? WHERE account_id = ?")
                .bind(display_name)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query("UPDATE accounts SET display_name = $1 WHERE account_id = $2")
                .bind(display_name)
                .bind(account_id)
                .execute(pool)
                .await
                .map(|_| ())
        }
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "social::set_display_name".into(),
        message: e.to_string(),
    })?;
    Ok(())
}
