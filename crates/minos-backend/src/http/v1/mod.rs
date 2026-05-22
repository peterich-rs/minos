//! Versioned `/v1` HTTP routes.
//!
//! Public `/v1` only exposes the formal account and host rails plus the
//! retained pairing teardown endpoint. The legacy caller-scoped `/v1/me/*`
//! surface and the legacy `/v1/pairing/tokens` and `/v1/pairing/consume`
//! routes are retired.

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
pub mod host;
pub mod pairing;
pub mod projects;
pub mod realtime;
pub mod social;
pub mod threads;

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
        .merge(host::router())
        .merge(pairing::router())
        .merge(projects::router())
        .merge(realtime::router())
        .merge(social_router)
        .merge(threads::router())
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
