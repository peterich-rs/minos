//! `GET /health` — liveness probe with DB connectivity check.
//!
//! Returns 200 when the backend is healthy (DB reachable), or 503 when
//! degraded (DB unreachable). Spec R7.6.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Serialize;

use super::BackendState;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: String,
    instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

/// Return `200 OK` with JSON body when healthy, `503 Service Unavailable`
/// when the DB is unreachable.
pub async fn get(State(state): State<BackendState>) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1").execute(&state.store).await.is_ok();

    if db_ok {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(HealthResponse {
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
            Json(HealthResponse {
                status: "degraded",
                version: format!("minos-backend v{}", state.version),
                instance_id: state.instance_id.clone(),
                reason: Some("db"),
            }),
        )
    }
}
