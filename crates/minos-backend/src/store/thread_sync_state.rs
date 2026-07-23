use minos_protocol::realtime::{HostGapManifest, SessionGapManifest};

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

pub async fn upsert_manifest(
    store: &impl AsStorePool,
    manifest: &HostGapManifest,
    now_ms: i64,
) -> Result<(), BackendError> {
    for session in &manifest.sessions {
        upsert_session_manifest(store, &manifest.host_id.to_string(), session, now_ms).await?;
    }
    Ok(())
}

pub async fn mark_backend_acked(
    store: &impl AsStorePool,
    host_device_id: &str,
    session_id: &str,
    accepted_to_seq: u64,
    now_ms: i64,
) -> Result<(), BackendError> {
    let accepted = i64::try_from(accepted_to_seq).unwrap_or(i64::MAX);
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO thread_sync_state( \
                host_device_id, session_id, backend_acked_seq, missing_ranges_json, updated_at_ms \
             ) VALUES (?, ?, ?, '[]', ?) \
             ON CONFLICT(host_device_id, session_id) DO UPDATE SET \
                backend_acked_seq = CASE \
                    WHEN backend_acked_seq < excluded.backend_acked_seq \
                    THEN excluded.backend_acked_seq ELSE backend_acked_seq END, \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(host_device_id)
        .bind(session_id)
        .bind(accepted)
        .bind(now_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO thread_sync_state( \
                host_device_id, session_id, backend_acked_seq, missing_ranges_json, updated_at_ms \
             ) VALUES ($1, $2, $3, '[]', $4) \
             ON CONFLICT(host_device_id, session_id) DO UPDATE SET \
                backend_acked_seq = GREATEST( \
                    thread_sync_state.backend_acked_seq, excluded.backend_acked_seq \
                ), \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(host_device_id)
        .bind(session_id)
        .bind(accepted)
        .bind(now_ms)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("thread_sync_state.mark_backend_acked"))
}

pub async fn backend_acked_seq(
    store: &impl AsStorePool,
    host_device_id: &str,
    session_id: &str,
) -> Result<u64, BackendError> {
    let value: Option<i64> = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar(
                "SELECT backend_acked_seq FROM thread_sync_state \
                 WHERE host_device_id = ?1 AND session_id = ?2",
            )
            .bind(host_device_id)
            .bind(session_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar(
                "SELECT backend_acked_seq FROM thread_sync_state \
                 WHERE host_device_id = $1 AND session_id = $2",
            )
            .bind(host_device_id)
            .bind(session_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("thread_sync_state.backend_acked_seq"))?;
    Ok(value.and_then(|seq| u64::try_from(seq).ok()).unwrap_or(0))
}

async fn upsert_session_manifest(
    store: &impl AsStorePool,
    host_device_id: &str,
    session: &SessionGapManifest,
    now_ms: i64,
) -> Result<(), BackendError> {
    let missing_ranges_json = serde_json::to_string(&session.missing_ranges).map_err(|error| {
        BackendError::StoreQuery {
            operation: "thread_sync_state.serialize_ranges".into(),
            message: error.to_string(),
        }
    })?;
    let backend_acked_seq = i64::try_from(session.backend_acked_seq).unwrap_or(i64::MAX);
    let local_from_seq = i64::try_from(session.local_from_seq).unwrap_or(i64::MAX);
    let local_to_seq = i64::try_from(session.local_to_seq).unwrap_or(i64::MAX);
    let bytes = i64::try_from(session.bytes).unwrap_or(i64::MAX);
    let event_count = i64::try_from(session.event_count).unwrap_or(i64::MAX);
    let running = i64::from(session.running);
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO thread_sync_state( \
                host_device_id, session_id, backend_acked_seq, local_from_seq, local_to_seq, \
                missing_ranges_json, bytes, event_count, first_ts_ms, last_ts_ms, running, updated_at_ms \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(host_device_id, session_id) DO UPDATE SET \
                backend_acked_seq = excluded.backend_acked_seq, \
                local_from_seq = excluded.local_from_seq, \
                local_to_seq = excluded.local_to_seq, \
                missing_ranges_json = excluded.missing_ranges_json, \
                bytes = excluded.bytes, \
                event_count = excluded.event_count, \
                first_ts_ms = excluded.first_ts_ms, \
                last_ts_ms = excluded.last_ts_ms, \
                running = excluded.running, \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(host_device_id)
        .bind(&session.session_id)
        .bind(backend_acked_seq)
        .bind(local_from_seq)
        .bind(local_to_seq)
        .bind(&missing_ranges_json)
        .bind(bytes)
        .bind(event_count)
        .bind(session.first_ts_ms)
        .bind(session.last_ts_ms)
        .bind(running)
        .bind(now_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO thread_sync_state( \
                host_device_id, session_id, backend_acked_seq, local_from_seq, local_to_seq, \
                missing_ranges_json, bytes, event_count, first_ts_ms, last_ts_ms, running, updated_at_ms \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
             ON CONFLICT(host_device_id, session_id) DO UPDATE SET \
                backend_acked_seq = excluded.backend_acked_seq, \
                local_from_seq = excluded.local_from_seq, \
                local_to_seq = excluded.local_to_seq, \
                missing_ranges_json = excluded.missing_ranges_json, \
                bytes = excluded.bytes, \
                event_count = excluded.event_count, \
                first_ts_ms = excluded.first_ts_ms, \
                last_ts_ms = excluded.last_ts_ms, \
                running = excluded.running, \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(host_device_id)
        .bind(&session.session_id)
        .bind(backend_acked_seq)
        .bind(local_from_seq)
        .bind(local_to_seq)
        .bind(&missing_ranges_json)
        .bind(bytes)
        .bind(event_count)
        .bind(session.first_ts_ms)
        .bind(session.last_ts_ms)
        .bind(running)
        .bind(now_ms)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("thread_sync_state.upsert_session_manifest"))
}

fn store_err(operation: &'static str) -> impl Fn(sqlx::Error) -> BackendError {
    move |e| BackendError::StoreQuery {
        operation: operation.into(),
        message: e.to_string(),
    }
}
