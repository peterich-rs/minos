//! `GET /v1/me/peer` — return the caller's currently paired peer.
//!
//! Authenticated via the same `X-Device-Id` + `X-Device-Secret` rail as
//! `DELETE /v1/pairing`. Used by the macOS daemon after each successful
//! WS connect to refresh its in-memory peer mirror without persisting
//! anything to disk (Phase D of the persistence-cleanup plan).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use minos_protocol::MePeerResponse;

use crate::http::auth;
use crate::http::v1::pairing::{err_body, ErrorEnvelope};
use crate::http::BackendState;
use crate::store::{devices, pairings};

pub fn router() -> Router<BackendState> {
    Router::new().route("/me/peer", get(get_me_peer))
}

async fn get_me_peer(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<MePeerResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = auth::authenticate(&state.store, &headers)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => {
                (StatusCode::UNAUTHORIZED, err_body("unauthorized", m))
            }
            auth::AuthError::Internal(m) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", m))
            }
        })?;
    if !outcome.authenticated_with_secret {
        return Err((
            StatusCode::UNAUTHORIZED,
            err_body("unauthorized", "X-Device-Secret required for /v1/me/peer"),
        ));
    }

    let pair = pairings::get_pair_with_created_at(&state.store, outcome.device_id)
        .await
        .map_err(|e| {
            tracing::warn!(
                target: "minos_backend::v1::me",
                error = %e,
                device_id = %outcome.device_id,
                "get_pair_with_created_at failed",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            )
        })?;

    let Some((peer_device_id, paired_at_ms)) = pair else {
        return Err((
            StatusCode::NOT_FOUND,
            err_body("not_paired", "device has no peer"),
        ));
    };

    // Pull the peer's display name from the devices table. If the peer row
    // is missing — should not happen given the pairings FK — fall back to a
    // stable placeholder so we never 500 on a recoverable shape.
    let peer_name = match devices::get_device(&state.store, peer_device_id).await {
        Ok(Some(row)) => row.display_name,
        Ok(None) => {
            tracing::warn!(
                target: "minos_backend::v1::me",
                device_id = %outcome.device_id,
                peer = %peer_device_id,
                "pairing row references a device with no devices row",
            );
            "unknown".to_string()
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::v1::me",
                error = %e,
                device_id = %outcome.device_id,
                peer = %peer_device_id,
                "get_device(peer) failed",
            );
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            ));
        }
    };

    Ok(Json(MePeerResponse {
        peer_device_id,
        peer_name,
        paired_at_ms,
    }))
}
