//! Account-scoped project handlers.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, post};
use axum::{Json, Router};
use minos_protocol::{
    AssignProjectThreadRequest, CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest,
    ListProjectThreadsParams, ListProjectThreadsResponse, ListProjectsResponse,
    UpdateProjectRequest,
};
use serde::Serialize;

use crate::auth::bearer;
use crate::http::auth;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/projects", post(create_project))
        .route("/projects/query", post(list_projects))
        .route("/projects/update", post(update_project))
        .route("/projects/delete", post(delete_project_query))
        .route("/projects/:project_id", delete(delete_project_path))
        .route("/projects/threads/assign", post(assign_project_thread))
        .route("/projects/threads/query", post(list_project_threads))
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

fn err(code: &'static str, message: impl Into<String>) -> Json<ErrorEnvelope> {
    Json(ErrorEnvelope {
        error: ErrorBody {
            code,
            message: message.into(),
        },
    })
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
    let projects = crate::store::projects::list(&state.store, &account_id)
        .await
        .map_err(internal)?;
    Ok(Json(ListProjectsResponse { projects }))
}

async fn create_project(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<CreateProjectResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    let name = req.name.trim();
    let workspace_slug = req.workspace_slug.trim();
    if name.is_empty() || !valid_workspace_slug(workspace_slug) {
        return Err((
            StatusCode::BAD_REQUEST,
            err(
                "bad_request",
                "project name and a valid workspace_slug are required",
            ),
        ));
    }
    let project_id = uuid::Uuid::new_v4().to_string();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let project = crate::store::projects::create(
        &state.store,
        &project_id,
        &account_id,
        name,
        workspace_slug,
        now_ms,
    )
    .await
    .map_err(internal)?;
    Ok(Json(CreateProjectResponse { project }))
}

async fn update_project(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            err("bad_request", "project name is required"),
        ));
    }
    crate::store::projects::update_name(
        &state.store,
        &account_id,
        &req.project_id,
        name,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_project_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<DeleteProjectRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    delete_project_inner(state, headers, req.project_id).await
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
    crate::store::projects::delete(&state.store, &account_id, &project_id)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_project_threads(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<ListProjectThreadsParams>,
) -> Result<Json<ListProjectThreadsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    let threads = crate::store::projects::list_threads(
        &state.store,
        &account_id,
        &req.project_id,
        req.before_ts_ms,
        req.limit.min(500),
    )
    .await
    .map_err(internal)?;
    Ok(Json(ListProjectThreadsResponse { threads }))
}

async fn assign_project_thread(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<AssignProjectThreadRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account(&state, &headers).await?;
    if req.project_id.trim().is_empty() || req.thread_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            err("bad_request", "project_id and thread_id are required"),
        ));
    }
    crate::store::projects::assign_thread(
        &state.store,
        &account_id,
        &req.project_id,
        &req.thread_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn internal(error: crate::error::BackendError) -> (StatusCode, Json<ErrorEnvelope>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        err("internal", error.to_string()),
    )
}

fn valid_workspace_slug(slug: &str) -> bool {
    !slug.is_empty() && slug != "." && slug != ".." && !slug.contains('/') && !slug.contains('\\')
}
