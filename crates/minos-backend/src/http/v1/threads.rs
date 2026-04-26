//! `GET /v1/threads*` handlers.
//!
//! All three routes require the caller to be an authenticated, paired
//! device. The HTTP handlers scope every query to the caller's pairing
//! peer (the `owner_device_id` on the `threads` row).

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

/// Resolve the caller's authenticated session AND assert it is paired.
/// Returns the *peer* `DeviceId` (the host who owns the threads being
/// read) and the bearer's `account_id`, so handlers can scope queries to
/// `owner_device_id = peer` AND `owner_device.account_id = account_id`
/// (Phase 2 Task 2.6 / spec §5.5).
async fn require_paired_session(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<(minos_domain::DeviceId, String), (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = auth::authenticate(&state.store, headers)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, err("unauthorized", m)),
            auth::AuthError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal", m)),
        })?;
    if !outcome.authenticated_with_secret {
        return Err((
            StatusCode::UNAUTHORIZED,
            err("unauthorized", "X-Device-Secret required"),
        ));
    }
    // Phase 2 Task 2.6: require a bearer JWT bound to the same device id;
    // the resolved `account_id` scopes the thread list / read.
    let bearer_outcome = bearer::require(state, headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err("unauthorized", m))
    })?;
    let owner = crate::store::pairings::get_pair(&state.store, outcome.device_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err("internal", e.to_string()),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                err("unauthorized", "session is not paired"),
            )
        })?;
    Ok((owner, bearer_outcome.account_id))
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
    let (owner, account_id) = require_paired_session(&state, &headers).await?;
    let owner_s = Some(owner.to_string());
    let threads = crate::store::threads::list(
        &state.store,
        owner_s.as_deref(),
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
    let (owner, _account_id) = require_paired_session(&state, &headers).await?;
    let params = ReadThreadParams {
        thread_id: thread_id.clone(),
        from_seq: q.from_seq,
        limit: q.limit,
    };
    let resp = crate::ingest::history::read_thread(&state, owner, params)
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
    let _ = require_paired_session(&state, &headers).await?;
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
