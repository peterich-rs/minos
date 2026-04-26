//! `GET /v1/threads*` handlers.

use axum::Router;

use super::super::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
}
