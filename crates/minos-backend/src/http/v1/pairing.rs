//! `POST /v1/pairing/*` and `DELETE /v1/pairings/:host_device_id` handlers.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, post};
use axum::{Json, Router};
use minos_domain::DeviceId;
use serde::Deserialize;
use serde::Serialize;

use crate::auth::bearer;
use crate::http::error_response::{err_json as err_body, ErrorEnvelope};
use crate::http::v1::contract::{request_id, ResponseEnvelope};
use crate::http::BackendState;
use crate::pairing::FormalPairingError;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/pairing/confirm", post(post_confirm))
        .route("/pairing/revoke", post(post_revoke))
        .route("/pairing/list-hosts", post(post_list_hosts))
        .route("/pairings/:host_device_id", delete(delete_pair_for_host))
}

#[derive(Debug, Serialize)]
struct ListHostsData {
    hosts: Vec<FormalHostSummary>,
}

#[derive(Debug, Serialize)]
struct FormalHostSummary {
    host_installation_id: String,
    host_display_name: String,
    paired_at_ms: i64,
    linked_via_installation_id: String,
    online: bool,
}

#[derive(Debug, Deserialize)]
struct ConfirmPairingRequest {
    pairing_code: String,
    client_request_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConfirmPairingData {
    host_installation_id: String,
    status: &'static str,
    already_confirmed: bool,
}

#[derive(Debug, Deserialize)]
struct RevokePairingRequest {
    host_installation_id: String,
    client_request_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct RevokePairingData {
    host_installation_id: String,
    revoked: bool,
    remaining_link_count: i64,
    host_installation_token_revoked: bool,
}

/// Formal account rail pairing confirmation.
///
/// Consumes the host-issued pairing code with only the account bearer token,
/// creates the `(account_id, host_installation_id)` link idempotently, and
/// moves `pairing_codes.status` to `confirmed`.
async fn post_confirm(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<ConfirmPairingRequest>,
) -> Result<Json<ResponseEnvelope<ConfirmPairingData>>, (StatusCode, Json<ErrorEnvelope>)> {
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

    let outcome = state
        .pairing
        .confirm_pairing_code(
            &params.pairing_code,
            &bearer_outcome.account_id,
            caller_installation_id,
            params.client_request_id.as_deref(),
        )
        .await
        .map_err(formal_pairing_error)?;

    Ok(Json(ResponseEnvelope::new(
        ConfirmPairingData {
            host_installation_id: outcome.host_installation_id.to_string(),
            status: "confirmed",
            already_confirmed: outcome.already_confirmed,
        },
        request_id(&headers),
    )))
}

/// Formal account rail host-link revoke.
///
/// This removes only the caller account's link. If it was the last link for
/// the host installation, all active host installation tokens are revoked and
/// the live host session is closed through the registry revocation path.
async fn post_revoke(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<RevokePairingRequest>,
) -> Result<Json<ResponseEnvelope<RevokePairingData>>, (StatusCode, Json<ErrorEnvelope>)> {
    let _client_request_id = params.client_request_id.as_deref();
    let bearer_outcome = bearer::require_account(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;
    let host_installation_id = uuid::Uuid::parse_str(&params.host_installation_id)
        .map(DeviceId)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                err_body("bad_request", "invalid host_installation_id"),
            )
        })?;

    let outcome = state
        .pairing
        .revoke_link(
            state.registry.as_ref(),
            host_installation_id,
            &bearer_outcome.account_id,
        )
        .await
        .map_err(formal_pairing_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                err_body("not_found", "host link does not exist"),
            )
        })?;

    Ok(Json(ResponseEnvelope::new(
        RevokePairingData {
            host_installation_id: outcome.host_installation_id.to_string(),
            revoked: true,
            remaining_link_count: outcome.remaining_link_count,
            host_installation_token_revoked: outcome.revoked_token_count > 0,
        },
        request_id(&headers),
    )))
}

/// Formal account rail endpoint for listing linked host installations.
///
/// This is the supported account-side host-list contract after the
/// caller-scoped `/v1/me/hosts/query` MVP route was retired. It uses only
/// the bearer token for business identity; no `X-Device-*` header is
/// required.
async fn post_list_hosts(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ResponseEnvelope<ListHostsData>>, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require_account(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;

    let pairs = crate::store::account_host_pairings::list_hosts_for_account(
        &state.store,
        &bearer_outcome.account_id,
    )
    .await
    .map_err(|e| {
        tracing::warn!(
            target: "minos_backend::v1::pairing",
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
        let row = crate::store::devices::get_device(&state.store, pair.host_device_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "minos_backend::v1::pairing",
                    error = %e,
                    host_installation_id = %pair.host_device_id,
                    "get_device(host) failed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err_body("internal", e.to_string()),
                )
            })?;
        hosts.push(FormalHostSummary {
            host_installation_id: pair.host_device_id.to_string(),
            host_display_name: row.map_or_else(|| "unknown".to_string(), |row| row.display_name),
            paired_at_ms: pair.paired_at_ms,
            linked_via_installation_id: pair.paired_via_device_id.to_string(),
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
    let row = crate::store::devices::get_device(&state.store, installation_id)
        .await
        .map_err(|e| {
            tracing::warn!(
                target: "minos_backend::v1::pairing",
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

fn formal_pairing_error(error: FormalPairingError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        FormalPairingError::PairingNotConfirmed => (
            StatusCode::CONFLICT,
            err_body(
                "pairing_not_confirmed",
                "pairing code has not been confirmed",
            ),
        ),
        FormalPairingError::PairingCodeInvalid => (
            StatusCode::CONFLICT,
            err_body(
                "pairing_code_invalid",
                "pairing code is unknown, expired, or already redeemed",
            ),
        ),
        FormalPairingError::PairingStateMismatch { actual } => (
            StatusCode::CONFLICT,
            err_body("pairing_state_mismatch", actual),
        ),
        FormalPairingError::Internal(error) => {
            tracing::warn!(
                target: "minos_backend::v1::pairing",
                error = %error,
                "formal pairing operation failed",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", error.to_string()),
            )
        }
    }
}

/// `DELETE /v1/pairings/:host_device_id`. Bearer-authenticated;
/// dissolves the pair between the bearer's account and `host_device_id`,
/// and pushes `Event::Unpaired` to the Mac's live session if any.
async fn delete_pair_for_host(
    State(state): State<BackendState>,
    headers: HeaderMap,
    axum::extract::Path(host_device_id): axum::extract::Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;
    let host_id = uuid::Uuid::parse_str(&host_device_id)
        .map(minos_domain::DeviceId)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                err_body("bad_request", "invalid host_device_id"),
            )
        })?;

    let existed = state
        .pairing
        .forget_pairing(state.registry.as_ref(), host_id, &bearer_outcome.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            )
        })?;

    if !existed {
        return Err((
            StatusCode::NOT_FOUND,
            err_body("not_found", "pair does not exist"),
        ));
    }

    Ok(StatusCode::NO_CONTENT)
}
