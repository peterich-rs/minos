//! `GET /v1/me/*` — caller's session-scoped views.
//!
//! Post ADR-0020 the legacy `GET /v1/me/peer` is replaced by
//! `GET /v1/me/macs`, which lists every Mac paired to the caller's
//! account. The legacy route returns `410 Gone` so older Mac daemons
//! see an explicit migration signal rather than a silent shape change.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use minos_protocol::{MacSummary, MeMacsResponse};

use crate::auth::bearer;
use crate::http::v1::pairing::{err_body, ErrorEnvelope};
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/me/macs", get(get_me_macs))
        .route("/me/peer", get(get_me_peer_legacy))
}

/// Return every Mac paired to the caller's `account_id`. Bearer-only.
async fn get_me_macs(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<MeMacsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer_outcome = bearer::require(&state, &headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err_body("unauthorized", m))
    })?;

    let pairs = crate::store::account_mac_pairings::list_macs_for_account(
        &state.store,
        &bearer_outcome.account_id,
    )
    .await
    .map_err(|e| {
        tracing::warn!(
            target: "minos_backend::v1::me",
            error = %e,
            account_id = %bearer_outcome.account_id,
            "list_macs_for_account failed",
        );
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("internal", e.to_string()),
        )
    })?;

    let mut macs = Vec::with_capacity(pairs.len());
    for p in pairs {
        let row = crate::store::devices::get_device(&state.store, p.mac_device_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "minos_backend::v1::me",
                    error = %e,
                    mac = %p.mac_device_id,
                    "get_device(mac) failed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err_body("internal", e.to_string()),
                )
            })?;
        let mac_display_name = if let Some(r) = row {
            r.display_name
        } else {
            tracing::warn!(
                target: "minos_backend::v1::me",
                mac = %p.mac_device_id,
                "pair row references device with no devices row; using placeholder name",
            );
            "unknown".into()
        };
        macs.push(MacSummary {
            mac_device_id: p.mac_device_id,
            mac_display_name,
            paired_at_ms: p.paired_at_ms,
            paired_via_device_id: p.paired_via_device_id,
        });
    }

    Ok(Json(MeMacsResponse { macs }))
}

/// Legacy `/v1/me/peer` always returns 410. Older Mac daemons that hit
/// this should migrate to `/v1/me/macs` (or rely on `EventKind::Paired`
/// at WS upgrade time, which already includes peer info).
async fn get_me_peer_legacy() -> (StatusCode, Json<ErrorEnvelope>) {
    (
        StatusCode::GONE,
        err_body("replaced", "Use GET /v1/me/macs"),
    )
}
