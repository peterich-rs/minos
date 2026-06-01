//! Formal host rail.
//!
//! Bootstrap endpoints use nonce-bound Ed25519 proof. Steady-state host
//! endpoints use the opaque host installation token issued by
//! `/v1/host/pairing/redeem`.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{extract::State, Json, Router};
use minos_protocol::{PairingQrPayload, RequestPairingQrResponse};
use serde::Deserialize;
use serde::Serialize;

use crate::auth::host_bootstrap::{self, HostBootstrapError, HostBootstrapProof};
use crate::auth::host_installation;
use crate::http::error_response::{err_json, ErrorEnvelope};
use crate::http::v1::contract::{request_id, ResponseEnvelope};
use crate::http::BackendState;

#[derive(Debug, Serialize)]
struct HostSelfData {
    host_installation_id: String,
    display_name: String,
    link_count: usize,
    links: Vec<HostSelfLinkSummary>,
}

#[derive(Debug, Serialize)]
struct HostSelfLinkSummary {
    linked_via_installation_id: String,
    link_display_name: String,
    paired_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct HostRealtimeTicketData {
    ticket: String,
    expires_at_ms: i64,
    gateway_url: String,
}

#[derive(Debug, Deserialize)]
struct BootstrapNonceRequest {
    installation_id: String,
}

#[derive(Debug, Serialize)]
struct BootstrapNonceData {
    nonce: String,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct HostPairingRequestCodeRequest {
    installation_id: String,
    nonce: String,
    public_key: Option<String>,
    signature: String,
    host_display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HostPairingRedeemRequest {
    installation_id: String,
    nonce: String,
    public_key: Option<String>,
    signature: String,
    pairing_code: String,
    client_request_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct HostPairingRedeemData {
    host_installation_id: String,
    host_installation_token: String,
    issued_at_ms: i64,
}

const REQUEST_CODE_PATH: &str = "/v1/host/pairing/request-code";
const REDEEM_PATH: &str = "/v1/host/pairing/redeem";

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/host/bootstrap/nonce", post(post_bootstrap_nonce))
        .route(
            "/host/pairing/request-code",
            post(post_pairing_request_code),
        )
        .route("/host/pairing/redeem", post(post_pairing_redeem))
        .route("/host/installations/self", post(post_installations_self))
        .route("/host/realtime/ws-ticket", post(post_realtime_ws_ticket))
}

async fn post_bootstrap_nonce(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<BootstrapNonceRequest>,
) -> Result<Json<ResponseEnvelope<BootstrapNonceData>>, (StatusCode, Json<ErrorEnvelope>)> {
    if uuid::Uuid::parse_str(&req.installation_id).is_err() {
        return Err((StatusCode::BAD_REQUEST, err("bad_request")));
    }
    let nonce = state
        .bootstrap_nonces
        .issue(&req.installation_id, chrono::Utc::now().timestamp_millis());

    Ok(Json(ResponseEnvelope::new(
        BootstrapNonceData {
            nonce: nonce.nonce,
            expires_at_ms: nonce.expires_at_ms,
        },
        request_id(&headers),
    )))
}

async fn post_pairing_request_code(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<HostPairingRequestCodeRequest>,
) -> Result<Json<ResponseEnvelope<RequestPairingQrResponse>>, (StatusCode, Json<ErrorEnvelope>)> {
    let display_name = req
        .host_display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("host");
    let installation_id = host_bootstrap::verify_and_register(
        &state.store,
        state.bootstrap_nonces.as_ref(),
        HostBootstrapProof {
            installation_id: &req.installation_id,
            nonce: &req.nonce,
            public_key: req.public_key.as_deref(),
            signature: &req.signature,
        },
        REQUEST_CODE_PATH,
        display_name,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(host_bootstrap_error)?;

    let (pairing_code, expires) = state
        .pairing
        .request_code(installation_id, state.token_ttl)
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "minos_backend::v1::host",
                error = %error,
                host_installation_id = %installation_id,
                "formal host pairing request-code failed",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
        })?;

    Ok(Json(ResponseEnvelope::new(
        RequestPairingQrResponse {
            qr_payload: PairingQrPayload {
                v: 2,
                host_display_name: display_name.to_string(),
                pairing_token: pairing_code,
                expires_at_ms: expires.timestamp_millis(),
            },
        },
        request_id(&headers),
    )))
}

async fn post_pairing_redeem(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<HostPairingRedeemRequest>,
) -> Result<Json<ResponseEnvelope<HostPairingRedeemData>>, (StatusCode, Json<ErrorEnvelope>)> {
    let installation_id = host_bootstrap::verify_and_register(
        &state.store,
        state.bootstrap_nonces.as_ref(),
        HostBootstrapProof {
            installation_id: &req.installation_id,
            nonce: &req.nonce,
            public_key: req.public_key.as_deref(),
            signature: &req.signature,
        },
        REDEEM_PATH,
        "host",
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(host_bootstrap_error)?;

    let redeemed = state
        .pairing
        .redeem_host_installation(
            &req.pairing_code,
            installation_id,
            req.client_request_id.as_deref(),
        )
        .await
        .map_err(formal_pairing_error)?;

    Ok(Json(ResponseEnvelope::new(
        HostPairingRedeemData {
            host_installation_id: redeemed.host_installation_id.to_string(),
            host_installation_token: redeemed.token,
            issued_at_ms: redeemed.issued_at_ms,
        },
        request_id(&headers),
    )))
}

async fn post_installations_self(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ResponseEnvelope<HostSelfData>>, (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = require_host(&state, &headers).await?;
    let host = crate::store::devices::get_device(&state.store, outcome.host_installation_id)
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "minos_backend::v1::host",
                error = %error,
                host_installation_id = %outcome.host_installation_id,
                "get_device(host) failed",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
        })?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, err("unauthorized")))?;

    let pairs = crate::store::account_host_pairings::list_accounts_for_host(
        &state.store,
        outcome.host_installation_id,
    )
    .await
    .map_err(|error| {
        tracing::warn!(
            target: "minos_backend::v1::host",
            error = %error,
            host_installation_id = %outcome.host_installation_id,
            "list_accounts_for_host failed",
        );
        (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
    })?;

    let mut links = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let display_name =
            crate::store::devices::get_device(&state.store, pair.paired_via_device_id)
                .await
                .map_err(|error| {
                    tracing::warn!(
                        target: "minos_backend::v1::host",
                        error = %error,
                        linked_via_installation_id = %pair.paired_via_device_id,
                        "get_device(linked_via) failed",
                    );
                    (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
                })?
                .map_or_else(|| "unknown".to_string(), |row| row.display_name);

        links.push(HostSelfLinkSummary {
            linked_via_installation_id: pair.paired_via_device_id.to_string(),
            link_display_name: display_name,
            paired_at_ms: pair.paired_at_ms,
        });
    }

    Ok(Json(ResponseEnvelope::new(
        HostSelfData {
            host_installation_id: outcome.host_installation_id.to_string(),
            display_name: host.display_name,
            link_count: links.len(),
            links,
        },
        request_id(&headers),
    )))
}

async fn post_realtime_ws_ticket(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Response {
    let outcome = match require_host(&state, &headers).await {
        Ok(outcome) => outcome,
        Err((status, body)) => return (status, body).into_response(),
    };
    let host_installation_id = outcome.host_installation_id.to_string();
    let session = match state
        .auth
        .issue_host_ws_ticket(outcome.host_installation_id)
        .await
    {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::v1::host",
                error = ?error,
                host_installation_id,
                "issue host ws ticket failed",
            );
            return (StatusCode::INTERNAL_SERVER_ERROR, err("internal")).into_response();
        }
    };

    Json(ResponseEnvelope::new(
        HostRealtimeTicketData {
            gateway_url: format!("/ws/host?ticket={}", session.ticket),
            ticket: session.ticket,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + (session.expires_in * 1000),
        },
        request_id(&headers),
    ))
    .into_response()
}

async fn require_host(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<host_installation::HostInstallationPrincipal, (StatusCode, Json<ErrorEnvelope>)> {
    host_installation::require(state, headers)
        .await
        .map_err(|error| {
            let (status, message) = error.into_response_tuple();
            let code = if status == StatusCode::UNAUTHORIZED {
                "unauthorized"
            } else {
                "internal"
            };
            (status, err_with_message(code, message))
        })
}

fn err(code: &'static str) -> Json<ErrorEnvelope> {
    err_json(code, code)
}

fn err_with_message(code: &'static str, message: String) -> Json<ErrorEnvelope> {
    err_json(code, message)
}

fn host_bootstrap_error(error: HostBootstrapError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        HostBootstrapError::NonceInvalid => (
            StatusCode::UNAUTHORIZED,
            err("host_bootstrap_nonce_invalid"),
        ),
        HostBootstrapError::ProofInvalid | HostBootstrapError::PublicKeyMismatch => (
            StatusCode::UNAUTHORIZED,
            err("host_bootstrap_proof_invalid"),
        ),
        HostBootstrapError::Store(error) => {
            tracing::warn!(
                target: "minos_backend::v1::host",
                error = %error,
                "host bootstrap store failure",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
        }
    }
}

fn formal_pairing_error(
    error: crate::pairing::FormalPairingError,
) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        crate::pairing::FormalPairingError::PairingCodeInvalid => {
            (StatusCode::CONFLICT, err("pairing_code_invalid"))
        }
        crate::pairing::FormalPairingError::PairingStateMismatch { actual } => (
            StatusCode::CONFLICT,
            err_with_message("pairing_state_mismatch", actual),
        ),
        crate::pairing::FormalPairingError::Internal(error) => {
            tracing::warn!(
                target: "minos_backend::v1::host",
                error = %error,
                "formal host pairing operation failed",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
        }
    }
}
