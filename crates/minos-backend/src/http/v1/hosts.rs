//! Same-account host link rail (`POST /v1/hosts/link`, unlink, list).
//!
//! Replaces QR pairing as the primary account↔host binding path while QR
//! endpoints remain mounted until Phase D cleanup.

use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{extract::State, Json, Router};
use minos_domain::DeviceId;
use serde::{Deserialize, Serialize};

use crate::auth::bearer;
use crate::auth::host_bootstrap::{self, HostBootstrapError, HostBootstrapProof};
use crate::http::error_response::{err_json as err_body, ErrorEnvelope};
use crate::http::v1::contract::{request_id, ResponseEnvelope};
use crate::http::BackendState;
use crate::host_link::HostLinkError;

pub const LINK_PATH: &str = "v1/hosts/link";

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/hosts/link", post(post_link))
        .route("/hosts/unlink", post(post_unlink))
        .route("/hosts", get(get_hosts))
}

#[derive(Debug, Deserialize)]
struct LinkHostRequest {
    installation_id: String,
    nonce: String,
    public_key: Option<String>,
    signature: String,
    host_display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct LinkHostData {
    host_installation_id: String,
    host_installation_token: String,
    link: LinkSummary,
}

#[derive(Debug, Serialize)]
struct LinkSummary {
    pair_id: String,
    account_id: String,
    host_display_name: String,
    linked_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct UnlinkHostRequest {
    host_installation_id: String,
}

#[derive(Debug, Serialize)]
struct ListHostsData {
    hosts: Vec<HostSummary>,
}

#[derive(Debug, Serialize)]
struct HostSummary {
    host_installation_id: String,
    host_display_name: String,
    linked_at_ms: i64,
    online: bool,
}

async fn post_link(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LinkHostRequest>,
) -> Result<Json<ResponseEnvelope<LinkHostData>>, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require_account(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;

    let caller_installation_id = uuid::Uuid::parse_str(&bearer_outcome.device_id)
        .map(DeviceId)
        .map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                err_body("unauthorized", "invalid bearer device id"),
            )
        })?;
    ensure_account_client(&state, caller_installation_id, &bearer_outcome.account_id).await?;

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
        LINK_PATH,
        display_name,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(host_bootstrap_error)?;

    let outcome = state
        .host_link
        .link_host(
            installation_id,
            &bearer_outcome.account_id,
            caller_installation_id,
            Some(display_name),
        )
        .await
        .map_err(host_link_error)?;

    Ok(Json(ResponseEnvelope::new(
        LinkHostData {
            host_installation_id: outcome.host_installation_id.to_string(),
            host_installation_token: outcome.host_installation_token,
            link: LinkSummary {
                pair_id: outcome.link.pair_id,
                account_id: outcome.link.mobile_account_id,
                host_display_name: outcome
                    .link
                    .link_display_name
                    .unwrap_or_else(|| display_name.to_string()),
                linked_at_ms: outcome.link.paired_at_ms,
            },
        },
        request_id(&headers),
    )))
}

async fn post_unlink(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<UnlinkHostRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require_account(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;

    let host_installation_id = uuid::Uuid::parse_str(&req.host_installation_id)
        .map(DeviceId)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                err_body("bad_request", "invalid host_installation_id"),
            )
        })?;

    state
        .host_link
        .unlink_host(
            state.registry.as_ref(),
            host_installation_id,
            &bearer_outcome.account_id,
        )
        .await
        .map_err(host_link_error)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn get_hosts(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ResponseEnvelope<ListHostsData>>, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require_account(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;

    let pairs =
        crate::store::host_links::list_hosts_for_account(&state.store, &bearer_outcome.account_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "minos_backend::v1::hosts",
                    error = %e,
                    account_id = %bearer_outcome.account_id,
                    "list_hosts_for_account failed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err_body("internal", e.to_string()),
                )
            })?;

    let mut hosts = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let row = crate::store::device_installations::get_device(&state.store, pair.host_device_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "minos_backend::v1::hosts",
                    error = %e,
                    host_installation_id = %pair.host_device_id,
                    "get_device(host) failed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err_body("internal", e.to_string()),
                )
            })?;
        let host_display_name = pair
            .link_display_name
            .filter(|name| !name.trim().is_empty())
            .or_else(|| row.map(|r| r.display_name))
            .unwrap_or_else(|| "unknown".to_string());
        hosts.push(HostSummary {
            host_installation_id: pair.host_device_id.to_string(),
            host_display_name,
            linked_at_ms: pair.paired_at_ms,
            online: state.registry.get(pair.host_device_id).is_some(),
        });
    }

    Ok(Json(ResponseEnvelope::new(
        ListHostsData { hosts },
        request_id(&headers),
    )))
}

async fn ensure_account_client(
    state: &BackendState,
    installation_id: DeviceId,
    account_id: &str,
) -> Result<(), (StatusCode, Json<ErrorEnvelope>)> {
    let row = crate::store::device_installations::get_device(&state.store, installation_id)
        .await
        .map_err(|e| {
            tracing::warn!(
                target: "minos_backend::v1::hosts",
                error = %e,
                installation_id = %installation_id,
                "get_device(account caller) failed",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                err_body("unauthorized", "unknown account installation"),
            )
        })?;

    if !row.role.is_account_client() || row.account_id.as_deref() != Some(account_id) {
        return Err((
            StatusCode::UNAUTHORIZED,
            err_body("unauthorized", "account installation mismatch"),
        ));
    }

    Ok(())
}

fn host_bootstrap_error(error: HostBootstrapError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        HostBootstrapError::NonceInvalid => (
            StatusCode::UNAUTHORIZED,
            err_body("bootstrap_nonce_invalid", "bootstrap nonce invalid"),
        ),
        HostBootstrapError::ProofInvalid => (
            StatusCode::UNAUTHORIZED,
            err_body("proof_invalid", "host bootstrap proof invalid"),
        ),
        HostBootstrapError::PublicKeyMismatch => (
            StatusCode::UNAUTHORIZED,
            err_body("public_key_mismatch", "host public key mismatch"),
        ),
        HostBootstrapError::Store(error) => {
            tracing::warn!(
                target: "minos_backend::v1::hosts",
                error = %error,
                "host link bootstrap store failure",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", "internal"),
            )
        }
    }
}

fn host_link_error(error: HostLinkError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        HostLinkError::HostLinkedElsewhere { .. } => (
            StatusCode::CONFLICT,
            err_body(
                "host_linked_elsewhere",
                "host is already linked to another account",
            ),
        ),
        HostLinkError::NotFound => (
            StatusCode::NOT_FOUND,
            err_body("not_found", "host link does not exist"),
        ),
        HostLinkError::Internal(error) => {
            tracing::warn!(
                target: "minos_backend::v1::hosts",
                error = %error,
                "host link operation failed",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", error.to_string()),
            )
        }
    }
}
