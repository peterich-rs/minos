//! `GET /health` — plaintext liveness probe.
//!
//! The body contains both the crate name (`minos-backend`) and its version so
//! a deploy smoke can assert each independently. See plan §14's "manual
//! smoke" acceptance for the exact contract.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
};

use super::BackendState;

/// Return `200 OK` with body `"minos-backend v<VERSION>\n"`.
///
/// `Content-Type` is set to `text/plain; charset=utf-8` so `curl -i` renders
/// the body cleanly in the smoke.
#[allow(clippy::unused_async)] // axum handlers must be `async`
pub async fn get(State(state): State<BackendState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("minos-backend v{}\n", state.version),
    )
}
