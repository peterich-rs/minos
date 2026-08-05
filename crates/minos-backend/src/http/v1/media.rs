//! `/v1/media/*` — account-scoped blob upload / download via R2 or local store.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::v1::require_authed_session;
use crate::http::BackendState;
use crate::media::{CreateUploadResult, DownloadResult, MediaBlobDto, MediaError};

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/media/blobs", post(create_blob))
        .route("/media/blobs/get", post(get_blob))
        .route("/media/blobs/delete", post(delete_blob))
        .route(
            "/media/blobs/:blob_id/content",
            put(put_content).get(get_content),
        )
        .route("/media/status", get(media_status))
}

// ── Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateBlobRequest {
    content_type: String,
    byte_size: u64,
    #[serde(default)]
    original_filename: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateBlobResponse {
    #[serde(flatten)]
    result: CreateUploadResult,
}

#[derive(Debug, Deserialize)]
struct GetBlobRequest {
    blob_id: String,
}

#[derive(Debug, Serialize)]
struct GetBlobResponse {
    #[serde(flatten)]
    result: DownloadResult,
}

#[derive(Debug, Deserialize)]
struct DeleteBlobRequest {
    blob_id: String,
}

#[derive(Debug, Serialize)]
struct DeleteBlobResponse {
    blob: MediaBlobDto,
}

#[derive(Debug, Deserialize)]
struct ContentQuery {
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct MediaStatusResponse {
    configured: bool,
    backend: &'static str,
    max_bytes: u64,
}

// ── Handlers ───────────────────────────────────────────────────────────

async fn media_status(State(state): State<BackendState>) -> Json<MediaStatusResponse> {
    Json(MediaStatusResponse {
        configured: state.media.is_configured(),
        backend: state.media.backend_name(),
        max_bytes: state.media.config_max_bytes(),
    })
}

async fn create_blob(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<CreateBlobRequest>,
) -> Result<Json<CreateBlobResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_device_id, account_id) = require_authed_session(&state, &headers).await?;
    let result = state
        .media
        .create_upload(
            &account_id,
            &req.content_type,
            req.byte_size,
            req.original_filename.as_deref(),
        )
        .await
        .map_err(media_err)?;
    Ok(Json(CreateBlobResponse { result }))
}

async fn put_content(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<MediaBlobDto>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_device_id, account_id) = require_authed_session(&state, &headers).await?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let dto = state
        .media
        .put_content(
            &account_id,
            &blob_id,
            bytes::Bytes::from(body.to_vec()),
            content_type,
        )
        .await
        .map_err(media_err)?;
    Ok(Json(dto))
}

async fn get_blob(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<GetBlobRequest>,
) -> Result<Json<GetBlobResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_device_id, account_id) = require_authed_session(&state, &headers).await?;
    let result = state
        .media
        .get_download(&account_id, &req.blob_id)
        .await
        .map_err(media_err)?;
    Ok(Json(GetBlobResponse { result }))
}

async fn delete_blob(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<DeleteBlobRequest>,
) -> Result<Json<DeleteBlobResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_device_id, account_id) = require_authed_session(&state, &headers).await?;
    let blob = state
        .media
        .delete(&account_id, &req.blob_id)
        .await
        .map_err(media_err)?;
    Ok(Json(DeleteBlobResponse { blob }))
}

async fn get_content(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
    Query(query): Query<ContentQuery>,
) -> Result<Response, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = match require_authed_session(&state, &headers).await {
        Ok((_d, aid)) => Some(aid),
        Err(_) if query.token.is_some() => None,
        Err(e) => return Err(e),
    };
    let row = state
        .media
        .authorize_read(&blob_id, account_id.as_deref(), query.token.as_deref())
        .await
        .map_err(media_err)?;
    let bytes = state.media.read_content(&row).await.map_err(media_err)?;
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_str(&row.content_type)
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, max-age=300"),
    );
    if let Some(name) = &row.original_filename {
        if let Ok(v) = header::HeaderValue::from_str(&format!(
            "inline; filename=\"{}\"",
            name.replace('"', "")
        )) {
            response
                .headers_mut()
                .insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(response)
}

fn media_err(e: MediaError) -> (StatusCode, Json<ErrorEnvelope>) {
    (e.status_code(), err(e.code(), e.to_string()))
}

// Silence unused import warning for IntoResponse if needed.
#[allow(dead_code)]
fn _assert_into_response() {
    let _: fn() -> Response = || StatusCode::OK.into_response();
}
