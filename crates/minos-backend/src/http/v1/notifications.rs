//! `/v1/notifications/*` HTTP routes.
//!
//! - `POST /v1/notifications/tokens/register` — register a push token
//! - `POST /v1/notifications/tokens/unregister` — unregister a push token
//! - `POST /v1/notifications/preferences/get` — get notification preferences
//! - `POST /v1/notifications/preferences/update` — update notification preferences

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::BackendState;
use crate::notifications::channels::PushKind;
use crate::notifications::{
    NotificationError, RegisterTokenInput, UnregisterTokenInput, UpdatePreferencesInput,
};

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/notifications/tokens/register", post(register_token))
        .route("/notifications/tokens/unregister", post(unregister_token))
        .route("/notifications/tokens/list", post(list_tokens))
        .route("/notifications/preferences/get", post(get_preferences))
        .route(
            "/notifications/preferences/update",
            post(update_preferences),
        )
}

// ── Request / Response types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RegisterTokenRequest {
    installation_id: String,
    kind: String,
    token: String,
    locale: Option<String>,
}

#[derive(Debug, Serialize)]
struct RegisterTokenResponse {
    ok: bool,
}

#[derive(Debug, Deserialize)]
struct UnregisterTokenRequest {
    token: String,
}

#[derive(Debug, Serialize)]
struct UnregisterTokenResponse {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct ListTokensResponse {
    tokens: Vec<crate::notifications::PushTokenDto>,
}

#[derive(Debug, Deserialize)]
struct GetPreferencesRequest {
    // Empty — account_id comes from auth
}

#[derive(Debug, Serialize)]
struct GetPreferencesResponse {
    preferences: crate::notifications::NotificationPreferences,
}

#[derive(Debug, Deserialize)]
struct UpdatePreferencesRequest {
    direct_message_enabled: Option<bool>,
    group_mention_enabled: Option<bool>,
    approval_required_enabled: Option<bool>,
    agent_session_ended_enabled: Option<bool>,
    quiet_hours_start_minute: Option<Option<i16>>,
    quiet_hours_end_minute: Option<Option<i16>>,
    quiet_hours_timezone: Option<Option<String>>,
}

#[derive(Debug, Serialize)]
struct UpdatePreferencesResponse {
    preferences: crate::notifications::NotificationPreferences,
}

// ── Handlers ───────────────────────────────────────────────────────────

async fn register_token(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<RegisterTokenRequest>,
) -> Result<Json<RegisterTokenResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;

    let kind: PushKind = request.kind.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            err("invalid_push_kind", "kind must be 'apns' or 'fcm'"),
        )
    })?;

    state
        .notifications
        .register_token(RegisterTokenInput {
            account_id,
            installation_id: request.installation_id,
            kind,
            token: request.token,
            locale: request.locale,
        })
        .await
        .map_err(map_notification_error)?;

    Ok(Json(RegisterTokenResponse { ok: true }))
}

async fn unregister_token(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<UnregisterTokenRequest>,
) -> Result<Json<UnregisterTokenResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;

    state
        .notifications
        .unregister_token(UnregisterTokenInput {
            account_id,
            token: request.token,
        })
        .await
        .map_err(map_notification_error)?;

    Ok(Json(UnregisterTokenResponse { ok: true }))
}

async fn list_tokens(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ListTokensResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;

    let tokens = state
        .notifications
        .list_tokens(&account_id)
        .await
        .map_err(map_notification_error)?;

    Ok(Json(ListTokensResponse { tokens }))
}

async fn get_preferences(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(_request): Json<GetPreferencesRequest>,
) -> Result<Json<GetPreferencesResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;

    let preferences = state
        .notifications
        .get_preferences(&account_id)
        .await
        .map_err(map_notification_error)?;

    Ok(Json(GetPreferencesResponse { preferences }))
}

async fn update_preferences(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<UpdatePreferencesRequest>,
) -> Result<Json<UpdatePreferencesResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;

    let preferences = state
        .notifications
        .update_preferences(UpdatePreferencesInput {
            account_id,
            direct_message_enabled: request.direct_message_enabled,
            group_mention_enabled: request.group_mention_enabled,
            approval_required_enabled: request.approval_required_enabled,
            agent_session_ended_enabled: request.agent_session_ended_enabled,
            quiet_hours_start_minute: request.quiet_hours_start_minute,
            quiet_hours_end_minute: request.quiet_hours_end_minute,
            quiet_hours_timezone: request.quiet_hours_timezone,
        })
        .await
        .map_err(map_notification_error)?;

    Ok(Json(UpdatePreferencesResponse { preferences }))
}

fn map_notification_error(error: NotificationError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        NotificationError::InvalidToken(msg) => {
            (StatusCode::BAD_REQUEST, err("invalid_push_token", msg))
        }
        NotificationError::TokenNotFound => (
            StatusCode::NOT_FOUND,
            err("token_not_found", "push token not found"),
        ),
        NotificationError::Internal(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            err("internal", e.to_string()),
        ),
    }
}
