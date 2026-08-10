//! Health endpoints.
//!
//! Three-way split:
//!
//! - `GET /health/live`  — process is up. Always 200 unless the runtime
//!   itself is toast. Used by orchestrators (launchd / k8s) for
//!   liveness probes.
//! - `GET /health/ready` — process is ready to serve traffic. 200 when
//!   the configured SQL store is reachable; 503 otherwise. Used by load
//!   balancers — flipping this to 503 during graceful shutdown lets
//!   upstream LBs drain traffic before the listener stops.
//! - `GET /health/info`  — non-cached structured info (version,
//!   instance id, build commit, environment). Useful for "what is
//!   actually running" debugging.
//!
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Serialize;

use super::BackendState;

#[derive(Serialize)]
struct LiveResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct ReadyResponse {
    status: &'static str,
    version: String,
    instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

#[derive(Serialize)]
struct InfoResponse {
    name: &'static str,
    version: String,
    instance_id: String,
    build_profile: &'static str,
    /// Stable label for the deployment environment. Sourced from
    /// `MINOS_ENV` at boot when available, otherwise `"dev"`.
    env: String,
}

/// `GET /health/live` — process liveness.
///
/// Always 200 unless the process is unable to handle a request (in
/// which case axum will not call this anyway). The body carries the
/// minimum: orchestrators only need a 2xx.
#[utoipa::path(
    get,
    path = "/health/live",
    responses(
        (status = 200, description = "Process is alive")
    ),
    tag = "health"
)]
pub async fn live() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(LiveResponse { status: "ok" }),
    )
}

/// `GET /health/ready` — readiness probe; returns 503 when DB pings fail.
#[utoipa::path(
    get,
    path = "/health/ready",
    responses(
        (status = 200, description = "Ready to serve traffic"),
        (status = 503, description = "Not ready (dependency unreachable)")
    ),
    tag = "health"
)]
pub async fn ready(State(state): State<BackendState>) -> impl IntoResponse {
    let db_ok = state.store.ping().await.is_ok();

    if db_ok {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(ReadyResponse {
                status: "ok",
                version: format!("minos-backend v{}", state.version),
                instance_id: state.instance_id.clone(),
                reason: None,
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            Json(ReadyResponse {
                status: "degraded",
                version: format!("minos-backend v{}", state.version),
                instance_id: state.instance_id.clone(),
                reason: Some("db"),
            }),
        )
    }
}

/// `GET /health/info` — non-cached process metadata. Useful for
/// "what version is actually running here" sanity in incident response.
#[utoipa::path(
    get,
    path = "/health/info",
    responses(
        (status = 200, description = "Process metadata")
    ),
    tag = "health"
)]
pub async fn info(State(state): State<BackendState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(InfoResponse {
            name: "minos-backend",
            version: state.version.to_string(),
            instance_id: state.instance_id.clone(),
            build_profile: build_profile(),
            env: state.config.environment.as_str().to_string(),
        }),
    )
}

const fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// `GET /health/jobs` — worker job status.
///
/// Returns the status of background workers (token GC, realtime listener).
/// Useful for verifying that supervised workers are running in monolith mode.
#[utoipa::path(
    get,
    path = "/health/jobs",
    responses(
        (status = 200, description = "Worker job status")
    ),
    tag = "health"
)]
pub async fn jobs(State(state): State<BackendState>) -> impl IntoResponse {
    #[derive(Serialize)]
    struct JobsResponse {
        session_registry_size: usize,
        runtime_mode: &'static str,
        storage_mode: &'static str,
    }

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(JobsResponse {
            session_registry_size: state.registry.len(),
            runtime_mode: state.config.runtime_mode.as_str(),
            storage_mode: state.config.storage_mode.as_str(),
        }),
    )
}
