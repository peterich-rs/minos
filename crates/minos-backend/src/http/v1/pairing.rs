//! `POST /v1/pairing/*` and `DELETE /v1/pairing` handlers.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use minos_domain::DeviceRole;
use minos_protocol::{PairingQrPayload, RequestPairingQrParams, RequestPairingQrResponse};
use serde::Serialize;

use crate::http::auth;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new().route("/pairing/tokens", post(post_tokens))
    // pairing/consume + DELETE /pairing added in Tasks B1/B2
}

#[derive(Debug, Serialize)]
pub(crate) struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

pub(crate) fn err_body(code: &'static str, message: impl Into<String>) -> Json<ErrorEnvelope> {
    Json(ErrorEnvelope {
        error: ErrorBody {
            code,
            message: message.into(),
        },
    })
}

async fn post_tokens(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<RequestPairingQrParams>,
) -> Result<Json<RequestPairingQrResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = auth::authenticate_role(&state.store, &headers, DeviceRole::AgentHost)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => {
                (StatusCode::UNAUTHORIZED, err_body("unauthorized", m))
            }
            auth::AuthError::Internal(m) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", m))
            }
        })?;

    let (token, expires) = state
        .pairing
        .request_token(outcome.device_id, state.token_ttl)
        .await
        .map_err(|e| {
            tracing::warn!(target: "minos_backend::v1::pairing", error = %e, "request_token failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            )
        })?;

    let qr_payload = PairingQrPayload {
        v: 2,
        backend_url: state.public_cfg.public_url.clone(),
        host_display_name: params.host_display_name,
        pairing_token: token.as_str().to_string(),
        expires_at_ms: expires.timestamp_millis(),
        cf_access_client_id: state.public_cfg.cf_access_client_id.clone(),
        cf_access_client_secret: state.public_cfg.cf_access_client_secret.clone(),
    };
    Ok(Json(RequestPairingQrResponse { qr_payload }))
}
