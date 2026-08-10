//! Formal host rail.
//!
//! Bootstrap endpoints use nonce-bound Ed25519 proof. Steady-state host
//! endpoints use the opaque host installation token issued by
//! `POST /v1/hosts/link`.

use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{extract::State, Json, Router};
use serde::Deserialize;
use serde::Serialize;

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
    last_active_at_ms: i64,
    online: bool,
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

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/host/bootstrap/nonce", post(post_bootstrap_nonce))
        .route("/host/installations/self", post(post_installations_self))
    // Host realtime: Authorization: Bearer hit_* on /ws/host only (no ticket).
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
        .issue(&req.installation_id, chrono::Utc::now().timestamp_millis())
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "minos_backend::v1::host",
                error = %error,
                installation_id = %req.installation_id,
                "bootstrap nonce issue failed",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
        })?;

    Ok(Json(ResponseEnvelope::new(
        BootstrapNonceData {
            nonce: nonce.nonce,
            expires_at_ms: nonce.expires_at_ms,
        },
        request_id(&headers),
    )))
}

async fn post_installations_self(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ResponseEnvelope<HostSelfData>>, (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = require_host(&state, &headers).await?;
    let host =
        crate::store::device_installations::get_device(&state.store, outcome.host_installation_id)
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

    let pairs = crate::store::host_links::list_accounts_for_host(
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
        let mobile =
            crate::store::device_installations::get_device(&state.store, pair.paired_via_device_id)
                .await
                .map_err(|error| {
                    tracing::warn!(
                        target: "minos_backend::v1::host",
                        error = %error,
                        linked_via_installation_id = %pair.paired_via_device_id,
                        "get_device(linked_via) failed",
                    );
                    (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
                })?;

        let latest_mobile = crate::store::device_installations::latest_mobile_for_account(
            &state.store,
            &pair.mobile_account_id,
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "minos_backend::v1::host",
                error = %error,
                mobile_account_id = %pair.mobile_account_id,
                "latest_mobile_for_account failed",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, err("internal"))
        })?;

        let (display_name, last_active_at_ms) = if let Some(row) = mobile {
            let last_active_at_ms = latest_mobile
                .as_ref()
                .map_or(row.last_seen_at, |latest| latest.last_seen_at);
            (row.display_name, last_active_at_ms)
        } else {
            tracing::warn!(
                target: "minos_backend::v1::host",
                linked_via_installation_id = %pair.paired_via_device_id,
                "host link references mobile device with no devices row; using placeholder name",
            );
            latest_mobile.map_or_else(
                || ("unknown".to_string(), pair.paired_at_ms),
                |latest| (latest.display_name, latest.last_seen_at),
            )
        };

        links.push(HostSelfLinkSummary {
            linked_via_installation_id: pair.paired_via_device_id.to_string(),
            link_display_name: display_name,
            paired_at_ms: pair.paired_at_ms,
            last_active_at_ms,
            online: is_account_online(&state, &pair.mobile_account_id),
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

/// IM **account online**: at least one Mobile client has a live `/ws/client`
/// session for this account (user reachable on the phone app).
///
/// Not browser/desktop account shells — product account presence is mobile.
fn is_account_online(state: &BackendState, account_id: &str) -> bool {
    state.registry.mobile_client_session_count(account_id) > 0
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
