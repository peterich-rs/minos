//! `chat_message_attachments` join table + hydrate helpers.

use sqlx::{PgPool, SqlitePool};

use crate::error::BackendError;
use crate::store::media_blobs::MediaBlobRow;
use crate::store::{AsStorePool, StorePoolRef};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MessageAttachmentJoinRow {
    pub message_id: String,
    pub blob_id: String,
    pub sort_order: i64,
    pub content_type: String,
    pub byte_size: i64,
    pub kind: String,
    pub original_filename: Option<String>,
    pub status: String,
    pub account_id: String,
}

pub async fn link_blobs_to_message<S>(
    store: &S,
    message_id: &str,
    blob_ids: &[String],
) -> Result<(), BackendError>
where
    S: AsStorePool + ?Sized,
{
    if blob_ids.is_empty() {
        return Ok(());
    }
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => link_sqlite(pool, message_id, blob_ids).await,
        StorePoolRef::Postgres(pool) => link_postgres(pool, message_id, blob_ids).await,
    }
}

/// Load attachment metadata for many messages (ordered by sort_order).
pub async fn list_for_messages<S>(
    store: &S,
    message_ids: &[String],
) -> Result<Vec<MessageAttachmentJoinRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => list_sqlite(pool, message_ids).await,
        StorePoolRef::Postgres(pool) => list_postgres(pool, message_ids).await,
    }
}

/// Validate blobs are ready and owned by `account_id`. Returns rows in request order.
pub async fn load_ready_owned_blobs<S>(
    store: &S,
    account_id: &str,
    blob_ids: &[String],
) -> Result<Vec<MediaBlobRow>, BackendError>
where
    S: AsStorePool + ?Sized,
{
    if blob_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(blob_ids.len());
    let mut seen = std::collections::HashSet::new();
    for id in blob_ids {
        let id = id.trim();
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        let row = crate::store::media_blobs::get_by_id(store, id)
            .await?
            .ok_or_else(|| BackendError::StoreQuery {
                operation: "message_attachments.load_blob".into(),
                message: format!("blob {id} not found"),
            })?;
        if row.account_id != account_id {
            return Err(BackendError::StoreQuery {
                operation: "message_attachments.load_blob".into(),
                message: format!("blob {id} not owned by caller"),
            });
        }
        if row.status != "ready" {
            return Err(BackendError::StoreQuery {
                operation: "message_attachments.load_blob".into(),
                message: format!("blob {id} is not ready (status={})", row.status),
            });
        }
        out.push(row);
    }
    Ok(out)
}

async fn link_sqlite(
    pool: &SqlitePool,
    message_id: &str,
    blob_ids: &[String],
) -> Result<(), BackendError> {
    for (i, blob_id) in blob_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO chat_message_attachments (message_id, blob_id, sort_order)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(message_id, blob_id) DO NOTHING",
        )
        .bind(message_id)
        .bind(blob_id)
        .bind(i as i64)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "message_attachments.link".into(),
            message: e.to_string(),
        })?;
    }
    Ok(())
}

async fn link_postgres(
    pool: &PgPool,
    message_id: &str,
    blob_ids: &[String],
) -> Result<(), BackendError> {
    for (i, blob_id) in blob_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO chat_message_attachments (message_id, blob_id, sort_order)
             VALUES ($1, $2, $3)
             ON CONFLICT(message_id, blob_id) DO NOTHING",
        )
        .bind(message_id)
        .bind(blob_id)
        .bind(i as i32)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "message_attachments.link".into(),
            message: e.to_string(),
        })?;
    }
    Ok(())
}

async fn list_sqlite(
    pool: &SqlitePool,
    message_ids: &[String],
) -> Result<Vec<MessageAttachmentJoinRow>, BackendError> {
    let placeholders: Vec<String> = message_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect();
    let sql = format!(
        "SELECT a.message_id, a.blob_id, a.sort_order,
                b.content_type, b.byte_size, b.kind, b.original_filename, b.status, b.account_id
           FROM chat_message_attachments a
           JOIN media_blobs b ON b.blob_id = a.blob_id
          WHERE a.message_id IN ({})
          ORDER BY a.message_id, a.sort_order",
        placeholders.join(", ")
    );
    let mut q = sqlx::query_as::<_, MessageAttachmentJoinRow>(&sql);
    for id in message_ids {
        q = q.bind(id);
    }
    q.fetch_all(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "message_attachments.list".into(),
            message: e.to_string(),
        })
}

async fn list_postgres(
    pool: &PgPool,
    message_ids: &[String],
) -> Result<Vec<MessageAttachmentJoinRow>, BackendError> {
    let placeholders: Vec<String> = message_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("${}", i + 1))
        .collect();
    let sql = format!(
        "SELECT a.message_id, a.blob_id, a.sort_order,
                b.content_type, b.byte_size, b.kind, b.original_filename, b.status, b.account_id
           FROM chat_message_attachments a
           JOIN media_blobs b ON b.blob_id = a.blob_id
          WHERE a.message_id IN ({})
          ORDER BY a.message_id, a.sort_order",
        placeholders.join(", ")
    );
    let mut q = sqlx::query_as::<_, MessageAttachmentJoinRow>(&sql);
    for id in message_ids {
        q = q.bind(id);
    }
    q.fetch_all(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "message_attachments.list".into(),
            message: e.to_string(),
        })
}
