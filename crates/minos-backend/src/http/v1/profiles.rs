use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use minos_protocol::{
    MyProfileResponse, SearchUsersRequest, SearchUsersResponse, SetDisplayNameRequest,
    SetMinosIdRequest,
};

use crate::http::error_response::{err_response, ErrorEnvelope};
use crate::http::BackendState;
use crate::profiles::{ProfileError, ProfileService};

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/profiles/self", post(get_my_profile))
        .route("/profiles/minos-id", post(set_minos_id))
        .route("/profiles/display-name", post(set_display_name))
        .route("/profiles/search", post(search_users_query))
}

pub fn external_sql_router() -> Router<BackendState> {
    router()
}

fn err(code: &'static str, message: impl Into<String>) -> (StatusCode, Json<ErrorEnvelope>) {
    err_response(code, message)
}

fn map_profile_error(e: ProfileError) -> (StatusCode, Json<ErrorEnvelope>) {
    match e {
        ProfileError::NotFound => err("not_found", "profile not found"),
        ProfileError::MinosIdTaken => err("conflict", "minos_id already taken"),
        ProfileError::ValidationFormat(msg) => err("bad_request", msg),
        ProfileError::Internal(e) => err("internal", e.to_string()),
    }
}

#[allow(clippy::unused_async)]
async fn get_my_profile(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<MyProfileResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = crate::http::v1::social::require_account_id_from_state(&state, &headers)?;
    let profiles = crate::profiles::DefaultProfileService::new(state.store.clone());
    let profile = profiles
        .get_my_profile(&account_id)
        .await
        .map_err(map_profile_error)?;
    Ok(Json(MyProfileResponse {
        account_id: profile.account_id,
        email: profile.email,
        minos_id: profile.minos_id,
        display_name: profile.display_name,
    }))
}

#[allow(clippy::unused_async)]
async fn set_minos_id(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<SetMinosIdRequest>,
) -> Result<Json<MyProfileResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = crate::http::v1::social::require_account_id_from_state(&state, &headers)?;
    let profiles = crate::profiles::DefaultProfileService::new(state.store.clone());
    let profile = profiles
        .set_minos_id(&account_id, &req.minos_id)
        .await
        .map_err(map_profile_error)?;
    Ok(Json(MyProfileResponse {
        account_id: profile.account_id,
        email: profile.email,
        minos_id: profile.minos_id,
        display_name: profile.display_name,
    }))
}

#[allow(clippy::unused_async)]
async fn set_display_name(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<SetDisplayNameRequest>,
) -> Result<Json<MyProfileResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = crate::http::v1::social::require_account_id_from_state(&state, &headers)?;
    let profiles = crate::profiles::DefaultProfileService::new(state.store.clone());
    let profile = profiles
        .set_display_name(&account_id, req.display_name.as_deref())
        .await
        .map_err(map_profile_error)?;
    Ok(Json(MyProfileResponse {
        account_id: profile.account_id,
        email: profile.email,
        minos_id: profile.minos_id,
        display_name: profile.display_name,
    }))
}

#[allow(clippy::unused_async)]
async fn search_users_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(query): Json<SearchUsersRequest>,
) -> Result<Json<SearchUsersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = crate::http::v1::social::require_account_id_from_state(&state, &headers)?;
    let profiles = crate::profiles::DefaultProfileService::new(state.store.clone());
    let users = profiles
        .search_users(&query.minos_id, &account_id)
        .await
        .map_err(map_profile_error)?;
    Ok(Json(SearchUsersResponse { users }))
}
