//! `GET /v1/me/*` — caller's session-scoped views.
//!
//! Post ADR-0020 `GET /v1/me/hosts` lists every Mac paired to the caller's
//! account on the bearer-authenticated mobile rail. The host-authenticated
//! `GET /v1/me/peer` remains available for the Mac daemon's post-connect
//! refresh path and resolves against the newer `account_host_pairings`
//! table rather than the retired legacy pairings store.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get};
use axum::{Json, Router};
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::envelope::{Envelope, EventKind};
use minos_protocol::{HostPeerSummary, HostSummary, MeHostsResponse, MePeersResponse};

use crate::auth::bearer;
use crate::http::auth;
use crate::http::v1::pairing::{err_body, ErrorEnvelope};
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/me/hosts", get(get_me_hosts))
        .route("/me/peers", get(get_me_peers))
        .route("/me/peers/:mobile_device_id", delete(delete_me_peer))
        .route("/me/peer", get(get_me_peer))
}

/// Return every Mac paired to the caller's `account_id`. Bearer-only.
async fn get_me_hosts(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<MeHostsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require(&state, &headers).map_err(|e| {
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
            target: "minos_backend::v1::me",
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
    for p in pairs {
        let row = crate::store::devices::get_device(&state.store, p.host_device_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "minos_backend::v1::me",
                    error = %e,
                    host = %p.host_device_id,
                    "get_device(host) failed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err_body("internal", e.to_string()),
                )
            })?;
        let host_display_name = if let Some(r) = row {
            r.display_name
        } else {
            tracing::warn!(
                target: "minos_backend::v1::me",
                host = %p.host_device_id,
                "pair row references device with no devices row; using placeholder name",
            );
            "unknown".into()
        };
        hosts.push(HostSummary {
            host_device_id: p.host_device_id,
            host_display_name,
            paired_at_ms: p.paired_at_ms,
            paired_via_device_id: p.paired_via_device_id,
        });
    }

    Ok(Json(MeHostsResponse { hosts }))
}

/// Return every mobile/account pair currently associated with this host.
/// Host rail only (`X-Device-Secret`).
async fn get_me_peers(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<MePeersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
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

    let pairs = crate::store::account_host_pairings::list_accounts_for_host(
        &state.store,
        outcome.device_id,
    )
    .await
    .map_err(|e| {
        tracing::warn!(
            target: "minos_backend::v1::me",
            error = %e,
            host = %outcome.device_id,
            "list_accounts_for_host failed",
        );
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", e.to_string()),
        )
    })?;

    let mut peers = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let mobile = crate::store::devices::get_device(&state.store, pair.paired_via_device_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "minos_backend::v1::me",
                    error = %e,
                    mobile = %pair.paired_via_device_id,
                    "get_device(mobile) failed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err_body("internal", e.to_string()),
                )
            })?;
        let account = crate::store::accounts::find_by_id(&state.store, &pair.mobile_account_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "minos_backend::v1::me",
                    error = %e,
                    account_id = %pair.mobile_account_id,
                    "find_by_id(account) failed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err_body("internal", e.to_string()),
                )
            })?;

        let (mobile_device_name, last_active_at_ms) = match mobile {
            Some(row) => (row.display_name, row.last_seen_at),
            None => {
                tracing::warn!(
                    target: "minos_backend::v1::me",
                    mobile = %pair.paired_via_device_id,
                    "pair row references mobile device with no devices row; using placeholder name",
                );
                ("unknown".into(), pair.paired_at_ms)
            }
        };
        let account_email = match account {
            Some(row) => row.email,
            None => {
                tracing::warn!(
                    target: "minos_backend::v1::me",
                    account_id = %pair.mobile_account_id,
                    "pair row references missing account row; falling back to account id",
                );
                pair.mobile_account_id.clone()
            }
        };

        peers.push(HostPeerSummary {
            mobile_device_id: pair.paired_via_device_id,
            mobile_device_name,
            account_email,
            paired_at_ms: pair.paired_at_ms,
            last_active_at_ms,
            online: state.registry.get(pair.paired_via_device_id).is_some(),
        });
    }

    Ok(Json(MePeersResponse { peers }))
}

/// `DELETE /v1/me/peers/:mobile_device_id`. Host-authenticated;
/// dissolves exactly one mobile/account pair under the caller's host.
async fn delete_me_peer(
    State(state): State<BackendState>,
    headers: HeaderMap,
    axum::extract::Path(mobile_device_id): axum::extract::Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
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
    let mobile_id = uuid::Uuid::parse_str(&mobile_device_id)
        .map(DeviceId)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                err_body("bad_request", "invalid mobile_device_id"),
            )
        })?;

    let pair = crate::store::account_host_pairings::list_accounts_for_host(
        &state.store,
        outcome.device_id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", e.to_string()),
        )
    })?
    .into_iter()
    .find(|row| row.paired_via_device_id == mobile_id)
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            err_body("not_found", "pair does not exist"),
        )
    })?;

    crate::store::account_host_pairings::delete_pair(
        &state.store,
        outcome.device_id,
        &pair.mobile_account_id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", e.to_string()),
        )
    })?;

    if let Some(host_handle) = state.registry.get(outcome.device_id) {
        let _ = state.registry.try_send_current(
            &host_handle,
            Envelope::Event {
                version: 1,
                event: EventKind::Unpaired,
            },
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Return the most recently paired mobile peer for this host device.
///
/// The host rail is still authenticated by `X-Device-Secret`, so the
/// daemon can reuse this route after reconnect to rebuild its in-memory
/// peer mirror even though the durable pairing rows moved to
/// `account_host_pairings`.
async fn get_me_peer(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<minos_protocol::MePeerResponse>, (StatusCode, Json<ErrorEnvelope>)> {
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

    let pair = crate::store::account_host_pairings::list_accounts_for_host(
        &state.store,
        outcome.device_id,
    )
    .await
    .map_err(|e| {
        tracing::warn!(
            target: "minos_backend::v1::me",
            error = %e,
            host = %outcome.device_id,
            "list_accounts_for_host failed",
        );
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", e.to_string()),
        )
    })?
    .into_iter()
    .next()
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            err_body("not_paired", "host has no paired peer"),
        )
    })?;

    let peer_name = match crate::store::devices::get_device(&state.store, pair.paired_via_device_id)
        .await
        .map_err(|e| {
            tracing::warn!(
                target: "minos_backend::v1::me",
                error = %e,
                peer = %pair.paired_via_device_id,
                "get_device(peer) failed",
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_body("internal", e.to_string()),
            )
        })? {
        Some(row) => row.display_name,
        None => {
            tracing::warn!(
                target: "minos_backend::v1::me",
                peer = %pair.paired_via_device_id,
                "pair row references mobile device with no devices row; using placeholder name",
            );
            "unknown".into()
        }
    };

    Ok(Json(minos_protocol::MePeerResponse {
        peer_device_id: pair.paired_via_device_id,
        peer_name,
        paired_at_ms: pair.paired_at_ms,
    }))
}
