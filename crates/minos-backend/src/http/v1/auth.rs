//! `/v1/auth/{register,login,refresh,logout}` HTTP handlers (spec §5.2).
//!
//! All four endpoints share the same input/output JSON shapes and the
//! same dual-rail authentication: every request must carry the
//! `X-Device-Id` / `X-Device-Role` (+ `X-Device-Secret` once paired)
//! header bundle so the device-secret rail (`crate::http::auth`) can
//! resolve a `DeviceId` before the account-rail does its own work.
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
use serde::{Deserialize, Serialize};

use crate::auth::bearer;
use crate::auth::use_case::{AuthSession, AuthUseCaseError, RefreshSession};
use crate::http::auth::authenticate;
use crate::http::error_response::{err_json, ErrorEnvelope};
use crate::http::BackendState;

#[derive(Deserialize)]
pub struct RegisterReq {
    pub email: String,
    pub password: String,
}

// Hand-rolled `Debug` so a future maintainer adding `tracing::debug!(?req)`
// doesn't leak passwords into xlog. Email is fine to surface; the
// password field is replaced with the literal string `"<redacted>"`.
impl std::fmt::Debug for RegisterReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterReq")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
}

impl std::fmt::Debug for LoginReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginReq")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub struct RefreshReq {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutReq {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct AccountSummary {
    pub account_id: String,
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResp {
    pub account: AccountSummary,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Serialize)]
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
        AuthUseCaseError::AccountNotFound => {
            (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response()
        }
        AuthUseCaseError::EmailTaken => (StatusCode::CONFLICT, err("email_taken")).into_response(),
        AuthUseCaseError::InvalidCredentials => {
            (StatusCode::UNAUTHORIZED, err("invalid_credentials")).into_response()
        }
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
        AuthUseCaseError::WeakPassword => {
            (StatusCode::BAD_REQUEST, err("weak_password")).into_response()
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

#[tracing::instrument(skip_all, fields(email = %req.email))]
pub async fn post_register(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Response {
    let Ok(outcome) = authenticate(&state.store, &headers).await else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    match state
        .auth
        .register(
            outcome.device_id,
            &req.email,
            &req.password,
            &client_ip(&headers),
        )
        .await
    {
        Ok(session) => auth_session_response(session),
        Err(error) => auth_error_response(error),
    }
}

#[tracing::instrument(skip_all, fields(email = %req.email))]
pub async fn post_login(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Response {
    let Ok(outcome) = authenticate(&state.store, &headers).await else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    match state
        .auth
        .login(
            outcome.device_id,
            &req.email,
            &req.password,
            &client_ip(&headers),
        )
        .await
    {
        Ok(session) => auth_session_response(session),
        Err(error) => auth_error_response(error),
    }
}

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

#[tracing::instrument(skip_all)]
/// `POST /v1/auth/change-password`
///
/// Requires both the device rail (`X-Device-Id`/secret) and a valid bearer
/// token so only the logged-in owner of the account can rotate the password.
/// On success every active refresh token for the account is revoked,
/// forcing other devices to sign in again — matching the intuition that a
/// password change is a credential rotation, not a private-device-only
/// event.
#[tracing::instrument(skip_all)]
pub async fn post_change_password(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<minos_protocol::ChangePasswordRequest>,
) -> Response {
    if authenticate(&state.store, &headers).await.is_err() {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    }
    let Ok(bearer_outcome) = bearer::require(&state, &headers) else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    match state
        .auth
        .change_password(
            &bearer_outcome.account_id,
            &req.current_password,
            &req.new_password,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => auth_error_response(error),
    }
}

pub fn router() -> Router<BackendState> {
    // Routes are mounted under `/v1` by `crate::http::v1::router`, so the
    // path prefixes here are relative to `/v1`.
    Router::new()
        .route("/auth/register", post(post_register))
        .route("/auth/login", post(post_login))
        .route("/auth/refresh", post(post_refresh))
        .route("/auth/logout", post(post_logout))
        .route("/auth/change-password", post(post_change_password))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_req_debug_redacts_password() {
        let req = RegisterReq {
            email: "alice@example.com".into(),
            password: "supersecret".into(),
        };
        let s = format!("{req:?}");
        assert!(s.contains("alice@example.com"));
        assert!(s.contains("<redacted>"));
        assert!(!s.contains("supersecret"));
    }

    #[test]
    fn login_req_debug_redacts_password() {
        let req = LoginReq {
            email: "bob@example.com".into(),
            password: "anothersecret".into(),
        };
        let s = format!("{req:?}");
        assert!(s.contains("bob@example.com"));
        assert!(s.contains("<redacted>"));
        assert!(!s.contains("anothersecret"));
    }
}
