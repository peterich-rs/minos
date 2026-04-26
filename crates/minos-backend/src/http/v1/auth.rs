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

use std::sync::OnceLock;

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::rate_limit::RateLimiter;
use crate::auth::{bearer, jwt, passwords};
use crate::error::BackendError;
use crate::http::auth::authenticate;
use crate::http::BackendState;
use crate::store::{accounts, refresh_tokens};

/// Lazily-initialised argon2id PHC string used to burn the same compute
/// time on `find_by_email → None` as on `find_by_email → Some` followed
/// by a wrong-password verify. Without this, "unknown email" returned in
/// <1 ms while "valid email + wrong password" took ~50 ms — a timing
/// oracle for email enumeration. The plaintext is irrelevant; we only
/// rely on `passwords::verify` doing the same kdf work either way.
fn dummy_password_hash() -> &'static str {
    static DUMMY_HASH: OnceLock<String> = OnceLock::new();
    DUMMY_HASH.get_or_init(|| {
        passwords::hash("dummy_for_constant_time_check_xxxxxxx")
            .expect("argon2id default params must hash a static string")
    })
}

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

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub kind: &'static str,
}

const fn err(kind: &'static str) -> Json<ErrorBody> {
    Json(ErrorBody { kind })
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

/// Apply a single bucket and return a 429 response on overflow.
///
/// Returns the response in `Some(_)` on overflow so the caller can
/// `if let Some(resp) = check_bucket(..) { return resp; }`. The clippy
/// `result_large_err` lint pushed us off `Result<(), Response>` because
/// `Response` is ≈128 bytes and the success path doesn't allocate.
#[must_use]
fn check_bucket(limiter: &RateLimiter, key: &str) -> Option<Response> {
    limiter.check(key).err().map(rate_limited_response)
}

pub async fn post_register(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Response {
    if let Some(resp) = check_bucket(&state.auth_register_per_ip, &client_ip(&headers)) {
        return resp;
    }
    if req.password.len() < 8 {
        return (StatusCode::BAD_REQUEST, err("weak_password")).into_response();
    }
    let Ok(outcome) = authenticate(&state.store, &headers).await else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    let device_id = outcome.device_id;

    let Ok(hash) = passwords::hash(&req.password) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    };
    let account = match accounts::create(&state.store, &req.email, &hash).await {
        Ok(a) => a,
        Err(BackendError::EmailTaken) => {
            return (StatusCode::CONFLICT, err("email_taken")).into_response()
        }
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response(),
    };

    if crate::store::devices::set_account_id(&state.store, &device_id, &account.account_id)
        .await
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }

    let Ok(access) = jwt::sign(
        state.jwt_secret.as_bytes(),
        &account.account_id,
        &device_id.to_string(),
    ) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    };
    let refresh_plain = refresh_tokens::generate_plaintext();
    if refresh_tokens::insert(
        &state.store,
        &refresh_plain,
        &account.account_id,
        &device_id.to_string(),
    )
    .await
    .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }

    (
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
    )
        .into_response()
}

pub async fn post_login(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Response {
    // Bucket ordering: IP-keyed buckets fire pre-`authenticate` because
    // the IP is the abuse axis we want to throttle even on bogus requests.
    // Identity-keyed buckets (per-email here) fire post-`authenticate` so
    // an attacker spamming `email=victim@x.com` with garbage device
    // headers cannot lock the victim's bucket.
    if let Some(resp) = check_bucket(&state.auth_login_per_ip, &client_ip(&headers)) {
        return resp;
    }
    let Ok(outcome) = authenticate(&state.store, &headers).await else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    let device_id = outcome.device_id;
    if let Some(resp) = check_bucket(&state.auth_login_per_email, &req.email.to_lowercase()) {
        return resp;
    }

    let account = match accounts::find_by_email(&state.store, &req.email).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            // Burn the same argon2id verify time as the wrong-password
            // path so an attacker can't tell unknown emails apart from
            // valid-email-wrong-password by timing alone. Result is
            // ignored; we always return `invalid_credentials`.
            let _ = passwords::verify(&req.password, dummy_password_hash());
            return (StatusCode::UNAUTHORIZED, err("invalid_credentials")).into_response();
        }
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response(),
    };
    let Ok(ok) = passwords::verify(&req.password, &account.password_hash) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    };
    if !ok {
        return (StatusCode::UNAUTHORIZED, err("invalid_credentials")).into_response();
    }

    // Single-active-iPhone (spec §5.5):
    // 1. Revoke every prior refresh token for this account so any
    //    displaced device cannot rotate its way back to a valid access
    //    token.
    // 2. Forcibly close every live iOS WS session bound to this account
    //    EXCEPT the device that just logged in. The displaced device's
    //    socket loop will see its `revoked` watch flip and tear down.
    let _ = refresh_tokens::revoke_all_for_account(&state.store, &account.account_id).await;
    let closed = state
        .registry
        .close_account_sessions(&account.account_id, Some(&device_id.to_string()));
    if closed > 0 {
        tracing::info!(
            target: "minos_backend::v1::auth",
            account_id = %account.account_id,
            closed,
            "login displaced existing iOS sessions for this account"
        );
    }

    if accounts::touch_last_login(&state.store, &account.account_id)
        .await
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }
    if crate::store::devices::set_account_id(&state.store, &device_id, &account.account_id)
        .await
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }

    let Ok(access) = jwt::sign(
        state.jwt_secret.as_bytes(),
        &account.account_id,
        &device_id.to_string(),
    ) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    };
    let refresh_plain = refresh_tokens::generate_plaintext();
    if refresh_tokens::insert(
        &state.store,
        &refresh_plain,
        &account.account_id,
        &device_id.to_string(),
    )
    .await
    .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }

    Json(AuthResp {
        account: AccountSummary {
            account_id: account.account_id,
            email: account.email,
        },
        access_token: access,
        refresh_token: refresh_plain,
        expires_in: jwt::ACCESS_TTL_SECS,
    })
    .into_response()
}

pub async fn post_refresh(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RefreshReq>,
) -> Response {
    let Ok(outcome) = authenticate(&state.store, &headers).await else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    let device_id = outcome.device_id;

    let row = match refresh_tokens::find_active(&state.store, &req.refresh_token).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::UNAUTHORIZED, err("invalid_refresh")).into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response(),
    };

    if row.device_id != device_id.to_string() {
        return (StatusCode::UNAUTHORIZED, err("invalid_refresh")).into_response();
    }

    // Per-account bucket sits between the lookup and the rotation: we
    // already know the account binding once the row is loaded, so
    // limiting on `row.account_id` is more accurate than IP-keying for
    // refresh.
    if let Some(resp) = check_bucket(&state.auth_refresh_per_acc, &row.account_id) {
        return resp;
    }

    if refresh_tokens::revoke_one(&state.store, &req.refresh_token)
        .await
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }
    let new_plain = refresh_tokens::generate_plaintext();
    if refresh_tokens::insert(&state.store, &new_plain, &row.account_id, &row.device_id)
        .await
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }
    let Ok(access) = jwt::sign(state.jwt_secret.as_bytes(), &row.account_id, &row.device_id) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    };

    Json(RefreshResp {
        access_token: access,
        refresh_token: new_plain,
        expires_in: jwt::ACCESS_TTL_SECS,
    })
    .into_response()
}

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
    // Per-account bucket sits between bearer-success and `revoke_one` so a
    // compromised access token can't spam logout to revoke arbitrary
    // candidate refresh tokens. Reuses the refresh bucket — both write
    // refresh_tokens, both should share the same per-account budget.
    if let Some(resp) = check_bucket(&state.auth_refresh_per_acc, &bearer_outcome.account_id) {
        return resp;
    }
    if refresh_tokens::revoke_one(&state.store, &req.refresh_token)
        .await
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
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
