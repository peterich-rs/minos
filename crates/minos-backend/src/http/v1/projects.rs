//! Account-scoped project handlers.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, post};
use axum::{Json, Router};
use minos_domain::AgentName;
use minos_protocol::{
    ArchiveProjectRequest, CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest,
    ListProjectsResponse, UpdateProjectRequest,
};
use minos_ui_protocol::SessionEndReason;
use serde::{Deserialize, Serialize};

use crate::auth::bearer;
use crate::http::auth;
use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::BackendState;

#[derive(Debug, Deserialize)]
struct LinkProjectConversationRequest {
    project_id: String,
    conversation_id: String,
}

#[derive(Debug, Deserialize)]
struct LinkProjectAgentSessionRequest {
    project_id: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct ListProjectAgentSessionsRequest {
    project_id: String,
    before_started_at_ms: Option<i64>,
    limit: u32,
}

#[derive(Debug, Serialize)]
struct ProjectAgentSessionsResponse {
    sessions: Vec<ProjectAgentSessionSummary>,
    next_before_started_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ProjectAgentSessionSummary {
    session_id: String,
    conversation_id: String,
    project_id: Option<String>,
    agent_id: Option<String>,
    agent: Option<AgentName>,
    status: String,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    title: Option<String>,
    last_activity_at_ms: i64,
    message_count: u32,
    end_reason: Option<SessionEndReason>,
}

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/projects", post(create_project))
        .route("/projects/create", post(create_project))
        .route("/projects/query", post(list_projects))
        .route("/projects/list", post(list_projects))
        .route("/projects/update", post(update_project))
        .route("/projects/rename", post(update_project))
        .route("/projects/delete", post(delete_project_query))
        .route("/projects/archive", post(archive_project))
        .route("/projects/:project_id", delete(delete_project_path))
        .route(
            "/projects/link-conversation",
            post(link_project_conversation),
        )
        .route(
            "/projects/conversations/link",
            post(link_project_conversation),
        )
        .route(
            "/projects/agent-sessions/link",
            post(link_project_agent_session),
        )
        .route(
            "/projects/agent-sessions/query",
            post(list_project_agent_sessions),
        )
}

async fn require_account(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<ErrorEnvelope>)> {
    auth::authenticate(&state.store, headers)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, err("unauthorized", m)),
            auth::AuthError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal", m)),
        })?;
    let bearer_outcome = bearer::require(state, headers).map_err(|e| {
        let (s, m) = e.into_response_tuple();
        (s, err("unauthorized", m))
    })?;
    Ok(bearer_outcome.account_id)
}

async fn list_projects(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ListProjectsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    let projects = state
        .projects
        .list(&account_id)
        .await
        .map_err(project_error)?;
    Ok(Json(ListProjectsResponse { projects }))
}

async fn create_project(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<CreateProjectResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    let project = state
        .projects
        .create(&account_id, req)
        .await
        .map_err(project_error)?;
    Ok(Json(CreateProjectResponse { project }))
}

async fn update_project(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    state
        .projects
        .update_name(&account_id, req)
        .await
        .map_err(project_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_project_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<DeleteProjectRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    delete_project_inner(state, headers, req.project_id).await
}

async fn archive_project(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<ArchiveProjectRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    if req.project_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            err("bad_request", "project_id is required"),
        ));
    }
    let account_id = require_account(&state, &headers).await?;
    let archived = state
        .projects
        .archive(&account_id, &req.project_id)
        .await
        .map_err(project_error)?;
    if !archived {
        return Err((
            StatusCode::NOT_FOUND,
            err("not_found", "project not found or already archived"),
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_project_path(
    State(state): State<BackendState>,
    headers: HeaderMap,
    axum::extract::Path(project_id): axum::extract::Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    delete_project_inner(state, headers, project_id).await
}

async fn delete_project_inner(
    state: BackendState,
    headers: HeaderMap,
    project_id: String,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    state
        .projects
        .delete(&account_id, &project_id)
        .await
        .map_err(project_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn link_project_conversation(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LinkProjectConversationRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    if req.project_id.trim().is_empty() || req.conversation_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            err("bad_request", "project_id and conversation_id are required"),
        ));
    }

    let account_id = require_account(&state, &headers).await?;
    ensure_project_exists(&state.store, &account_id, &req.project_id).await?;
    let session = crate::store::agent_sessions::latest_for_account_conversation(
        &state.store,
        &req.conversation_id,
        &account_id,
    )
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            err("internal", error.to_string()),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            err(
                "conversation_not_found",
                "conversation not found or has no agent session",
            ),
        )
    })?;

    link_project_session_inner(&state, &account_id, &req.project_id, &session.session_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn link_project_agent_session(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LinkProjectAgentSessionRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    if req.project_id.trim().is_empty() || req.session_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            err("bad_request", "project_id and session_id are required"),
        ));
    }

    let account_id = require_account(&state, &headers).await?;
    link_project_session_inner(&state, &account_id, &req.project_id, &req.session_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn list_project_agent_sessions(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<ListProjectAgentSessionsRequest>,
) -> Result<Json<ProjectAgentSessionsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    ensure_project_exists(&state.store, &account_id, &req.project_id).await?;

    let limit = req.limit.min(500);
    let rows = crate::store::agent_sessions::list_for_account_project(
        &state.store,
        &req.project_id,
        &account_id,
        req.before_started_at_ms,
        limit,
    )
    .await
    .map_err(internal_error)?;
    let session_ids = rows
        .iter()
        .map(|row| row.session_id.clone())
        .collect::<Vec<_>>();
    let thread_summaries =
        crate::store::sessions::summaries_for_ids(&state.store, &account_id, &session_ids)
            .await
            .map_err(internal_error)?;
    let next_before_started_at_ms = if rows.len() == usize::try_from(limit).unwrap_or(usize::MAX) {
        rows.last().map(|row| row.started_at_ms)
    } else {
        None
    };
    let sessions = rows
        .into_iter()
        .map(|row| {
            let session_id = row.session_id.clone();
            project_agent_session_summary(row, thread_summaries.get(&session_id))
        })
        .collect();

    Ok(Json(ProjectAgentSessionsResponse {
        sessions,
        next_before_started_at_ms,
    }))
}

async fn link_project_session_inner(
    state: &BackendState,
    account_id: &str,
    project_id: &str,
    session_id: &str,
) -> Result<(), (StatusCode, Json<ErrorEnvelope>)> {
    ensure_project_exists(&state.store, account_id, project_id).await?;

    crate::store::agent_sessions::assign_project_for_account(
        &state.store,
        session_id,
        account_id,
        Some(project_id),
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            err(
                "agent_session_not_found",
                format!("agent session not found: {session_id}"),
            ),
        )
    })?;

    Ok(())
}

fn project_error(error: crate::project::ProjectError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        crate::project::ProjectError::InvalidInput(message) => {
            (StatusCode::BAD_REQUEST, err("bad_request", message))
        }
        crate::project::ProjectError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            err("internal", error.to_string()),
        ),
    }
}

fn project_agent_session_summary(
    row: crate::store::agent_sessions::AgentSessionRow,
    thread: Option<&minos_protocol::SessionSummary>,
) -> ProjectAgentSessionSummary {
    let agent = thread
        .map(|summary| summary.agent)
        .or_else(|| row.agent_id.as_deref().and_then(agent_name_from_agent_id));
    let ended_at_ms = thread
        .and_then(|summary| summary.ended_at_ms)
        .or(row.ended_at_ms);
    let last_activity_at_ms = thread
        .map(|summary| summary.last_ts_ms)
        .unwrap_or_else(|| ended_at_ms.unwrap_or(row.started_at_ms));
    ProjectAgentSessionSummary {
        session_id: row.session_id,
        conversation_id: row.conversation_id,
        project_id: row.project_id,
        agent_id: row.agent_id,
        agent,
        status: row.status,
        started_at_ms: row.started_at_ms,
        ended_at_ms,
        title: thread.and_then(|summary| summary.title.clone()),
        last_activity_at_ms,
        message_count: thread.map(|summary| summary.message_count).unwrap_or(0),
        end_reason: thread.and_then(|summary| summary.end_reason.clone()),
    }
}

fn agent_name_from_agent_id(agent_id: &str) -> Option<AgentName> {
    match agent_id {
        "agent_codex" | "codex" => Some(AgentName::Codex),
        "agent_claude" | "claude" => Some(AgentName::Claude),
        "agent_gemini" | "gemini" => Some(AgentName::Gemini),
        _ => None,
    }
}

async fn ensure_project_exists(
    store: &impl crate::store::AsStorePool,
    account_id: &str,
    project_id: &str,
) -> Result<(), (StatusCode, Json<ErrorEnvelope>)> {
    if crate::store::projects::exists(store, account_id, project_id)
        .await
        .map_err(internal_error)?
    {
        Ok(())
    } else {
        Err((
            StatusCode::NOT_FOUND,
            err(
                "project_not_found",
                format!("project not found: {project_id}"),
            ),
        ))
    }
}

fn internal_error(error: crate::error::BackendError) -> (StatusCode, Json<ErrorEnvelope>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        err("internal", error.to_string()),
    )
}
