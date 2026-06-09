//! Additive `/v1/agent-sessions/*` routes.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use minos_domain::AgentName;
use minos_ui_protocol::ThreadEndReason;
use serde::{Deserialize, Serialize};

use crate::agent_sessions::{AgentSessionError, ListAgentSessionsInput, ReadTurnsInput};
use crate::http::error_response::{err_json as err, ErrorEnvelope};
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/agent-sessions/start", post(start_session))
        .route("/agent-sessions/send-input", post(send_input))
        .route("/agent-sessions/stop", post(stop_session))
        .route("/agent-sessions/list", post(list_sessions))
        .route("/agent-sessions/read-turns", post(read_turns))
}

#[derive(Debug, Deserialize)]
struct StartAgentSessionRequest {
    conversation_id: String,
    project_id: Option<String>,
    agent_id: String,
    host_installation_id: Option<String>,
    initial_user_message: Option<String>,
    client_request_id: String,
}

#[derive(Debug, Serialize)]
struct StartAgentSessionResponse {
    session_id: String,
    conversation_id: String,
    host_installation_id: String,
    started_at_ms: i64,
    initial_turn_id: Option<String>,
    host_command_id: String,
}

#[derive(Debug, Deserialize)]
struct SendInputRequest {
    session_id: String,
    text: String,
    mentions: Option<Vec<String>>,
    client_request_id: String,
}

#[derive(Debug, Serialize)]
struct SendInputResponse {
    session_id: String,
    turn_id: String,
    turn_seq: i64,
}

#[derive(Debug, Deserialize)]
struct StopAgentSessionRequest {
    session_id: String,
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
    sessions: Vec<AgentSessionSummaryResponse>,
    next_before_started_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct AgentSessionSummaryResponse {
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
    end_reason: Option<ThreadEndReason>,
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
    turns: Vec<crate::agent_sessions::ReadTurnMetadata>,
    events: Vec<crate::agent_sessions::ReadTurnEvent>,
    next_turn_seq: Option<i64>,
    next_event_seq: Option<i64>,
}

async fn read_turns(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<ReadTurnsRequest>,
) -> Result<Json<ReadTurnsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    let output = state
        .agent_sessions
        .read_turns(ReadTurnsInput {
            session_id: request.session_id,
            turn_id: request.turn_id,
            after_turn_seq: request.after_turn_seq,
            after_event_seq: request.after_event_seq,
            limit: request.limit,
            caller_account_id: account_id,
        })
        .await
        .map_err(map_agent_session_error)?;

    Ok(Json(ReadTurnsResponse {
        session_id: output.session_id,
        turn_id: output.turn_id,
        turns: output.turns,
        events: output.events,
        next_turn_seq: output.next_turn_seq,
        next_event_seq: output.next_event_seq,
    }))
}

async fn start_session(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<StartAgentSessionRequest>,
) -> Result<Json<StartAgentSessionResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    let output = state
        .agent_sessions
        .start(crate::agent_sessions::StartAgentSessionInput {
            conversation_id: request.conversation_id,
            project_id: request.project_id,
            agent_id: request.agent_id,
            host_installation_id: request.host_installation_id,
            initial_user_message: request.initial_user_message,
            client_request_id: request.client_request_id,
            caller_account_id: account_id,
        })
        .await
        .map_err(map_agent_session_error)?;

    Ok(Json(StartAgentSessionResponse {
        session_id: output.session_id,
        conversation_id: output.conversation_id,
        host_installation_id: output.host_installation_id,
        started_at_ms: output.started_at_ms,
        initial_turn_id: output.initial_turn_id,
        host_command_id: output.host_command_id,
    }))
}

async fn send_input(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<SendInputRequest>,
) -> Result<Json<SendInputResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    let output = state
        .agent_sessions
        .send_input(crate::agent_sessions::SendInputInput {
            session_id: request.session_id,
            text: request.text,
            mentions: request.mentions.unwrap_or_default(),
            client_request_id: request.client_request_id,
            caller_account_id: account_id,
        })
        .await
        .map_err(map_agent_session_error)?;

    Ok(Json(SendInputResponse {
        session_id: output.session_id,
        turn_id: output.turn_id,
        turn_seq: output.turn_seq,
    }))
}

async fn stop_session(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<StopAgentSessionRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    state
        .agent_sessions
        .stop(crate::agent_sessions::StopAgentSessionInput {
            session_id: request.session_id,
            caller_account_id: account_id,
        })
        .await
        .map_err(map_agent_session_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_sessions(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(request): Json<ListAgentSessionsRequest>,
) -> Result<Json<ListAgentSessionsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let (_caller, account_id) = super::require_authed_session(&state, &headers).await?;
    let output = state
        .agent_sessions
        .list(ListAgentSessionsInput {
            conversation_id: request.conversation_id,
            project_id: request.project_id,
            before_started_at_ms: request.before_started_at_ms,
            limit: request.limit,
            caller_account_id: account_id.clone(),
        })
        .await
        .map_err(map_agent_session_error)?;
    let session_ids = output
        .sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<Vec<_>>();
    let thread_summaries =
        crate::store::threads::summaries_for_ids(&state.store, &account_id, &session_ids)
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    err("internal", error.to_string()),
                )
            })?;

    Ok(Json(ListAgentSessionsResponse {
        sessions: output
            .sessions
            .into_iter()
            .map(|session| {
                let session_id = session.session_id.clone();
                agent_session_summary_response(session, thread_summaries.get(&session_id))
            })
            .collect(),
        next_before_started_at_ms: output.next_before_started_at_ms,
    }))
}

fn agent_session_summary_response(
    session: crate::agent_sessions::AgentSessionSummary,
    thread: Option<&minos_protocol::ThreadSummary>,
) -> AgentSessionSummaryResponse {
    let agent = thread.map(|summary| summary.agent).or_else(|| {
        session
            .agent_id
            .as_deref()
            .and_then(agent_name_from_agent_id)
    });
    let ended_at_ms = thread
        .and_then(|summary| summary.ended_at_ms)
        .or(session.ended_at_ms);
    let last_activity_at_ms = thread
        .map(|summary| summary.last_ts_ms)
        .unwrap_or_else(|| ended_at_ms.unwrap_or(session.started_at_ms));
    AgentSessionSummaryResponse {
        session_id: session.session_id,
        conversation_id: session.conversation_id,
        project_id: session.project_id,
        agent_id: session.agent_id,
        agent,
        status: session.status,
        started_at_ms: session.started_at_ms,
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
        "agent_opencode" | "opencode" => Some(AgentName::Opencode),
        _ => None,
    }
}

fn map_agent_session_error(error: AgentSessionError) -> (StatusCode, Json<ErrorEnvelope>) {
    match error {
        AgentSessionError::NotFound => (
            StatusCode::NOT_FOUND,
            err("agent_session_not_found", "agent session not found"),
        ),
        AgentSessionError::TurnNotFound => (
            StatusCode::NOT_FOUND,
            err("agent_turn_not_found", "agent turn not found"),
        ),
        AgentSessionError::ConversationForbidden => (
            StatusCode::FORBIDDEN,
            err("conversation_forbidden", "conversation access denied"),
        ),
        AgentSessionError::HostUnavailable => (
            StatusCode::CONFLICT,
            err(
                "agent_session_host_unavailable",
                "no live host is available for this session",
            ),
        ),
        AgentSessionError::StateInvalid => (
            StatusCode::CONFLICT,
            err(
                "agent_session_state_invalid",
                "agent session is not writable",
            ),
        ),
        AgentSessionError::ValidationMissing(field) => (
            StatusCode::BAD_REQUEST,
            err(
                "validation_missing_field",
                format!("missing field: {field}"),
            ),
        ),
        AgentSessionError::ValidationFormat(message) => {
            (StatusCode::BAD_REQUEST, err("validation_format", message))
        }
        AgentSessionError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            err("internal", error.to_string()),
        ),
    }
}
