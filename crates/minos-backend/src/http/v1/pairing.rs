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

use crate::auth::bearer;
use crate::error::BackendError;
use crate::http::auth;
use crate::http::BackendState;
use crate::session::SessionHandle;

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
        host_display_name: params.host_display_name,
        pairing_token: token.as_str().to_string(),
        expires_at_ms: expires.timestamp_millis(),
    };
    Ok(Json(RequestPairingQrResponse { qr_payload }))
}

// `post_consume` has grown beyond the default `clippy::too_many_lines`
// budget after Phase 2 Task 2.3 added bearer-required + dual-side
// account_id propagation. The flow is straight-line — split halves
// would just shuffle locals through arguments — so we allow it here
// rather than fragment the handler.
#[allow(clippy::too_many_lines)]
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

    // Phase 2 Task 2.3: pairing/consume must come from a logged-in iOS
    // session. The bearer's `account_id` becomes the account that owns
    // both ends of the pair — the iOS row gets it (in case the row carried
    // a stale value) and the issuing Mac inherits it post-consume so
    // Mac→iOS routing can later filter by `account_id`.
    let bearer_outcome = bearer::require(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;
    let account_id = bearer_outcome.account_id;

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

    // Phase 2 Task 2.3: copy the bearer's account onto BOTH device rows.
    // - iOS: re-write in case the prior account changed (login swap).
    // - Mac: inherit the iOS-side account so subsequent Mac→iOS routing
    //   can scope to one account (Task 2.4) and so login-time
    //   `close_account_sessions` can find the issuing pair (Task 2.5).
    if let Err(e) =
        crate::store::devices::set_account_id(&state.store, &consumer_id, &account_id).await
    {
        tracing::warn!(
            target: "minos_backend::v1::pairing",
            error = %e,
            consumer = %consumer_id,
            "set_account_id (consumer) failed",
        );
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", e.to_string()),
        ));
    }
    if let Err(e) =
        crate::store::devices::set_account_id(&state.store, &issuer_id, &account_id).await
    {
        tracing::warn!(
            target: "minos_backend::v1::pairing",
            error = %e,
            issuer = %issuer_id,
            "set_account_id (issuer) failed",
        );
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", e.to_string()),
        ));
    }

    let mac_name = match crate::store::devices::get_device(&state.store, issuer_id).await {
        Ok(Some(row)) => row.display_name,
        _ => "Mac".to_string(),
    };

    // Push Event::Paired to the issuer's live WS, if any. If issuer is offline
    // OR the queue rejects, compensate (clear the pairing) — guarantees the
    // §7.1 invariant that a Paired DB row implies the Mac saw Event::Paired.
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
        // Phase 2 Task 2.3: also seed the live Mac handle's `account_id`
        // so Mac→iOS routing (Task 2.4) does not have to wait for the Mac
        // to reconnect before routing scopes by account.
        issuer_handle.set_account_id(account_id.clone());
        if let Err(e) = state.registry.try_send_current(&issuer_handle, frame) {
            tracing::warn!(
                target: "minos_backend::v1::pairing",
                error = ?e,
                issuer = %issuer_id,
                "Event::Paired delivery failed; compensating",
            );
            let _ = state.pairing.forget_pair(consumer_id).await;
            refresh_live_pair_slot(&state, &issuer_handle).await;
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
        refresh_live_pair_slot(&state, &self_handle).await;
        let _ = state
            .registry
            .try_send_current(&self_handle, unpaired.clone());
    }
    // Peer's live session (if any). ORDER MATTERS: clear paired_with BEFORE
    // pushing Event::Unpaired so the peer dispatcher cannot route one last
    // Forward off stale state.
    if let Some(peer_handle) = state.registry.get(peer) {
        refresh_live_pair_slot(&state, &peer_handle).await;
        let _ = state.registry.try_send_current(&peer_handle, unpaired);
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn refresh_live_pair_slot(state: &BackendState, handle: &SessionHandle) {
    let next_peer = match crate::store::pairings::get_peers(&state.store, handle.device_id).await {
        Ok(peers) => peers.into_iter().next(),
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::v1::pairing",
                error = %e,
                device = %handle.device_id,
                "failed to refresh live paired_with slot"
            );
            None
        }
    };
    *handle.paired_with.write().await = next_peer;
}
