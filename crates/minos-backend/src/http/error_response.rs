//! Shared HTTP error response types used across all `/v1/*` handlers.
//!
//! Centralises the `ErrorEnvelope` / `ErrorBody` pattern so each handler
//! module doesn't re-define its own copy.

use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

/// Standard JSON error envelope: `{ "error": { "code": "...", "message": "..." } }`.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
}

/// Construct an `ErrorEnvelope` wrapped in `Json`.
pub fn err_json(code: &'static str, message: impl Into<String>) -> Json<ErrorEnvelope> {
    Json(ErrorEnvelope {
        error: ErrorBody {
            code,
            message: message.into(),
        },
    })
}

/// Construct a `(StatusCode, Json<ErrorEnvelope>)` tuple using the
/// conventional code-to-status mapping.
pub fn err_response(
    code: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorEnvelope>) {
    (status_for_code(code), err_json(code, message))
}

/// Map well-known error codes to HTTP status codes.
pub fn status_for_code(code: &str) -> StatusCode {
    match code {
        "unauthorized" => StatusCode::UNAUTHORIZED,
        "not_found" | "thread_not_found" => StatusCode::NOT_FOUND,
        "forbidden" => StatusCode::FORBIDDEN,
        "conflict" => StatusCode::CONFLICT,
        "bad_request" | "weak_password" => StatusCode::BAD_REQUEST,
        "rate_limited" => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_for_code_maps_forbidden_to_403() {
        assert_eq!(status_for_code("forbidden"), StatusCode::FORBIDDEN);
    }
}
