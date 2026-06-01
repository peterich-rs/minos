//! `/v1/threads*` handlers on a POST-first query surface.
//!
//! **DEPRECATED**: These routes are superseded by `/v1/agent-sessions/*`.
//! They will be removed in a future release. Set
//! `MINOS_ENABLE_DEPRECATED_ROUTES=false` to disable them.
//!
//! All three routes require a valid account bearer token. After ADR-0020
//! the listing/read APIs scope by the
//! caller's `account_id` (one iOS account may be paired with multiple
//! Macs); the legacy device-keyed pairing lookup has been retired.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use minos_protocol::{
    GetThreadLastSeqParams, GetThreadLastSeqResponse, ListThreadsParams, ListThreadsResponse,
    ReadThreadParams, ReadThreadResponse,
};
use serde::Deserialize;

use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::BackendState;

/// Route paths served by this deprecated module. Used for startup logging
/// and deprecation metrics.
pub const DEPRECATED_THREAD_ROUTES: &[&str] = &[
    "/v1/threads",
    "/v1/threads/query",
    "/v1/threads/:thread_id/events",
    "/v1/threads/read",
    "/v1/threads/:thread_id/last_seq",
    "/v1/threads/last-seq",
];

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/threads", post(list_threads_query))
        .route("/threads/query", post(list_threads_query))
        .route("/threads/:thread_id/events", post(read_thread_path))
        .route("/threads/read", post(read_thread_query))
        .route(
            "/threads/:thread_id/last_seq",
            post(get_thread_last_seq_path),
        )
        .route("/threads/last-seq", post(get_thread_last_seq_query))
}

async fn list_threads_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<ListThreadsParams>,
) -> Result<Json<ListThreadsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    list_threads_inner(state, headers, params).await
}

async fn list_threads_inner(
    state: BackendState,
    headers: HeaderMap,
    params: ListThreadsParams,
) -> Result<Json<ListThreadsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    let threads = crate::store::threads::list(
        &state.store,
        None, // no owner-device filter; account scope below
        params.agent,
        params.before_ts_ms,
        params.limit.min(500),
        Some(&account_id),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err("internal", e.to_string()),
        )
    })?;
    let next_before_ts_ms = threads.last().map(|t| t.last_ts_ms);
    Ok(Json(ListThreadsResponse {
        threads,
        next_before_ts_ms,
    }))
}

#[derive(Debug, Deserialize)]
struct ReadThreadPathBody {
    from_seq: Option<u64>,
    limit: u32,
}

async fn read_thread_path(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
    Json(body): Json<ReadThreadPathBody>,
) -> Result<Json<ReadThreadResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    read_thread_inner(
        state,
        headers,
        ReadThreadParams {
            thread_id,
            from_seq: body.from_seq,
            limit: body.limit,
        },
    )
    .await
}

async fn read_thread_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<ReadThreadParams>,
) -> Result<Json<ReadThreadResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    read_thread_inner(state, headers, params).await
}

async fn read_thread_inner(
    state: BackendState,
    headers: HeaderMap,
    params: ReadThreadParams,
) -> Result<Json<ReadThreadResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let thread_id = params.thread_id.clone();
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    let resp = crate::ingest::history::read_thread(&state, &account_id, params)
        .await
        .map_err(|e| match e {
            crate::ingest::history::HistoryError::NotFound => (
                StatusCode::NOT_FOUND,
                err("thread_not_found", format!("thread not found: {thread_id}")),
            ),
            crate::ingest::history::HistoryError::Internal(m) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err("internal", m))
            }
        })?;
    Ok(Json(resp))
}

async fn get_thread_last_seq_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<GetThreadLastSeqParams>,
) -> Result<Json<GetThreadLastSeqResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    get_thread_last_seq_inner(state, headers, params).await
}

async fn get_thread_last_seq_path(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
) -> Result<Json<GetThreadLastSeqResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    get_thread_last_seq_inner(state, headers, GetThreadLastSeqParams { thread_id }).await
}

async fn get_thread_last_seq_inner(
    state: BackendState,
    headers: HeaderMap,
    params: GetThreadLastSeqParams,
) -> Result<Json<GetThreadLastSeqResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    // Verify the thread belongs to a device owned by this account.
    let thread_exists =
        crate::store::threads::exists_for_account(&state.store, &params.thread_id, &account_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err("internal", e.to_string()),
                )
            })?;
    if !thread_exists {
        return Err((
            StatusCode::NOT_FOUND,
            err(
                "thread_not_found",
                format!("thread not found: {}", params.thread_id),
            ),
        ));
    }
    let last_seq = crate::store::raw_events::last_seq(&state.store, &params.thread_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err("internal", e.to_string()),
            )
        })?;
    Ok(Json(GetThreadLastSeqResponse { last_seq }))
}
