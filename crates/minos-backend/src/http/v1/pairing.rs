//! `POST /v1/pairing/*` and `DELETE /v1/pairing` handlers.

use axum::Router;

use super::super::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
}
