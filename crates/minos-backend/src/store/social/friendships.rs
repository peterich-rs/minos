use sqlx::{Executor, Postgres, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use minos_protocol::FriendRequestStatus;

use super::{
    friend_request_status_str, normalized_pair, store_err, FriendRequestRow, FriendshipRow,
    ResolveFriendRequestTxResult,
};

pub async fn create_friend_request(
    store: &impl AsStorePool,
    from_account_id: &str,
    to_account_id: &str,
    created_at_ms: i64,
) -> Result<String, BackendError> {
    let request_id = Uuid::new_v4().to_string();
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO friend_requests
                    (request_id, from_account_id, to_account_id, status, created_at_ms)
                 VALUES (?, ?, ?, 'pending', ?)",
        )
        .bind(&request_id)
        .bind(from_account_id)
        .bind(to_account_id)
        .bind(created_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO friend_requests
                    (request_id, from_account_id, to_account_id, status, created_at_ms)
                 VALUES ($1, $2, $3, 'pending', $4)",
        )
        .bind(&request_id)
        .bind(from_account_id)
        .bind(to_account_id)
        .bind(created_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("social::create_friend_request"))?;

    Ok(request_id)
}

pub async fn get_friend_request(
    store: &impl AsStorePool,
    request_id: &str,
) -> Result<Option<FriendRequestRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE request_id = ?",
            )
            .bind(request_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE request_id = $1",
            )
            .bind(request_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::get_friend_request"))
}

pub async fn list_incoming_friend_requests(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Vec<FriendRequestRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE to_account_id = ?
                  ORDER BY created_at_ms DESC",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE to_account_id = $1
                  ORDER BY created_at_ms DESC",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_incoming_friend_requests"))
}

pub async fn list_outgoing_friend_requests(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Vec<FriendRequestRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE from_account_id = ?
                  ORDER BY created_at_ms DESC",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE from_account_id = $1
                  ORDER BY created_at_ms DESC",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_outgoing_friend_requests"))
}

pub async fn has_pending_friend_request_between(
    store: &impl AsStorePool,
    left: &str,
    right: &str,
) -> Result<bool, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM friend_requests
                  WHERE status = 'pending'
                    AND ((from_account_id = ? AND to_account_id = ?) OR
                         (from_account_id = ? AND to_account_id = ?))",
            )
            .bind(left)
            .bind(right)
            .bind(right)
            .bind(left)
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM friend_requests
                  WHERE status = 'pending'
                    AND ((from_account_id = $1 AND to_account_id = $2) OR
                         (from_account_id = $3 AND to_account_id = $4))",
            )
            .bind(left)
            .bind(right)
            .bind(right)
            .bind(left)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("social::has_pending_friend_request_between"))?;

    Ok(row > 0)
}

pub async fn resolve_friend_request(
    store: &impl AsStorePool,
    request_id: &str,
    status: FriendRequestStatus,
    resolved_at_ms: i64,
) -> Result<bool, BackendError> {
    let status = friend_request_status_str(status);
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE friend_requests
                    SET status = ?, resolved_at_ms = ?
                  WHERE request_id = ? AND status = 'pending'",
        )
        .bind(status)
        .bind(resolved_at_ms)
        .bind(request_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE friend_requests
                    SET status = $1, resolved_at_ms = $2
                  WHERE request_id = $3 AND status = 'pending'",
        )
        .bind(status)
        .bind(resolved_at_ms)
        .bind(request_id)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("social::resolve_friend_request"))?;

    Ok(result == 1)
}

pub async fn resolve_friend_request_transactional(
    store: &impl AsStorePool,
    acting_account_id: &str,
    request_id: &str,
    status: FriendRequestStatus,
    resolved_at_ms: i64,
) -> Result<ResolveFriendRequestTxResult, BackendError> {
    let status = friend_request_status_str(status);

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx = pool.begin().await.map_err(store_err(
                "social::resolve_friend_request_transactional.begin",
            ))?;

            let Some(existing) = sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE request_id = ?",
            )
            .bind(request_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_err(
                "social::resolve_friend_request_transactional.load",
            ))?
            else {
                tx.rollback().await.ok();
                return Ok(ResolveFriendRequestTxResult::NotFound);
            };

            if existing.to_account_id != acting_account_id {
                tx.rollback().await.ok();
                return Ok(ResolveFriendRequestTxResult::Unauthorized);
            }
            if existing.status != "pending" {
                tx.rollback().await.ok();
                return Ok(ResolveFriendRequestTxResult::AlreadyResolved);
            }

            sqlx::query(
                "UPDATE friend_requests
                    SET status = ?, resolved_at_ms = ?
                  WHERE request_id = ? AND status = 'pending'",
            )
            .bind(status)
            .bind(resolved_at_ms)
            .bind(request_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err(
                "social::resolve_friend_request_transactional.update",
            ))?;

            if status == "accepted" {
                create_friendship_with_sqlite_executor(
                    &mut *tx,
                    &existing.from_account_id,
                    &existing.to_account_id,
                    resolved_at_ms,
                )
                .await
                .map_err(|error| BackendError::StoreQuery {
                    operation: "social::resolve_friend_request_transactional.create_friendship"
                        .into(),
                    message: error.to_string(),
                })?;
            }

            let row = sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE request_id = ?",
            )
            .bind(request_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_err(
                "social::resolve_friend_request_transactional.reload",
            ))?;

            tx.commit().await.map_err(store_err(
                "social::resolve_friend_request_transactional.commit",
            ))?;

            Ok(ResolveFriendRequestTxResult::Resolved(row))
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool.begin().await.map_err(store_err(
                "social::resolve_friend_request_transactional.begin",
            ))?;

            let Some(existing) = sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE request_id = $1
                  FOR UPDATE",
            )
            .bind(request_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_err(
                "social::resolve_friend_request_transactional.load",
            ))?
            else {
                tx.rollback().await.ok();
                return Ok(ResolveFriendRequestTxResult::NotFound);
            };

            if existing.to_account_id != acting_account_id {
                tx.rollback().await.ok();
                return Ok(ResolveFriendRequestTxResult::Unauthorized);
            }
            if existing.status != "pending" {
                tx.rollback().await.ok();
                return Ok(ResolveFriendRequestTxResult::AlreadyResolved);
            }

            sqlx::query(
                "UPDATE friend_requests
                    SET status = $1, resolved_at_ms = $2
                  WHERE request_id = $3 AND status = 'pending'",
            )
            .bind(status)
            .bind(resolved_at_ms)
            .bind(request_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err(
                "social::resolve_friend_request_transactional.update",
            ))?;

            if status == "accepted" {
                create_friendship_with_postgres_executor(
                    &mut *tx,
                    &existing.from_account_id,
                    &existing.to_account_id,
                    resolved_at_ms,
                )
                .await
                .map_err(|error| BackendError::StoreQuery {
                    operation: "social::resolve_friend_request_transactional.create_friendship"
                        .into(),
                    message: error.to_string(),
                })?;
            }

            let row = sqlx::query_as::<_, FriendRequestRow>(
                "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
                   FROM friend_requests
                  WHERE request_id = $1",
            )
            .bind(request_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_err(
                "social::resolve_friend_request_transactional.reload",
            ))?;

            tx.commit().await.map_err(store_err(
                "social::resolve_friend_request_transactional.commit",
            ))?;

            Ok(ResolveFriendRequestTxResult::Resolved(row))
        }
    }
}

pub(crate) async fn create_friendship_with_sqlite_executor<'e, E>(
    executor: E,
    left: &str,
    right: &str,
    created_at_ms: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let (low, high) = normalized_pair(left, right);
    sqlx::query(
        "INSERT OR IGNORE INTO friendships
            (friendship_id, account_low_id, account_high_id, created_at_ms)
         VALUES (?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(low)
    .bind(high)
    .bind(created_at_ms)
    .execute(executor)
    .await
    .map_err(store_err("social::create_friendship"))?;
    Ok(())
}

pub(crate) async fn create_friendship_with_postgres_executor<'e, E>(
    executor: E,
    left: &str,
    right: &str,
    created_at_ms: i64,
) -> Result<(), BackendError>
where
    E: Executor<'e, Database = Postgres>,
{
    let (low, high) = normalized_pair(left, right);
    sqlx::query(
        "INSERT INTO friendships
            (friendship_id, account_low_id, account_high_id, created_at_ms)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(low)
    .bind(high)
    .bind(created_at_ms)
    .execute(executor)
    .await
    .map_err(store_err("social::create_friendship"))?;
    Ok(())
}

pub async fn create_friendship(
    store: &impl AsStorePool,
    left: &str,
    right: &str,
    created_at_ms: i64,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            create_friendship_with_sqlite_executor(pool, left, right, created_at_ms).await
        }
        StorePoolRef::Postgres(pool) => {
            create_friendship_with_postgres_executor(pool, left, right, created_at_ms).await
        }
    }
}

pub async fn are_friends(
    store: &impl AsStorePool,
    left: &str,
    right: &str,
) -> Result<bool, BackendError> {
    let (low, high) = normalized_pair(left, right);
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM friendships
                  WHERE account_low_id = ? AND account_high_id = ?",
            )
            .bind(low)
            .bind(high)
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM friendships
                  WHERE account_low_id = $1 AND account_high_id = $2",
            )
            .bind(low)
            .bind(high)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("social::are_friends"))?;

    Ok(row > 0)
}

pub async fn list_friendships_for(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Vec<FriendshipRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, FriendshipRow>(
                "SELECT friendship_id, account_low_id, account_high_id, created_at_ms
                   FROM friendships
                  WHERE account_low_id = ? OR account_high_id = ?
                  ORDER BY created_at_ms DESC",
            )
            .bind(account_id)
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, FriendshipRow>(
                "SELECT friendship_id, account_low_id, account_high_id, created_at_ms
                   FROM friendships
                  WHERE account_low_id = $1 OR account_high_id = $2
                  ORDER BY created_at_ms DESC",
            )
            .bind(account_id)
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_friendships_for"))
}
