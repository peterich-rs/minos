//! Additive `/v1/agent-sessions/*` routes.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/agent-sessions/list", post(list_sessions))
        .route("/agent-sessions/read-turns", post(read_turns))
}

#[derive(Debug, Deserialize)]
struct ListAgentSessionsRequest {
    conversation_id: Option<String>,
    project_id: Option<String>,
    before_started_at_ms: Option<i64>,
    limit: u32,
}

#[derive(Debug, Serialize)]
struct ListAgentSessionsResponse {
    sessions: Vec<AgentSessionSummary>,
    next_before_started_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct AgentSessionSummary {
    session_id: String,
    conversation_id: String,
    project_id: Option<String>,
    agent_id: Option<String>,
    status: String,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ReadTurnsRequest {
    session_id: Option<String>,
    turn_id: Option<String>,
    after_turn_seq: Option<i64>,
    after_event_seq: Option<i64>,
    limit: u32,
}

#[derive(Debug, Serialize)]
struct ReadTurnsResponse {
    session_id: Option<String>,
    turn_id: Option<String>,
    turns: Vec<ReadTurnMetadata>,
    events: Vec<ReadTurnEvent>,
    next_turn_seq: Option<i64>,
    next_event_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ReadTurnMetadata {
    turn_id: String,
    turn_seq: i64,
    role: String,
    status: String,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    summary_text: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReadTurnEvent {
    turn_id: String,
    event_seq: i64,
    kind: String,
    payload: serde_json::Value,
    created_at_ms: i64,
}

async fn read_turns(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<ReadTurnsRequest>,
) -> Result<Json<ReadTurnsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;

    if let Some(turn_id) = request.turn_id.as_deref() {
        return read_turn_events_mode(
            state,
            &account_id,
            turn_id,
            request.after_event_seq,
            request.limit,
        )
        .await;
    }

    let session_id = request.session_id.as_deref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            err(
                "invalid_request",
                "session_id is required when turn_id is absent",
            ),
        )
    })?;

    read_turn_metadata_mode(
        state,
        &account_id,
        session_id,
        request.after_turn_seq,
        request.limit,
    )
    .await
}

async fn list_sessions(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<ListAgentSessionsRequest>,
) -> Result<Json<ListAgentSessionsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    let limit = request.limit.min(500);

    let rows = match (
        request.conversation_id.as_deref(),
        request.project_id.as_deref(),
    ) {
        (Some(_), Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                err(
                    "invalid_request",
                    "conversation_id and project_id cannot be combined",
                ),
            ));
        }
        (Some(conversation_id), None) => {
            crate::store::agent_sessions::list_for_account_conversation(
                &state.store,
                conversation_id,
                &account_id,
                request.before_started_at_ms,
                limit,
            )
            .await
            .map_err(internal_error)?
        }
        (None, Some(project_id)) => crate::store::agent_sessions::list_for_account_project(
            &state.store,
            project_id,
            &account_id,
            request.before_started_at_ms,
            limit,
        )
        .await
        .map_err(internal_error)?,
        (None, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                err(
                    "invalid_request",
                    "conversation_id or project_id is required",
                ),
            ));
        }
    };

    let next_before_started_at_ms = if rows.len() == usize::try_from(limit).unwrap_or(usize::MAX)
    {
        rows.last().map(|row| row.started_at_ms)
    } else {
        None
    };
    let sessions = rows.into_iter().map(session_summary).collect();

    Ok(Json(ListAgentSessionsResponse {
        sessions,
        next_before_started_at_ms,
    }))
}

async fn read_turn_metadata_mode(
    state: BackendState,
    account_id: &str,
    session_id: &str,
    after_turn_seq: Option<i64>,
    limit: u32,
) -> Result<Json<ReadTurnsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let session =
        crate::store::agent_sessions::get_for_account(&state.store, session_id, account_id)
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

    let turns = crate::store::agent_turns::list_for_session(
        &state.store,
        &session.session_id,
        after_turn_seq,
        limit.min(500),
    )
    .await
    .map_err(internal_error)?;

    let next_turn_seq = turns.last().map(|turn| turn.turn_seq);
    let turns = turns
        .into_iter()
        .map(|turn| ReadTurnMetadata {
            turn_id: turn.turn_id,
            turn_seq: turn.turn_seq,
            role: turn.role,
            status: turn.status,
            started_at_ms: turn.started_at_ms,
            finished_at_ms: turn.finished_at_ms,
            summary_text: turn.summary_text,
        })
        .collect();

    Ok(Json(ReadTurnsResponse {
        session_id: Some(session.session_id),
        turn_id: None,
        turns,
        events: Vec::new(),
        next_turn_seq,
        next_event_seq: None,
    }))
}

async fn read_turn_events_mode(
    state: BackendState,
    account_id: &str,
    turn_id: &str,
    after_event_seq: Option<i64>,
    limit: u32,
) -> Result<Json<ReadTurnsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let turn = crate::store::agent_turns::get_for_account(&state.store, turn_id, account_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                err(
                    "agent_turn_not_found",
                    format!("agent turn not found: {turn_id}"),
                ),
            )
        })?;

    let events = crate::store::agent_turn_events::list_for_turn(
        &state.store,
        &turn.turn_id,
        after_event_seq,
        limit.min(500),
    )
    .await
    .map_err(internal_error)?;

    let mut decoded_events = Vec::with_capacity(events.len());
    for event in events {
        let payload = serde_json::from_str(&event.payload_json).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err(
                    "internal",
                    format!("invalid stored turn event payload: {e}"),
                ),
            )
        })?;
        decoded_events.push(ReadTurnEvent {
            turn_id: event.turn_id,
            event_seq: event.event_seq,
            kind: event.kind,
            payload,
            created_at_ms: event.created_at_ms,
        });
    }
    let next_event_seq = decoded_events.last().map(|event| event.event_seq);

    Ok(Json(ReadTurnsResponse {
        session_id: Some(turn.agent_session_id),
        turn_id: Some(turn.turn_id),
        turns: Vec::new(),
        events: decoded_events,
        next_turn_seq: None,
        next_event_seq,
    }))
}

fn internal_error(error: crate::error::BackendError) -> (StatusCode, Json<ErrorEnvelope>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        err("internal", error.to_string()),
    )
}

fn session_summary(row: crate::store::agent_sessions::AgentSessionRow) -> AgentSessionSummary {
    AgentSessionSummary {
        session_id: row.session_id,
        conversation_id: row.conversation_id,
        project_id: row.project_id,
        agent_id: row.agent_id,
        status: row.status,
        started_at_ms: row.started_at_ms,
        ended_at_ms: row.ended_at_ms,
    }
}
