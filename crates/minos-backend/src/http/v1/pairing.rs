//! `POST /v1/pairing/*` and `DELETE /v1/pairing` handlers.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, post};
use axum::{Json, Router};
use minos_domain::DeviceRole;
use minos_protocol::{
    Envelope, EventKind, PairConsumeRequest, PairResponse, PairingQrPayload,
    RequestPairingQrParams, RequestPairingQrResponse,
};
use serde::Serialize;

use crate::error::BackendError;
use crate::http::auth;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/pairing/tokens", post(post_tokens))
        .route("/pairing/consume", post(post_consume))
        .route("/pairing", delete(delete_pairing))
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

async fn post_consume(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<PairConsumeRequest>,
) -> Result<Json<PairResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = auth::authenticate_role(&state.store, &headers, DeviceRole::IosClient)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => {
                (StatusCode::UNAUTHORIZED, err_body("unauthorized", m))
            }
            auth::AuthError::Internal(m) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", m))
            }
        })?;
    let consumer_id = outcome.device_id;

    let pairing_outcome = match state
        .pairing
        .consume_token(&params.token, consumer_id, params.device_name.clone())
        .await
    {
        Ok(o) => o,
        Err(BackendError::PairingTokenInvalid) => {
            return Err((
                StatusCode::CONFLICT,
                err_body(
                    "pairing_token_invalid",
                    "pairing token is unknown, expired, or already consumed",
                ),
            ));
        }
        Err(BackendError::PairingStateMismatch { actual }) => {
            let msg = if actual == "self" {
                "device cannot pair with itself".to_string()
            } else {
                format!("peer already paired (state: {actual})")
            };
            return Err((
                StatusCode::CONFLICT,
                err_body("pairing_state_mismatch", msg),
            ));
        }
        Err(e) => {
            tracing::warn!(target: "minos_backend::v1::pairing", error = %e, "consume_token failed");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            ));
        }
    };

    let issuer_id = pairing_outcome.issuer_device_id;
    let consumer_secret = pairing_outcome.consumer_secret.clone();

    let mac_name = match crate::store::devices::get_device(&state.store, issuer_id).await {
        Ok(Some(row)) => row.display_name,
        _ => "Mac".to_string(),
    };

    // Push Event::Paired to the issuer's live WS, if any. If issuer is offline
    // OR the queue rejects, compensate (clear the pairing) — same as the WS
    // `envelope::local_rpc::handle_pair` reference implementation.
    if let Some(issuer_handle) = state.registry.get(issuer_id) {
        let frame = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: consumer_id,
                peer_name: params.device_name.clone(),
                your_device_secret: pairing_outcome.issuer_secret.clone(),
            },
        };
        *issuer_handle.paired_with.write().await = Some(consumer_id);
        if let Err(e) = state.registry.try_send_current(&issuer_handle, frame) {
            tracing::warn!(
                target: "minos_backend::v1::pairing",
                error = ?e,
                issuer = %issuer_id,
                "Event::Paired delivery failed; compensating",
            );
            *issuer_handle.paired_with.write().await = None;
            let _ = state.pairing.forget_pair(consumer_id).await;
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body(
                    "internal",
                    "failed to deliver pairing secret to issuer; pairing rolled back",
                ),
            ));
        }
    } else {
        tracing::warn!(
            target: "minos_backend::v1::pairing",
            issuer = %issuer_id,
            consumer = %consumer_id,
            "issuer offline at pair time; compensating",
        );
        let _ = state.pairing.forget_pair(consumer_id).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", "issuer is offline; pairing rolled back"),
        ));
    }

    Ok(Json(PairResponse {
        peer_device_id: issuer_id,
        peer_name: mac_name,
        your_device_secret: consumer_secret,
    }))
}

async fn delete_pairing(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
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
            err_body("unauthorized", "X-Device-Secret required for forget"),
        ));
    }

    let peer = match state.pairing.forget_pair(outcome.device_id).await {
        Ok(Some(peer)) => peer,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                err_body(
                    "pairing_state_mismatch",
                    "session is not paired; nothing to forget",
                ),
            ));
        }
        Err(e) => {
            tracing::warn!(target: "minos_backend::v1::pairing", error = %e, "forget_pair failed");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            ));
        }
    };

    let unpaired = Envelope::Event {
        version: 1,
        event: EventKind::Unpaired,
    };

    // Caller's own live session (if any).
    if let Some(self_handle) = state.registry.get(outcome.device_id) {
        *self_handle.paired_with.write().await = None;
        let _ = state
            .registry
            .try_send_current(&self_handle, unpaired.clone());
    }
    // Peer's live session (if any). ORDER MATTERS: clear paired_with BEFORE
    // pushing Event::Unpaired so the peer dispatcher cannot route one last
    // Forward off stale state.
    if let Some(peer_handle) = state.registry.get(peer) {
        *peer_handle.paired_with.write().await = None;
        let _ = state.registry.try_send_current(&peer_handle, unpaired);
    }

    Ok(StatusCode::NO_CONTENT)
}
