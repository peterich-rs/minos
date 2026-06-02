//! Account-authenticated host command proxy routes.
//!
//! These endpoints let mobile callers target one paired host over the backend's
//! durable host-command queue instead of the legacy relay `Envelope::Forward`
//! path. They intentionally mirror a narrow subset of the daemon RPC surface.

use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use minos_domain::DeviceId;
use minos_protocol::{
    ListClisResponse, ListHostClisRequest, ListHostSkillsCommandRequest, ListHostSkillsRequest,
    ListHostSkillsResponse, WriteHostSkillConfigCommandRequest, WriteHostSkillConfigRequest,
    WriteHostSkillConfigResponse,
};
use uuid::Uuid;

use crate::error::BackendError;
use crate::http::error_response::{err_json, ErrorEnvelope};
use crate::http::BackendState;

const LIST_CLIS_METHOD: &str = "minos_list_clis";
const LIST_HOST_SKILLS_METHOD: &str = "minos_list_host_skills";
const WRITE_HOST_SKILL_CONFIG_METHOD: &str = "minos_write_host_skill_config";
const LIST_CLIS_TIMEOUT: Duration = Duration::from_secs(10);
const HOST_SKILLS_TIMEOUT: Duration = Duration::from_secs(15);

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/host-commands/list-clis", post(list_clis))
        .route("/host-commands/list-host-skills", post(list_host_skills))
        .route(
            "/host-commands/write-host-skill-config",
            post(write_host_skill_config),
        )
}

async fn list_clis(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<ListHostClisRequest>,
) -> Result<Json<ListClisResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller_device_id, account_id) = super::require_authed_session(&state, &headers).await?;
    let host_device_id =
        require_paired_host(&state, &account_id, &request.host_installation_id).await?;

    let response = state
        .host_commands
        .dispatch_json(
            &format!("cmd-http-list-clis-{account_id}-{host_device_id}"),
            host_device_id,
            None,
            LIST_CLIS_METHOD,
            &serde_json::Value::Null,
            Some(&account_id),
            LIST_CLIS_TIMEOUT,
        )
        .await
        .map_err(map_backend_error)?;

    let body = serde_json::from_value(response)
        .map_err(|error| invalid_host_response(LIST_CLIS_METHOD, error))?;
    Ok(Json(body))
}

async fn list_host_skills(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<ListHostSkillsCommandRequest>,
) -> Result<Json<ListHostSkillsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller_device_id, account_id) = super::require_authed_session(&state, &headers).await?;
    let host_device_id =
        require_paired_host(&state, &account_id, &request.host_installation_id).await?;
    let params = serde_json::to_value(ListHostSkillsRequest {
        workspace: request.workspace,
        force_reload: request.force_reload,
    })
    .map_err(|error| invalid_host_response(LIST_HOST_SKILLS_METHOD, error))?;

    let response = state
        .host_commands
        .dispatch_json(
            &format!("cmd-http-list-host-skills-{account_id}-{host_device_id}"),
            host_device_id,
            None,
            LIST_HOST_SKILLS_METHOD,
            &params,
            Some(&account_id),
            HOST_SKILLS_TIMEOUT,
        )
        .await
        .map_err(map_backend_error)?;

    let body = serde_json::from_value(response)
        .map_err(|error| invalid_host_response(LIST_HOST_SKILLS_METHOD, error))?;
    Ok(Json(body))
}

async fn write_host_skill_config(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<WriteHostSkillConfigCommandRequest>,
) -> Result<Json<WriteHostSkillConfigResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller_device_id, account_id) = super::require_authed_session(&state, &headers).await?;
    let host_device_id =
        require_paired_host(&state, &account_id, &request.host_installation_id).await?;
    let params = serde_json::to_value(WriteHostSkillConfigRequest {
        workspace: request.workspace,
        path: request.path,
        enabled: request.enabled,
    })
    .map_err(|error| invalid_host_response(WRITE_HOST_SKILL_CONFIG_METHOD, error))?;

    let response = state
        .host_commands
        .dispatch_json(
            &format!("cmd-http-write-host-skill-config-{account_id}-{host_device_id}"),
            host_device_id,
            None,
            WRITE_HOST_SKILL_CONFIG_METHOD,
            &params,
            Some(&account_id),
            HOST_SKILLS_TIMEOUT,
        )
        .await
        .map_err(map_backend_error)?;

    let body = serde_json::from_value(response)
        .map_err(|error| invalid_host_response(WRITE_HOST_SKILL_CONFIG_METHOD, error))?;
    Ok(Json(body))
}

async fn require_paired_host(
    state: &BackendState,
    account_id: &str,
    host_installation_id: &str,
) -> Result<DeviceId, (StatusCode, Json<ErrorEnvelope>)> {
    let host_device_id = Uuid::parse_str(host_installation_id)
        .map(DeviceId)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                err_json("bad_request", "invalid host_installation_id"),
            )
        })?;

    let paired = state
        .data
        .repos
        .account_host_pairings
        .exists(host_device_id, account_id)
        .await
        .map_err(map_backend_error)?;

    if paired {
        Ok(host_device_id)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            err_json("forbidden", "host not paired to account"),
        ))
    }
}

fn invalid_host_response(
    method: &'static str,
    error: serde_json::Error,
) -> (StatusCode, Json<ErrorEnvelope>) {
    (
        StatusCode::BAD_GATEWAY,
        err_json(
            "host_command_invalid_response",
            format!("{method} returned invalid JSON: {error}"),
        ),
    )
}

fn map_backend_error(error: BackendError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        BackendError::ForwardRpcTimeout { .. } => (
            StatusCode::GATEWAY_TIMEOUT,
            err_json("host_command_timeout", "host command timed out"),
        ),
        BackendError::ForwardRpc { message, .. } => (
            StatusCode::BAD_GATEWAY,
            err_json("host_command_failed", message),
        ),
        BackendError::PeerOffline { .. } => (
            StatusCode::CONFLICT,
            err_json("peer_offline", "host is offline"),
        ),
        BackendError::PeerBackpressure { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            err_json("peer_backpressure", "host is busy"),
        ),
        other => {
            tracing::warn!(
                target: "minos_backend::v1::host_commands",
                error = %other,
                "host command proxy failed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_json("internal", other.to_string()),
            )
        }
    }
}
