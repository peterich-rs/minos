//! `/v1/auth/{supabase,refresh,logout}` HTTP handlers.
//!
//! Human account create/login is **Supabase-only** via `POST /v1/auth/supabase`.
//! Password register/login/change-password endpoints have been removed.
//!
//! Supabase exchange does **not** call `authenticate()` / device-secret;
//! it only requires `X-Device-Id` (+ optional role/name) and a Supabase
//! access token body.
//!
//! Logout additionally requires `Authorization: Bearer <jwt>` because
//! the act of revoking a refresh token must be authenticated by the
//! account that owns it.

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use minos_domain::DeviceRole;
use serde::{Deserialize, Serialize};

use crate::auth::bearer;
use crate::auth::use_case::{AuthSession, AuthUseCaseError, RefreshSession};
use crate::http::auth::{
    authenticate, extract_device_id, extract_device_name, extract_device_role,
};
use crate::http::error_response::{err_json, ErrorEnvelope};
use crate::http::BackendState;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RefreshReq {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LogoutReq {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AccountSummary {
    pub account_id: String,
    pub email: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AuthResp {
    pub account: AccountSummary,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RefreshResp {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

fn err(code: &'static str) -> Json<ErrorEnvelope> {
    err_json(code, code)
}

/// Pull the client IP from `X-Forwarded-For`. We trust the upstream
/// reverse proxy to set this — direct internet exposure of the backend
/// is not a supported deployment per the spec. When missing, fall back
/// to the literal `"unknown"` so the bucket key is still stable per
/// request (i.e. a flood from a misconfigured upstream still gets
/// rate-limited as one bucket).
fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// 429 with `Retry-After`. Returned as a custom `Response` so we can
/// emit the header alongside the JSON body.
fn rate_limited_response(retry: u32) -> Response {
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, err("rate_limited")).into_response();
    resp.headers_mut().insert(
        "Retry-After",
        HeaderValue::from_str(&retry.to_string()).unwrap_or_else(|_| HeaderValue::from_static("1")),
    );
    resp
}

fn auth_error_response(error: AuthUseCaseError) -> Response {
    match error {
        AuthUseCaseError::EmailTaken => (StatusCode::CONFLICT, err("email_taken")).into_response(),
        AuthUseCaseError::InvalidRefresh => {
            (StatusCode::UNAUTHORIZED, err("invalid_refresh")).into_response()
        }
        AuthUseCaseError::WsTicketAccountMismatch => {
            (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response()
        }
        AuthUseCaseError::Internal => {
            (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response()
        }
        AuthUseCaseError::RateLimited { retry_after_secs } => {
            rate_limited_response(retry_after_secs)
        }
        AuthUseCaseError::UnsupportedWsTicketRole => {
            (StatusCode::BAD_REQUEST, err("ws_ticket_unsupported_role")).into_response()
        }
        AuthUseCaseError::SupabaseNotConfigured => (
            StatusCode::SERVICE_UNAVAILABLE,
            err("supabase_not_configured"),
        )
            .into_response(),
        AuthUseCaseError::InvalidSupabaseToken => {
            (StatusCode::UNAUTHORIZED, err("invalid_supabase_token")).into_response()
        }
        AuthUseCaseError::SupabaseTokenExpired => {
            (StatusCode::UNAUTHORIZED, err("supabase_token_expired")).into_response()
        }
        AuthUseCaseError::SupabaseTokenInvalid => {
            (StatusCode::UNAUTHORIZED, err("supabase_token_invalid")).into_response()
        }
        AuthUseCaseError::IdpUnavailable => {
            (StatusCode::SERVICE_UNAVAILABLE, err("idp_unavailable")).into_response()
        }
        AuthUseCaseError::MergeConflict => {
            (StatusCode::CONFLICT, err("merge_conflict")).into_response()
        }
    }
}

fn auth_session_response(session: AuthSession) -> Response {
    (
        StatusCode::OK,
        Json(AuthResp {
            account: AccountSummary {
                account_id: session.account_id,
                email: session.email,
            },
            access_token: session.access_token,
            refresh_token: session.refresh_token,
            expires_in: session.expires_in,
        }),
    )
        .into_response()
}

fn refresh_session_response(session: RefreshSession) -> Response {
    Json(RefreshResp {
        access_token: session.access_token,
        refresh_token: session.refresh_token,
        expires_in: session.expires_in,
    })
    .into_response()
}

/// Refresh an access token using a refresh token.
#[utoipa::path(
    post,
    path = "/v1/auth/refresh",
    request_body = RefreshReq,
    responses(
        (status = 200, description = "Token refreshed", body = RefreshResp),
        (status = 401, description = "Invalid refresh token", body = ErrorEnvelope),
    ),
    tag = "auth"
)]
#[tracing::instrument(skip_all)]
pub async fn post_refresh(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RefreshReq>,
) -> Response {
    let Ok(outcome) = authenticate(&state.store, &headers).await else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    match state
        .auth
        .refresh(outcome.device_id, &req.refresh_token)
        .await
    {
        Ok(session) => refresh_session_response(session),
        Err(error) => auth_error_response(error),
    }
}

/// Logout and revoke refresh token.
#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    request_body = LogoutReq,
    responses(
        (status = 204, description = "Logged out"),
        (status = 401, description = "Unauthorized", body = ErrorEnvelope),
    ),
    tag = "auth"
)]
#[tracing::instrument(skip_all)]
pub async fn post_logout(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LogoutReq>,
) -> Response {
    if authenticate(&state.store, &headers).await.is_err() {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    }
    let Ok(bearer_outcome) = bearer::require(&state, &headers) else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    match state
        .auth
        .logout(&bearer_outcome.account_id, &req.refresh_token)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => auth_error_response(error),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SupabaseExchangeReq {
    /// Supabase Auth access token (JWT) from the client SDK session.
    pub access_token: String,
    /// Optional human label; overrides `X-Device-Name` when set.
    #[serde(default)]
    pub device_name: Option<String>,
}

/// Exchange a Supabase access token for Minos access/refresh tokens.
///
/// Does **not** require `X-Device-Secret`. Requires `X-Device-Id` (UUID).
/// This is the only human account create/login path.
#[utoipa::path(
    post,
    path = "/v1/auth/supabase",
    request_body = SupabaseExchangeReq,
    responses(
        (status = 200, description = "Exchange successful", body = AuthResp),
        (status = 401, description = "Invalid Supabase token", body = ErrorEnvelope),
        (status = 409, description = "Merge conflict", body = ErrorEnvelope),
        (status = 429, description = "Rate limited"),
        (status = 503, description = "IdP unavailable or not configured", body = ErrorEnvelope),
    ),
    tag = "auth"
)]
#[tracing::instrument(skip_all)]
pub async fn post_supabase_exchange(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<SupabaseExchangeReq>,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response(),
    };
    let device_role = match extract_device_role(&headers) {
        Ok(Some(role)) => role,
        Ok(None) => DeviceRole::BrowserAdmin,
        Err(_) => return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response(),
    };
    let header_name = extract_device_name(&headers);
    let device_name = req
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or(header_name);

    match state
        .auth
        .supabase_exchange(
            device_id,
            device_role,
            device_name.as_deref(),
            &req.access_token,
            &client_ip(&headers),
        )
        .await
    {
        Ok(session) => auth_session_response(session),
        Err(error) => auth_error_response(error),
    }
}

pub fn router() -> Router<BackendState> {
    // Routes are mounted under `/v1` by `crate::http::v1::router`, so the
    // path prefixes here are relative to `/v1`.
    Router::new()
        .route("/auth/refresh", post(post_refresh))
        .route("/auth/logout", post(post_logout))
        .route("/auth/supabase", post(post_supabase_exchange))
}
