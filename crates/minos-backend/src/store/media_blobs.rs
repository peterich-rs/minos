//! `media_blobs` table CRUD. Metadata only — object bytes live in R2 / local store.

use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct MediaBlobRow {
    pub blob_id: String,
    pub account_id: String,
    pub object_key: String,
    pub content_type: String,
    pub byte_size: i64,
    pub sha256_hex: Option<String>,
    pub original_filename: Option<String>,
    pub kind: String,
    pub status: String,
    pub created_at_ms: i64,
    pub ready_at_ms: Option<i64>,
    pub deleted_at_ms: Option<i64>,
}

pub async fn insert_pending<S>(
    store: &S,
    blob_id: &str,
    account_id: &str,
    object_key: &str,
    content_type: &str,
    byte_size: i64,
    original_filename: Option<&str>,
    kind: &str,
    at_ms: i64,
) -> Result<MediaBlobRow, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            insert_pending_sqlite(
                pool,
                blob_id,
                account_id,
                object_key,
                content_type,
                byte_size,
                original_filename,
                kind,
                at_ms,
            )
            .await
        }
        StorePoolRef::Postgres(pool) => {
            insert_pending_postgres(
                pool,
                blob_id,
                account_id,
                object_key,
                content_type,
                byte_size,
                original_filename,
                kind,
                at_ms,
            )
            .await
        }
    }
}

pub async fn get_by_id<S>(store: &S, blob_id: &str) -> Result<Option<MediaBlobRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => get_by_id_sqlite(pool, blob_id).await,
        StorePoolRef::Postgres(pool) => get_by_id_postgres(pool, blob_id).await,
    }
}

pub async fn mark_ready<S>(
    store: &S,
    blob_id: &str,
    byte_size: i64,
    sha256_hex: Option<&str>,
    at_ms: i64,
) -> Result<Option<MediaBlobRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            mark_ready_sqlite(pool, blob_id, byte_size, sha256_hex, at_ms).await
        }
        StorePoolRef::Postgres(pool) => {
            mark_ready_postgres(pool, blob_id, byte_size, sha256_hex, at_ms).await
        }
    }
}

pub async fn mark_failed<S>(store: &S, blob_id: &str, at_ms: i64) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => mark_status_sqlite(pool, blob_id, "failed", at_ms).await,
        StorePoolRef::Postgres(pool) => mark_status_postgres(pool, blob_id, "failed", at_ms).await,
    }
}

pub async fn soft_delete<S>(
    store: &S,
    blob_id: &str,
    at_ms: i64,
) -> Result<Option<MediaBlobRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => soft_delete_sqlite(pool, blob_id, at_ms).await,
        StorePoolRef::Postgres(pool) => soft_delete_postgres(pool, blob_id, at_ms).await,
    }
}

// ── SQLite ─────────────────────────────────────────────────────────────

async fn insert_pending_sqlite(
    pool: &SqlitePool,
    blob_id: &str,
    account_id: &str,
    object_key: &str,
    content_type: &str,
    byte_size: i64,
    original_filename: Option<&str>,
    kind: &str,
    at_ms: i64,
) -> Result<MediaBlobRow, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>(
        "INSERT INTO media_blobs (
            blob_id, account_id, object_key, content_type, byte_size,
            sha256_hex, original_filename, kind, status, created_at_ms, ready_at_ms, deleted_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, 'pending', ?8, NULL, NULL)
         RETURNING *",
    )
    .bind(blob_id)
    .bind(account_id)
    .bind(object_key)
    .bind(content_type)
    .bind(byte_size)
    .bind(original_filename)
    .bind(kind)
    .bind(at_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "media_blobs.insert_pending".into(),
        message: e.to_string(),
    })
}

async fn get_by_id_sqlite(
    pool: &SqlitePool,
    blob_id: &str,
) -> Result<Option<MediaBlobRow>, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>("SELECT * FROM media_blobs WHERE blob_id = ?1")
        .bind(blob_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "media_blobs.get_by_id".into(),
            message: e.to_string(),
        })
}

async fn mark_ready_sqlite(
    pool: &SqlitePool,
    blob_id: &str,
    byte_size: i64,
    sha256_hex: Option<&str>,
    at_ms: i64,
) -> Result<Option<MediaBlobRow>, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>(
        "UPDATE media_blobs
         SET status = 'ready', byte_size = ?1, sha256_hex = ?2, ready_at_ms = ?3
         WHERE blob_id = ?4 AND status = 'pending'
         RETURNING *",
    )
    .bind(byte_size)
    .bind(sha256_hex)
    .bind(at_ms)
    .bind(blob_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "media_blobs.mark_ready".into(),
        message: e.to_string(),
    })
}

async fn mark_status_sqlite(
    pool: &SqlitePool,
    blob_id: &str,
    status: &str,
    at_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query("UPDATE media_blobs SET status = ?1 WHERE blob_id = ?2 AND status = 'pending'")
        .bind(status)
        .bind(blob_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "media_blobs.mark_status".into(),
            message: e.to_string(),
        })?;
    let _ = at_ms;
    Ok(())
}

async fn soft_delete_sqlite(
    pool: &SqlitePool,
    blob_id: &str,
    at_ms: i64,
) -> Result<Option<MediaBlobRow>, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>(
        "UPDATE media_blobs
         SET status = 'deleted', deleted_at_ms = ?1
         WHERE blob_id = ?2 AND status IN ('pending', 'ready', 'failed')
         RETURNING *",
    )
    .bind(at_ms)
    .bind(blob_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "media_blobs.soft_delete".into(),
        message: e.to_string(),
    })
}

// ── Postgres ───────────────────────────────────────────────────────────

async fn insert_pending_postgres(
    pool: &PgPool,
    blob_id: &str,
    account_id: &str,
    object_key: &str,
    content_type: &str,
    byte_size: i64,
    original_filename: Option<&str>,
    kind: &str,
    at_ms: i64,
) -> Result<MediaBlobRow, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>(
        "INSERT INTO media_blobs (
            blob_id, account_id, object_key, content_type, byte_size,
            sha256_hex, original_filename, kind, status, created_at_ms, ready_at_ms, deleted_at_ms
         ) VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, 'pending', $8, NULL, NULL)
         RETURNING *",
    )
    .bind(blob_id)
    .bind(account_id)
    .bind(object_key)
    .bind(content_type)
    .bind(byte_size)
    .bind(original_filename)
    .bind(kind)
    .bind(at_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "media_blobs.insert_pending".into(),
        message: e.to_string(),
    })
}

async fn get_by_id_postgres(
    pool: &PgPool,
    blob_id: &str,
) -> Result<Option<MediaBlobRow>, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>("SELECT * FROM media_blobs WHERE blob_id = $1")
        .bind(blob_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "media_blobs.get_by_id".into(),
            message: e.to_string(),
        })
}

async fn mark_ready_postgres(
    pool: &PgPool,
    blob_id: &str,
    byte_size: i64,
    sha256_hex: Option<&str>,
    at_ms: i64,
) -> Result<Option<MediaBlobRow>, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>(
        "UPDATE media_blobs
         SET status = 'ready', byte_size = $1, sha256_hex = $2, ready_at_ms = $3
         WHERE blob_id = $4 AND status = 'pending'
         RETURNING *",
    )
    .bind(byte_size)
    .bind(sha256_hex)
    .bind(at_ms)
    .bind(blob_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "media_blobs.mark_ready".into(),
        message: e.to_string(),
    })
}

async fn mark_status_postgres(
    pool: &PgPool,
    blob_id: &str,
    status: &str,
    _at_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query("UPDATE media_blobs SET status = $1 WHERE blob_id = $2 AND status = 'pending'")
        .bind(status)
        .bind(blob_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "media_blobs.mark_status".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

async fn soft_delete_postgres(
    pool: &PgPool,
    blob_id: &str,
    at_ms: i64,
) -> Result<Option<MediaBlobRow>, BackendError> {
    sqlx::query_as::<_, MediaBlobRow>(
        "UPDATE media_blobs
         SET status = 'deleted', deleted_at_ms = $1
         WHERE blob_id = $2 AND status IN ('pending', 'ready', 'failed')
         RETURNING *",
    )
    .bind(at_ms)
    .bind(blob_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::StoreQuery {
        operation: "media_blobs.soft_delete".into(),
        message: e.to_string(),
    })
}
