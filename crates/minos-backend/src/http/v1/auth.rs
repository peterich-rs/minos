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
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::{bearer, jwt, passwords};
use crate::error::BackendError;
use crate::http::auth::authenticate;
use crate::http::BackendState;
use crate::store::{accounts, refresh_tokens};

#[derive(Debug, Deserialize)]
pub struct RegisterReq {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
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

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub kind: &'static str,
}

const fn err(kind: &'static str) -> Json<ErrorBody> {
    Json(ErrorBody { kind })
}

pub async fn post_register(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Result<(StatusCode, Json<AuthResp>), (StatusCode, Json<ErrorBody>)> {
    if req.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, err("weak_password")));
    }
    let outcome = authenticate(&state.store, &headers)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, err("unauthorized")))?;
    let device_id = outcome.device_id;

    let hash = passwords::hash(&req.password)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    let account = accounts::create(&state.store, &req.email, &hash)
        .await
        .map_err(|e| match e {
            BackendError::EmailTaken => (StatusCode::CONFLICT, err("email_taken")),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, err("internal")),
        })?;

    crate::store::devices::set_account_id(&state.store, &device_id, &account.account_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;

    let access = jwt::sign(
        state.jwt_secret.as_bytes(),
        &account.account_id,
        &device_id.to_string(),
    )
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    let refresh_plain = refresh_tokens::generate_plaintext();
    refresh_tokens::insert(
        &state.store,
        &refresh_plain,
        &account.account_id,
        &device_id.to_string(),
    )
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;

    Ok((
        StatusCode::OK,
        Json(AuthResp {
            account: AccountSummary {
                account_id: account.account_id,
                email: account.email,
            },
            access_token: access,
            refresh_token: refresh_plain,
            expires_in: jwt::ACCESS_TTL_SECS,
        }),
    ))
}

pub async fn post_login(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Result<Json<AuthResp>, (StatusCode, Json<ErrorBody>)> {
    let outcome = authenticate(&state.store, &headers)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, err("unauthorized")))?;
    let device_id = outcome.device_id;

    let account = accounts::find_by_email(&state.store, &req.email)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?
        .ok_or((StatusCode::UNAUTHORIZED, err("invalid_credentials")))?;
    let ok = passwords::verify(&req.password, &account.password_hash)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    if !ok {
        return Err((StatusCode::UNAUTHORIZED, err("invalid_credentials")));
    }

    // Single-active-iPhone: revoke prior refresh tokens for this account.
    // Forcibly closing the WS sessions on other devices is wired in
    // Phase 2 Task 2.5 once SessionRegistry::close_account_sessions
    // exists; until then login can still succeed and the next refresh
    // attempt from the displaced device will fail with `invalid_refresh`.
    let _ = refresh_tokens::revoke_all_for_account(&state.store, &account.account_id).await;

    accounts::touch_last_login(&state.store, &account.account_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    crate::store::devices::set_account_id(&state.store, &device_id, &account.account_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;

    let access = jwt::sign(
        state.jwt_secret.as_bytes(),
        &account.account_id,
        &device_id.to_string(),
    )
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    let refresh_plain = refresh_tokens::generate_plaintext();
    refresh_tokens::insert(
        &state.store,
        &refresh_plain,
        &account.account_id,
        &device_id.to_string(),
    )
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;

    Ok(Json(AuthResp {
        account: AccountSummary {
            account_id: account.account_id,
            email: account.email,
        },
        access_token: access,
        refresh_token: refresh_plain,
        expires_in: jwt::ACCESS_TTL_SECS,
    }))
}

pub async fn post_refresh(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RefreshReq>,
) -> Result<Json<RefreshResp>, (StatusCode, Json<ErrorBody>)> {
    let outcome = authenticate(&state.store, &headers)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, err("unauthorized")))?;
    let device_id = outcome.device_id;

    let row = refresh_tokens::find_active(&state.store, &req.refresh_token)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?
        .ok_or((StatusCode::UNAUTHORIZED, err("invalid_refresh")))?;

    if row.device_id != device_id.to_string() {
        return Err((StatusCode::UNAUTHORIZED, err("invalid_refresh")));
    }

    refresh_tokens::revoke_one(&state.store, &req.refresh_token)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    let new_plain = refresh_tokens::generate_plaintext();
    refresh_tokens::insert(&state.store, &new_plain, &row.account_id, &row.device_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    let access = jwt::sign(state.jwt_secret.as_bytes(), &row.account_id, &row.device_id)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;

    Ok(Json(RefreshResp {
        access_token: access,
        refresh_token: new_plain,
        expires_in: jwt::ACCESS_TTL_SECS,
    }))
}

pub async fn post_logout(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LogoutReq>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let _ = authenticate(&state.store, &headers)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, err("unauthorized")))?;
    let _ = bearer::require(&state, &headers).map_err(|e| {
        let (status, _) = e.into_response_tuple();
        (status, err("unauthorized"))
    })?;
    refresh_tokens::revoke_one(&state.store, &req.refresh_token)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, err("internal")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router() -> Router<BackendState> {
    // Routes are mounted under `/v1` by `crate::http::v1::router`, so the
    // path prefixes here are relative to `/v1`.
    Router::new()
        .route("/auth/register", post(post_register))
        .route("/auth/login", post(post_login))
        .route("/auth/refresh", post(post_refresh))
        .route("/auth/logout", post(post_logout))
}
