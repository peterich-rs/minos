//! Versioned `/v1` HTTP routes.
//!
//! Public `/v1` only exposes the formal account and host rails plus the
//! retained pairing teardown endpoint. Legacy caller-scoped `/v1/me/*`,
//! `/v1/threads/*`, `/v1/pairing/tokens`, and `/v1/pairing/consume` routes are
//! retired.

use axum::http::{HeaderMap, StatusCode};
use axum::{Json, Router};
use minos_domain::DeviceId;
use uuid::Uuid;

use super::BackendState;
use crate::auth::bearer;
use crate::http::error_response::{err_json as err, ErrorEnvelope};

pub mod agent_sessions;
pub mod approvals;
pub mod auth;
pub mod contract;
pub mod conversations;
pub mod friends;
pub mod host;
pub mod host_commands;
pub mod notifications;
pub mod pairing;
pub mod profiles;
pub mod projects;
pub mod realtime;
pub mod social;

pub fn router() -> Router<BackendState> {
    router_with_social(social::router())
}

pub fn external_sql_router() -> Router<BackendState> {
    router_with_social(social::external_sql_router())
}

fn router_with_social(social_router: Router<BackendState>) -> Router<BackendState> {
    Router::new()
        .merge(approvals::router())
        .merge(agent_sessions::router())
        .merge(auth::router())
        .merge(conversations::router())
        .merge(friends::router())
        .merge(host::router())
        .merge(host_commands::router())
        .merge(notifications::router())
        .merge(pairing::router())
        .merge(profiles::router())
        .merge(projects::router())
        .merge(realtime::router())
        .merge(social_router)
}

/// Build a rate-limited auth router for sensitive endpoints.
///
/// Wraps the auth router with per-IP rate limiting on register and login.
pub fn rate_limited_auth_router() -> Router<BackendState> {
    let limiter = crate::http::rate_limit::RateLimiter::from_env();
    auth::router().route_layer(axum::middleware::from_fn_with_state(
        limiter,
        crate::http::rate_limit::rate_limit_middleware,
    ))
}

pub(crate) async fn require_authed_session(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<(minos_domain::DeviceId, String), (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require_account(state, headers).map_err(|error| {
        let (status, message) = error.into_response_tuple();
        (status, err("unauthorized", message))
    })?;
    let device_id = Uuid::parse_str(&bearer_outcome.device_id)
        .map(DeviceId)
        .map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                err("unauthorized", "invalid bearer device id"),
            )
        })?;
    Ok((device_id, bearer_outcome.account_id))
}
