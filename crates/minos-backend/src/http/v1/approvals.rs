use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::approvals::{ApprovalError, RespondApprovalInput};
use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::BackendState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRespondRequest {
    request_id: String,
    decision: Value,
    client_request_id: Option<String>,
}

pub fn router() -> Router<BackendState> {
    Router::new().route("/approvals/respond", post(submit_approval_decision))
}

pub(crate) async fn submit_approval_decision_inner(
    state: BackendState,
    headers: HeaderMap,
    req: RespondApprovalInput,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    match state
        .approvals
        .respond(RespondApprovalInput {
            request_id: req.request_id,
            decision: req.decision,
            client_request_id: req.client_request_id,
            caller_account_id: account_id,
        })
        .await
    {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(ApprovalError::NotFound) => Err((
            StatusCode::NOT_FOUND,
            err("approval_not_found", "pending approval not found"),
        )),
        Err(ApprovalError::AlreadyResolved) => Err((
            StatusCode::CONFLICT,
            err("approval_already_resolved", "approval already resolved"),
        )),
        Err(ApprovalError::Forbidden) => Err((
            StatusCode::FORBIDDEN,
            err("conversation_forbidden", "approval is not visible to this account"),
        )),
        Err(ApprovalError::ValidationFormat(message)) => {
            Err((StatusCode::BAD_REQUEST, err("validation_format", message)))
        }
        Err(ApprovalError::Internal(error)) => Err((
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
    submit_approval_decision_inner(
        state,
        headers,
        RespondApprovalInput {
            request_id: req.request_id,
            decision: req.decision,
            client_request_id: req.client_request_id,
            caller_account_id: String::new(),
        },
    )
    .await
}
