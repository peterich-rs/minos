//! Formal account realtime ticket endpoint.
//!
//! `POST /v1/realtime/ws-ticket` is the forward path for account clients.
//! It uses only the bearer token as business identity, then validates the
//! requested installation against the account before issuing the short-lived
//! gateway ticket for `/ws/client`.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{extract::State, Json, Router};
use minos_domain::DeviceId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{bearer, jwt};
use crate::http::error_response::{err_json, ErrorEnvelope};
use crate::http::v1::contract::{request_id, ResponseEnvelope};
use crate::http::BackendState;

#[derive(Debug, Deserialize)]
pub struct RealtimeWsTicketRequest {
    pub installation_id: String,
}

#[derive(Debug, Serialize)]
pub struct RealtimeWsTicketData {
    pub ticket: String,
    pub expires_at_ms: i64,
    pub gateway_url: String,
}

pub fn router() -> Router<BackendState> {
    Router::new().route("/realtime/ws-ticket", post(post_ws_ticket))
}

async fn post_ws_ticket(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RealtimeWsTicketRequest>,
) -> Response {
    let Ok(bearer_outcome) = bearer::require_account(&state, &headers) else {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    };
    let installation_id = match Uuid::parse_str(&req.installation_id).map(DeviceId) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, err("bad_request")).into_response(),
    };

    let row = match crate::store::devices::get_device(&state.store, installation_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response(),
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::v1::realtime",
                error = %error,
                installation_id = %req.installation_id,
                "get_device failed while issuing formal ws ticket",
            );
            return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
        }
    };

    if row.account_id.as_deref() != Some(&bearer_outcome.account_id)
        || !row.role.is_account_client()
    {
        return (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response();
    }

    match state
        .auth
        .issue_ws_ticket(&bearer_outcome.account_id, installation_id, row.role)
        .await
    {
        Ok(session) => {
            let expires_at_ms =
                chrono::Utc::now().timestamp_millis() + (jwt::WS_TICKET_TTL_SECS * 1000);
            Json(ResponseEnvelope::new(
                RealtimeWsTicketData {
                    gateway_url: format!("/ws/client?ticket={}", session.ticket),
                    ticket: session.ticket,
                    expires_at_ms,
                },
                request_id(&headers),
            ))
            .into_response()
        }
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::v1::realtime",
                error = ?error,
                installation_id = %req.installation_id,
                account_id = %bearer_outcome.account_id,
                "issue_ws_ticket failed",
            );
            (StatusCode::UNAUTHORIZED, err("unauthorized")).into_response()
        }
    }
}

fn err(code: &'static str) -> Json<ErrorEnvelope> {
    err_json(code, code)
}
