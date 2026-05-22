use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::approval_relay::ApprovalDecisionInput;
use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::BackendState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRespondRequest {
    request_id: String,
    decision: Value,
    client_request_id: Option<String>,
}

impl From<ApprovalRespondRequest> for ApprovalDecisionInput {
    fn from(req: ApprovalRespondRequest) -> Self {
        ApprovalDecisionInput::new(req.request_id, req.decision, req.client_request_id)
    }
}

pub fn router() -> Router<BackendState> {
    Router::new().route("/approvals/respond", post(submit_approval_decision))
}

pub(crate) async fn submit_approval_decision_inner(
    state: BackendState,
    headers: HeaderMap,
    req: ApprovalDecisionInput,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    match state.approval_relay.submit_decision(&account_id, req).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            err("approval_not_found", "pending approval not found"),
        )),
        Err(crate::error::BackendError::ForwardRpc { message, .. })
            if message.contains("invalid decision") =>
        {
            Err((StatusCode::BAD_REQUEST, err("bad_request", message)))
        }
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            err("internal", error.to_string()),
        )),
    }
}

async fn submit_approval_decision(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<ApprovalRespondRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    submit_approval_decision_inner(state, headers, req.into()).await
}
