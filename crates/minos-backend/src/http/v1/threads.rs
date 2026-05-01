//! `GET /v1/threads*` handlers.
//!
//! All three routes require the caller to be authenticated (bearer-bound
//! device row). After ADR-0020 the listing/read APIs scope by the
//! caller's `account_id` (one iOS account may be paired with multiple
//! Macs); the legacy device-keyed pairing lookup has been retired.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use minos_protocol::{
    GetThreadLastSeqResponse, ListThreadsResponse, ReadThreadParams, ReadThreadResponse,
};
use serde::{Deserialize, Serialize};

use crate::auth::bearer;
use crate::http::auth;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/threads", get(list_threads))
        .route("/threads/:thread_id/events", get(read_thread))
        .route("/threads/:thread_id/last_seq", get(get_thread_last_seq))
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

fn err(code: &'static str, message: impl Into<String>) -> Json<ErrorEnvelope> {
    Json(ErrorEnvelope {
        error: ErrorBody {
            code,
            message: message.into(),
        },
    })
}

/// Resolve the caller's authenticated session. Returns the caller's
/// `DeviceId` (used for tracing only) and the bearer's `account_id`, so
/// handlers can scope queries to `owner_device.account_id = account_id`
/// (Phase 2 Task 2.6 / spec §5.5; ADR-0020).
async fn require_authed_session(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<(minos_domain::DeviceId, String), (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = auth::authenticate(&state.store, headers)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, err("unauthorized", m)),
            auth::AuthError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal", m)),
        })?;
    // Phase 2 Task 2.6: require a bearer JWT bound to the same device id;
    // the resolved `account_id` scopes the thread list / read. After
    // ADR-0020 the device-secret check on iOS is dropped — the bearer is
    // the trust root.
    let bearer_outcome = bearer::require(state, headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err("unauthorized", m))
    })?;
    Ok((outcome.device_id, bearer_outcome.account_id))
}

#[derive(Debug, Deserialize)]
struct ListThreadsQuery {
    limit: u32,
    before_ts_ms: Option<i64>,
    agent: Option<minos_domain::AgentName>,
}

async fn list_threads(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Query(q): Query<ListThreadsQuery>,
) -> Result<Json<ListThreadsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = require_authed_session(&state, &headers).await?;
    let threads = crate::store::threads::list(
        &state.store,
        None, // no owner-device filter; account scope below
        q.agent,
        q.before_ts_ms,
        q.limit.min(500),
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
struct ReadThreadQuery {
    from_seq: Option<u64>,
    limit: u32,
}

async fn read_thread(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
    Query(q): Query<ReadThreadQuery>,
) -> Result<Json<ReadThreadResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = require_authed_session(&state, &headers).await?;
    let params = ReadThreadParams {
        thread_id: thread_id.clone(),
        from_seq: q.from_seq,
        limit: q.limit,
    };
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

async fn get_thread_last_seq(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
) -> Result<Json<GetThreadLastSeqResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let _ = require_authed_session(&state, &headers).await?;
    let last_seq = crate::store::raw_events::last_seq(&state.store, &thread_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err("internal", e.to_string()),
            )
        })?;
    Ok(Json(GetThreadLastSeqResponse { last_seq }))
}
