use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use minos_protocol::{
    CreateFriendRequestRequest, FriendRequestStatus, FriendRequestSummary, FriendRequestsResponse,
    FriendsResponse,
};

use crate::friends::{DefaultFriendService, FriendError, FriendService};
use crate::http::error_response::{err_response, ErrorEnvelope};
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/friends/query", post(list_friends))
        .route("/friends/requests", post(create_friend_request))
        .route("/friends/requests/query", post(list_friend_requests))
        .route(
            "/friends/requests/:request_id/accept",
            post(accept_friend_request),
        )
        .route(
            "/friends/requests/:request_id/reject",
            post(reject_friend_request),
        )
        .route("/friend-requests", post(create_friend_request))
        .route("/friend-requests/query", post(list_friend_requests))
        .route(
            "/friend-requests/:request_id/accept",
            post(accept_friend_request),
        )
        .route(
            "/friend-requests/:request_id/reject",
            post(reject_friend_request),
        )
}

pub fn external_sql_router() -> Router<BackendState> {
    router()
}

fn err(code: &'static str, message: impl Into<String>) -> (StatusCode, Json<ErrorEnvelope>) {
    err_response(code, message)
}

fn map_friend_error(e: FriendError) -> (StatusCode, Json<ErrorEnvelope>) {
    match e {
        FriendError::TargetNotFound => err("not_found", "target user not found"),
        FriendError::CannotAddSelf => err("bad_request", "cannot add yourself"),
        FriendError::AlreadyFriends => err("conflict", "already friends"),
        FriendError::RequestAlreadyPending => err("conflict", "friend request already pending"),
        FriendError::RequestNotFound => err("not_found", "friend request not found"),
        FriendError::Unauthorized => err("unauthorized", "not allowed to resolve this request"),
        FriendError::AlreadyResolved => err("conflict", "friend request already resolved"),
        FriendError::Internal(e) => err("internal", e.to_string()),
    }
}

async fn create_friend_request(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<CreateFriendRequestRequest>,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let friends = DefaultFriendService::new(state.store.clone());
    let summary = friends
        .create_request(&account_id, &req.target_minos_id)
        .await
        .map_err(map_friend_error)?;
    Ok(Json(summary))
}

async fn list_friend_requests(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<FriendRequestsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let friends = DefaultFriendService::new(state.store.clone());
    let result = friends
        .list_requests(&account_id)
        .await
        .map_err(map_friend_error)?;
    Ok(Json(FriendRequestsResponse {
        incoming: result.incoming,
        outgoing: result.outgoing,
    }))
}

async fn accept_friend_request(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    resolve_request(state, headers, request_id, FriendRequestStatus::Accepted).await
}

async fn reject_friend_request(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    resolve_request(state, headers, request_id, FriendRequestStatus::Rejected).await
}

async fn resolve_request(
    state: BackendState,
    headers: HeaderMap,
    request_id: String,
    status: FriendRequestStatus,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let friends = DefaultFriendService::new(state.store.clone());
    let summary = friends
        .resolve_request(&account_id, &request_id, status)
        .await
        .map_err(map_friend_error)?;
    Ok(Json(summary))
}

async fn list_friends(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<FriendsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let friends = DefaultFriendService::new(state.store.clone());
    let friend_summaries = friends
        .list_friends(&account_id)
        .await
        .map_err(map_friend_error)?;
    Ok(Json(FriendsResponse {
        friends: friend_summaries,
    }))
}
