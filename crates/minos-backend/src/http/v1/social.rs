use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use serde::Deserialize;

const GROUP_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const GROUP_COMPLETION_IDLE_LOG_INTERVAL: Duration = Duration::from_mins(5);
const GROUP_COMPLETION_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// After raw seq stops advancing (and tools are idle), treat final text as stable.
/// Slightly generous so Grok progress MessageCompleted → ToolCallPlaced is not
/// mis-projected as the turn answer.
const GROUP_COMPLETION_SEQ_STABLE: Duration = Duration::from_secs(2);
use crate::app::tx::Storage;
use crate::auth::bearer;
use crate::http::error_response::{err_response, ErrorBody, ErrorEnvelope};
use crate::http::BackendState;
use axum::{Json, Router};
use minos_domain::AgentName;
use minos_protocol::{
    AddAgentToGroupRequest, AgentSummary, ChatMessageSummary, ConversationAgentMembersResponse,
    DurableEvent, EnsureHostRuntimeAgentRequest, Envelope, EventKind, ListAgentsResponse,
    RealtimeTopic, RegisterAgentRequest, RemoveAgentFromGroupRequest, SendAgentMessageRequest,
    SenderRef, SenderType, UpdateAgentRequest,
};
use serde_json::json;
use uuid::Uuid;

/// Known Host runtime bins that Mobile/Desktop may @-mention for dispatch.
const HOST_RUNTIME_MENTIONS: &[&str] = &["codex", "claude", "gemini", "opencode", "grok"];

pub fn router() -> Router<BackendState> {
    Router::new()
        // ─── Agent routes ───
        .route("/agents", post(register_agent))
        .route("/agents/query", post(list_agents))
        .route(
            "/agents/ensure-host-runtime",
            post(ensure_host_runtime_agent),
        )
        .route("/agents/:agent_id/update", post(update_agent_handler))
        .route("/agents/:agent_id/delete", post(delete_agent_handler))
        .route(
            "/conversations/:conversation_id/agents",
            post(list_conversation_agents_handler),
        )
        .route(
            "/conversations/:conversation_id/agents/add",
            post(add_agent_to_group),
        )
        .route(
            "/conversations/:conversation_id/agents/remove",
            post(remove_agent_from_group),
        )
        .route(
            "/conversations/:conversation_id/agents/message",
            post(send_agent_message),
        )
}

pub fn external_sql_router() -> Router<BackendState> {
    router()
}

fn err(code: &'static str, message: impl Into<String>) -> (StatusCode, Json<ErrorEnvelope>) {
    err_response(code, message)
}

#[derive(Debug, Deserialize)]
pub struct SendConversationMessageRequest {
    pub conversation_id: String,
    pub text: String,
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub client_message_id: Option<String>,
    #[serde(default)]
    pub message_source: Option<minos_protocol::MessageSource>,
    #[serde(default)]
    pub client_sent_at_ms: Option<i64>,
    #[serde(default)]
    pub created_at_ms: Option<i64>,
}

#[allow(clippy::unused_async)]
async fn require_account_id(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<ErrorEnvelope>)> {
    require_account_id_from_state(state, headers)
}

/// Extract the authenticated account ID from the bearer token in the request headers.
/// Public so other handler modules (profiles, friends, conversations) can reuse this.
pub fn require_account_id_from_state(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer = bearer::require(state, headers).map_err(|e| {
        let (status, message) = e.into_response_tuple();
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: "unauthorized",
                    message,
                },
            }),
        )
    })?;
    Ok(bearer.account_id)
}

pub async fn fan_out_social_message(state: &BackendState, message: &ChatMessageSummary) {
    let members = match crate::store::social::list_conversation_members(
        &state.store,
        &message.conversation_id,
    )
    .await
    {
        Ok(members) => members,
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::social",
                conversation_id = %message.conversation_id,
                error = %error,
                "failed to list conversation members for social fan-out"
            );
            return;
        }
    };

    let frame = Envelope::Event {
        version: 1,
        event: EventKind::SocialMessage {
            conversation_id: message.conversation_id.clone(),
            message: message.clone(),
        },
    };
    state.realtime.fanout_social_message(&members, &frame).await;
    // conversation:{id} for open-timeline subscribers + account:* inbox summaries.
    if let Err(error) = fan_out_conversation_topic_event(state, message).await {
        tracing::warn!(
            target: "minos_backend::social",
            conversation_id = %message.conversation_id,
            message_id = %message.message_id,
            error = %error,
            "failed to publish conversation topic durable event"
        );
    }
    if let Err(error) = fan_out_account_conversation_event(state, &members, message).await {
        tracing::warn!(
            target: "minos_backend::social",
            conversation_id = %message.conversation_id,
            message_id = %message.message_id,
            error = %error,
            "failed to publish formal social message event"
        );
    }
}

/// Durable fanout on `conversation:{id}` (open chat timeline hot path).
async fn fan_out_conversation_topic_event(
    state: &BackendState,
    message: &ChatMessageSummary,
) -> Result<(), crate::error::BackendError> {
    let at_ms = message.recalled_at_ms.unwrap_or(message.created_at_ms);
    let event = if message.recalled_at_ms.is_some() {
        DurableEvent::ConversationMessageRecalled {
            conversation_id: message.conversation_id.clone(),
            message_id: message.message_id.clone(),
            at_ms,
            message: Some(message.clone()),
        }
    } else {
        DurableEvent::ConversationMessageAppended {
            conversation_id: message.conversation_id.clone(),
            message_id: message.message_id.clone(),
            sender: sender_ref_for_message(message),
            at_ms,
            message: Some(message.clone()),
        }
    };
    let action = if message.recalled_at_ms.is_some() {
        "recalled"
    } else {
        "appended"
    };
    let event_id = format!(
        "social-conv-{action}-{}-{}",
        message.conversation_id, message.message_id
    );
    let mut tx = Storage::begin(&state.store).await?;
    let cursor =
        crate::store::durable_event_log::record_in_tx(&mut tx, &event_id, &event, at_ms).await?;
    let outbox_id = Uuid::new_v4().to_string();
    crate::store::outbox_events::enqueue_in_tx(
        &mut tx,
        &outbox_id,
        cursor.topic.kind().as_str(),
        &cursor.event_id,
        at_ms,
    )
    .await?;
    tx.commit().await?;
    state
        .realtime
        .publish_durable_event_by_id(cursor.topic.kind().as_str(), &cursor.event_id)
        .await?;
    crate::store::outbox_events::ack(
        &state.store,
        &outbox_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    Ok(())
}

async fn fan_out_account_conversation_event(
    state: &BackendState,
    target_account_ids: &[String],
    message: &ChatMessageSummary,
) -> Result<(), crate::error::BackendError> {
    let at_ms = message.recalled_at_ms.unwrap_or(message.created_at_ms);
    let mut tx = Storage::begin(&state.store).await?;
    let mut pending_events = Vec::<(String, String, String)>::new();
    for account_id in target_account_ids {
        let event = if message.recalled_at_ms.is_some() {
            DurableEvent::AccountConversationMessageRecalled {
                account_id: account_id.clone(),
                conversation_id: message.conversation_id.clone(),
                message_id: message.message_id.clone(),
                at_ms,
                message: message.clone(),
            }
        } else {
            DurableEvent::AccountConversationMessageAppended {
                account_id: account_id.clone(),
                conversation_id: message.conversation_id.clone(),
                message_id: message.message_id.clone(),
                sender: sender_ref_for_message(message),
                at_ms,
                message: message.clone(),
            }
        };
        let event_id = account_conversation_event_id(account_id, message);
        let cursor =
            crate::store::durable_event_log::record_in_tx(&mut tx, &event_id, &event, at_ms)
                .await?;
        let outbox_id = Uuid::new_v4().to_string();
        crate::store::outbox_events::enqueue_in_tx(
            &mut tx,
            &outbox_id,
            cursor.topic.kind().as_str(),
            &cursor.event_id,
            at_ms,
        )
        .await?;
        pending_events.push((
            cursor.topic.kind().as_str().to_string(),
            cursor.event_id,
            outbox_id,
        ));
    }
    tx.commit().await?;
    for (topic_kind, event_id, outbox_id) in pending_events {
        state
            .realtime
            .publish_durable_event_by_id(&topic_kind, &event_id)
            .await?;
        crate::store::outbox_events::ack(
            &state.store,
            &outbox_id,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
    }
    Ok(())
}

fn sender_ref_for_message(message: &ChatMessageSummary) -> SenderRef {
    match message.sender_type {
        SenderType::Agent => SenderRef::Agent {
            agent_id: message.sender.account_id.clone(),
            session_id: None,
        },
        SenderType::User => SenderRef::User {
            account_id: message.sender.account_id.clone(),
        },
    }
}

fn account_conversation_event_id(account_id: &str, message: &ChatMessageSummary) -> String {
    let action = if message.recalled_at_ms.is_some() {
        "recalled"
    } else {
        "appended"
    };
    format!("social-{action}-{account_id}-{}", message.message_id)
}

#[derive(Clone)]
struct AgentDispatchPlan {
    agent: crate::store::social::AgentRow,
    session_id: Option<String>,
    forwarded_text: String,
    mention_sender: bool,
}

struct ForwardedAgentDispatch {
    session_id: String,
    watcher_from_seq: u64,
}

async fn build_agent_dispatch_plan(
    state: &BackendState,
    conversation_id: &str,
    text: &str,
    reply_target: Option<&crate::store::social::ChatMessageRow>,
) -> Result<Option<AgentDispatchPlan>, (StatusCode, Json<ErrorEnvelope>)> {
    let conversation = crate::store::social::get_conversation(&state.store, conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    let agents = crate::store::social::list_conversation_agents(&state.store, conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    if agents.is_empty() {
        return Ok(None);
    }
    let human_members =
        crate::store::social::list_conversation_members(&state.store, conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let mention_sender = human_members.len() > 1;

    if let Some(reply_target) = reply_target {
        if reply_target.sender_type == "agent" {
            if let Some(session_id) = crate::store::social::lookup_session_id_for_message(
                &state.store,
                &reply_target.message_id,
            )
            .await
            .map_err(|e| err("internal", e.to_string()))?
            {
                let agent_id = reply_target
                    .sender_agent_id
                    .as_deref()
                    .unwrap_or(&reply_target.sender_account_id);
                if let Some(agent) = crate::store::social::get_agent(&state.store, agent_id)
                    .await
                    .map_err(|e| err("internal", e.to_string()))?
                {
                    return Ok(Some(AgentDispatchPlan {
                        agent,
                        session_id: Some(session_id),
                        forwarded_text: text.to_string(),
                        mention_sender,
                    }));
                }
            }
        }
    }

    if conversation.kind == "group" && human_members.len() == 1 && agents.len() == 1 {
        let agent = agents[0].clone();
        // Bare text auto-routes; `@agent` / `@agent#short` still honored for reuse.
        let (session_short, forwarded_text) =
            if let Some(route) = first_mentioned_agent_route(text, &agents) {
                if route.agent.agent_id == agent.agent_id {
                    (route.session_short_id, route.forwarded_text)
                } else {
                    (None, text.to_string())
                }
            } else {
                (None, text.to_string())
            };
        let session_id =
            resolve_dispatch_session_id(state, conversation_id, &agent, session_short.as_deref())
                .await
                .map_err(|e| err("internal", e.to_string()))?;
        return Ok(Some(AgentDispatchPlan {
            agent,
            session_id,
            forwarded_text,
            mention_sender: false,
        }));
    }

    // Explicit @agent / @agent#short mention (group or agent DM-style rooms).
    if let Some(route) = first_mentioned_agent_route(text, &agents) {
        let session_id = resolve_dispatch_session_id(
            state,
            conversation_id,
            &route.agent,
            route.session_short_id.as_deref(),
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?;
        return Ok(Some(AgentDispatchPlan {
            agent: route.agent.clone(),
            session_id,
            forwarded_text: route.forwarded_text,
            mention_sender: true,
        }));
    }

    Ok(None)
}

/// Resolve which formal agent session should receive a Hub `@agent` dispatch.
///
/// Order (parity with Desktop workbench reuse):
/// 1. Explicit `@agent#short` → formal session short-id match
/// 2. Latest chat_messages bind for this agent (prior Hub dispatch / projector)
/// 3. Latest reusable formal `agent_sessions` row (Desktop-started via Host ingest)
async fn resolve_dispatch_session_id(
    state: &BackendState,
    conversation_id: &str,
    agent: &crate::store::social::AgentRow,
    session_short_id: Option<&str>,
) -> Result<Option<String>, crate::error::BackendError> {
    if let Some(short) = session_short_id.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(session_id) = crate::store::agent_sessions::find_reusable_by_short_id(
            &state.store,
            conversation_id,
            &agent.agent_id,
            &agent.runtime_agent,
            short,
        )
        .await?
        {
            return Ok(Some(session_id));
        }
        // No formal match: fall through to chat bind / latest reuse rather than
        // hard-fail — Host may still accept send_input if bind points at short.
    }

    if let Some(session_id) = crate::store::social::lookup_latest_session_id_for_conversation_agent(
        &state.store,
        conversation_id,
        &agent.agent_id,
    )
    .await?
    {
        return Ok(Some(session_id));
    }

    // Desktop-started sessions are registered by Host ingest into agent_sessions
    // without necessarily writing chat_messages.agent_session_id yet.
    crate::store::agent_sessions::latest_reusable_for_conversation_agent(
        &state.store,
        conversation_id,
        &agent.agent_id,
        &agent.runtime_agent,
    )
    .await
}

async fn forward_agent_dispatch(
    state: &BackendState,
    account_id: &str,
    agent: &crate::store::social::AgentRow,
    session_id: Option<String>,
    text: &str,
    conversation_id: &str,
    origin_message_id: &str,
) -> Result<ForwardedAgentDispatch, crate::error::BackendError> {
    if let Some(session_id) = session_id {
        let watcher_from_seq =
            crate::store::raw_events::last_seq(&state.store, &session_id).await?;
        state
            .agent_sessions
            .send_input(crate::agent_sessions::SendInputInput {
                session_id: session_id.clone(),
                text: text.to_string(),
                mentions: Vec::new(),
                client_request_id: format!("social-send-{origin_message_id}"),
                caller_account_id: account_id.to_string(),
            })
            .await
            .map_err(|error| map_agent_session_dispatch_error("agent_session.send_input", error))?;
        return Ok(ForwardedAgentDispatch {
            session_id,
            watcher_from_seq,
        });
    }

    let host_device_id = select_live_host_for_account(state, &agent.owner_account_id).await?;
    let project_id =
        resolve_project_id_for_agent_dispatch(state, account_id, conversation_id, agent).await?;
    let conversation_title = crate::store::social::get_conversation(&state.store, conversation_id)
        .await?
        .and_then(|c| c.title);
    let output = state
        .agent_sessions
        .start(crate::agent_sessions::StartAgentSessionInput {
            conversation_id: conversation_id.to_string(),
            project_id,
            agent_id: agent.agent_id.clone(),
            host_installation_id: Some(host_device_id.to_string()),
            workspace_path: agent.workspace_path.clone(),
            initial_user_message: Some(text.to_string()),
            client_request_id: format!("social-start-{origin_message_id}"),
            caller_account_id: account_id.to_string(),
            conversation_title,
        })
        .await
        .map_err(|error| map_agent_session_dispatch_error("agent_session.start", error))?;
    Ok(ForwardedAgentDispatch {
        session_id: output.session_id,
        watcher_from_seq: 0,
    })
}

/// Resolve a host/Desktop project id for agent start so Host does not invent
/// "Direct agent sessions". Prefer prior session scope, then workspace match.
async fn resolve_project_id_for_agent_dispatch(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    agent: &crate::store::social::AgentRow,
) -> Result<Option<String>, crate::error::BackendError> {
    if let Some(session) = crate::store::agent_sessions::latest_for_account_conversation(
        &state.store,
        conversation_id,
        account_id,
    )
    .await?
    {
        if let Some(project_id) = session
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(Some(project_id.to_string()));
        }
    }

    let Some(workspace) = agent
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };

    let projects = crate::store::projects::list(&state.store, account_id).await?;
    if let Some(project) = projects.into_iter().find(|p| {
        p.workspace_path
            .as_deref()
            .map(str::trim)
            .is_some_and(|path| path == workspace)
    }) {
        return Ok(Some(project.project_id));
    }

    Ok(None)
}

fn map_agent_session_dispatch_error(
    method: &'static str,
    error: crate::agent_sessions::AgentSessionError,
) -> crate::error::BackendError {
    match error {
        crate::agent_sessions::AgentSessionError::Internal(error) => error,
        other => crate::error::BackendError::ForwardRpc {
            method: method.into(),
            message: other.to_string(),
        },
    }
}

async fn select_live_host_for_account(
    state: &BackendState,
    account_id: &str,
) -> Result<minos_domain::DeviceId, crate::error::BackendError> {
    let hosts = crate::store::host_links::list_hosts_for_account(&state.store, account_id).await?;
    for host in hosts {
        if state.registry.get(host.host_device_id).is_some() {
            return Ok(host.host_device_id);
        }
    }
    Err(crate::error::BackendError::ForwardRpc {
        method: "agent_session.start".into(),
        // Stable code substring used by agent_error_from_backend_error / UX copy.
        message: format!("no live host paired to account {account_id}"),
    })
}

fn agent_error_code_for_message(message: &str) -> Option<&'static str> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("no live host") {
        Some("no_live_host")
    } else {
        None
    }
}

struct MentionedAgentRoute {
    agent: crate::store::social::AgentRow,
    session_short_id: Option<String>,
    forwarded_text: String,
}

fn first_mentioned_agent(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Option<crate::store::social::AgentRow> {
    first_mentioned_agent_route(text, agents).map(|r| r.agent)
}

/// First `@agent` / `@agent#short` route for dispatch (Desktop `parseAgentRouting` parity).
fn first_mentioned_agent_route(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Option<MentionedAgentRoute> {
    // Prefer full-token parse so `@codex#deadbeef prompt` keeps the short id
    // (collect_mention_tokens stops at `#`).
    if let Some(route) = parse_leading_agent_route(text, agents) {
        return Some(route);
    }
    // Fall back: any mid-body @token (legacy multi-mention / name forms).
    collect_mention_tokens(text).into_iter().find_map(|token| {
        let t = token.trim();
        if t.is_empty() {
            return None;
        }
        let (name_part, short) = split_agent_session_token(t);
        let agent = match_agent_token(name_part, agents)?;
        Some(MentionedAgentRoute {
            agent: agent.clone(),
            session_short_id: short.map(str::to_string),
            forwarded_text: strip_agent_mention_once(text, &agent),
        })
    })
}

fn parse_leading_agent_route(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Option<MentionedAgentRoute> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('@') {
        return None;
    }
    let rest = &trimmed[1..];
    let split_at = rest
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(i, _)| i);
    let token = split_at.map_or(rest, |i| &rest[..i]);
    let body = split_at.map_or("", |i| rest[i..].trim_start());
    if token.is_empty() {
        return None;
    }
    let (name_part, short) = split_agent_session_token(token);
    let agent = match_agent_token(name_part, agents)?;
    let forwarded = if body.is_empty() {
        // Keep original when mention is bare so Host still has content if needed.
        text.to_string()
    } else {
        body.to_string()
    };
    Some(MentionedAgentRoute {
        agent: agent.clone(),
        session_short_id: short.map(str::to_string),
        forwarded_text: forwarded,
    })
}

fn split_agent_session_token(token: &str) -> (&str, Option<&str>) {
    match token.split_once('#') {
        Some((name, short)) if !name.is_empty() && !short.is_empty() => (name, Some(short)),
        _ => (token, None),
    }
}

fn match_agent_token<'a>(
    token: &str,
    agents: &'a [crate::store::social::AgentRow],
) -> Option<&'a crate::store::social::AgentRow> {
    let t = token.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    // Match cloud agent_id, runtime bin (codex/grok/…), or display name.
    // Desktop dual-writes host-runtime agents; Mobile users type @grok not @bot-uuid.
    agents.iter().find(|agent| {
        agent.agent_id.eq_ignore_ascii_case(t)
            || agent.runtime_agent.eq_ignore_ascii_case(&lower)
            || agent.name.eq_ignore_ascii_case(t)
    })
}

fn strip_agent_mention_once(text: &str, agent: &crate::store::social::AgentRow) -> String {
    let candidates = [
        agent.agent_id.as_str(),
        agent.runtime_agent.as_str(),
        agent.name.as_str(),
    ];
    let mut stripped = text.to_string();
    for token in candidates {
        let needle = format!("@{token}");
        if stripped
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
        {
            // Case-insensitive single replace of @token.
            if let Some(idx) = stripped
                .to_ascii_lowercase()
                .find(&needle.to_ascii_lowercase())
            {
                stripped = format!("{}{}", &stripped[..idx], &stripped[idx + needle.len()..]);
                break;
            }
        }
    }
    let normalised = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalised.is_empty() {
        text.to_string()
    } else {
        normalised
    }
}

fn normalize_workspace_path(path: Option<&str>) -> Option<String> {
    path.map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
}

fn is_valid_workspace_path(path: &str) -> bool {
    path.starts_with('/') || path.starts_with("~/")
}

fn agent_error_from_backend_error(error: &crate::error::BackendError) -> (&'static str, String) {
    match error {
        crate::error::BackendError::PeerOffline { .. } => {
            ("peer_offline", "agent host is offline".to_string())
        }
        crate::error::BackendError::PeerBackpressure { .. } => {
            ("peer_backpressure", "agent host is busy".to_string())
        }
        crate::error::BackendError::ForwardRpcTimeout { .. } => (
            "dispatch_timeout",
            "agent host did not reply in time".to_string(),
        ),
        crate::error::BackendError::ForwardRpc { message, .. } => {
            if let Some(code) = agent_error_code_for_message(message) {
                (code, message.clone())
            } else {
                ("dispatch_failed", message.clone())
            }
        }
        other => ("dispatch_failed", other.to_string()),
    }
}

fn fan_out_agent_error(
    state: &BackendState,
    account_id: &str,
    session_id: Option<String>,
    code: &str,
    message: String,
) {
    let frame = Envelope::Event {
        version: 1,
        event: EventKind::AgentError {
            session_id,
            code: code.to_string(),
            message,
        },
    };
    let _ = state.registry.broadcast_mobile_account(account_id, frame);
}

#[allow(clippy::too_many_arguments)]
fn spawn_group_completion_watcher(
    state: BackendState,
    conversation_id: String,
    reply_to_message_id: String,
    session_id: String,
    agent: crate::store::social::AgentRow,
    trigger_seq: u64,
    mention_account_id: Option<String>,
    mention_minos_id: Option<String>,
) {
    tokio::spawn(async move {
        let mut cursor = CompletionWatchCursor::new(trigger_seq, tokio::time::Instant::now());
        loop {
            let now = tokio::time::Instant::now();
            match crate::store::raw_events::last_seq(&state.store, &session_id).await {
                Ok(latest_seq) => {
                    cursor.observe(latest_seq, now);
                }
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::social",
                        error = %error,
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        "group completion watcher failed to inspect latest raw event seq"
                    );
                }
            }

            let session_terminal =
                match crate::store::agent_sessions::get(&state.store, &session_id).await {
                    Ok(Some(session)) => {
                        session.ended_at_ms.is_some()
                            || crate::turn_completion::is_session_terminal_status(&session.status)
                    }
                    Ok(None) => false,
                    Err(error) => {
                        tracing::warn!(
                            target: "minos_backend::social",
                            error = %error,
                            session_id = %session_id,
                            "group completion watcher failed to load agent session"
                        );
                        false
                    }
                };

            let seq_stable = now.saturating_duration_since(cursor.last_activity_at)
                >= GROUP_COMPLETION_SEQ_STABLE;

            match find_completed_agent_reply(
                &state.store,
                &session_id,
                agent_name_for_row(&agent),
                trigger_seq,
                session_terminal,
                seq_stable,
            )
            .await
            {
                Ok(crate::turn_completion::CompletionProbe::Ready(text)) => {
                    let final_text = mention_minos_id
                        .as_deref()
                        .map_or(text.clone(), |minos_id| format!("@{minos_id} {text}"));
                    let mentions = mention_account_id.iter().cloned().collect::<Vec<_>>();
                    // TurnCompletionProjector is the sole multi-end agent bubble writer.
                    let client_message_id =
                        crate::turn_completion::TurnCompletionProjector::agent_result_client_message_id(
                            &conversation_id,
                            &session_id,
                            trigger_seq,
                        );
                    match post_agent_social_message(
                        &state,
                        &conversation_id,
                        &agent,
                        Some(session_id.as_str()),
                        &reply_to_message_id,
                        &final_text,
                        &mentions,
                        Some(client_message_id.as_str()),
                    )
                    .await
                    {
                        Ok(()) => {
                            tracing::info!(
                                target: "minos_backend::social",
                                conversation_id = %conversation_id,
                                session_id = %session_id,
                                client_message_id = %client_message_id,
                                "TurnCompletionProjector posted agent bubble"
                            );
                            return;
                        }
                        Err((_, body)) => {
                            // Transient store/fanout failure must not abandon the turn —
                            // keep polling (idempotent client_message_id on retry).
                            // Cap via should_stop_after_long_idle (~5m without raw activity).
                            tracing::warn!(
                                target: "minos_backend::social",
                                conversation_id = %conversation_id,
                                session_id = %session_id,
                                error = %body.0.error.message,
                                "TurnCompletionProjector failed to post agent bubble; will retry"
                            );
                            tokio::time::sleep(GROUP_COMPLETION_IDLE_POLL_INTERVAL).await;
                            continue;
                        }
                    }
                }
                Ok(crate::turn_completion::CompletionProbe::DoneWithoutText) => {
                    tracing::info!(
                        target: "minos_backend::social",
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        trigger_seq,
                        "TurnCompletionProjector finished without clean final text"
                    );
                    return;
                }
                Ok(crate::turn_completion::CompletionProbe::Pending) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::social",
                        error = %error,
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        "TurnCompletionProjector failed to translate session state"
                    );
                }
            }

            // Bound infinite idle poll when Grok/etc. never yield final text and
            // session never flips terminal (host offline mid-turn).
            if cursor.should_stop_after_long_idle(now) {
                tracing::warn!(
                    target: "minos_backend::social",
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    trigger_seq,
                    last_observed_seq = cursor.last_observed_seq,
                    "group completion watcher giving up after prolonged agent inactivity"
                );
                return;
            }

            if cursor.should_log_idle(now) {
                tracing::warn!(
                    target: "minos_backend::social",
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    trigger_seq,
                    last_observed_seq = cursor.last_observed_seq,
                    "group completion watcher still waiting after agent inactivity"
                );
            }

            tokio::time::sleep(cursor.next_poll_delay(now)).await;
        }
    });
}

#[derive(Debug, Clone, Copy)]
struct CompletionWatchCursor {
    last_observed_seq: u64,
    last_activity_at: tokio::time::Instant,
    next_idle_log_at: tokio::time::Instant,
}

impl CompletionWatchCursor {
    fn new(trigger_seq: u64, now: tokio::time::Instant) -> Self {
        Self {
            last_observed_seq: trigger_seq,
            last_activity_at: now,
            next_idle_log_at: now + GROUP_COMPLETION_IDLE_LOG_INTERVAL,
        }
    }

    fn observe(&mut self, latest_seq: u64, now: tokio::time::Instant) {
        if latest_seq <= self.last_observed_seq {
            return;
        }
        self.last_observed_seq = latest_seq;
        self.last_activity_at = now;
        self.next_idle_log_at = now + GROUP_COMPLETION_IDLE_LOG_INTERVAL;
    }

    fn should_log_idle(&mut self, now: tokio::time::Instant) -> bool {
        if now < self.next_idle_log_at {
            return false;
        }
        self.next_idle_log_at = now + GROUP_COMPLETION_IDLE_LOG_INTERVAL;
        true
    }

    /// Stop watching after one full idle-log window with no new raw events.
    fn should_stop_after_long_idle(self, now: tokio::time::Instant) -> bool {
        now.saturating_duration_since(self.last_activity_at) >= GROUP_COMPLETION_IDLE_LOG_INTERVAL
    }

    fn next_poll_delay(self, now: tokio::time::Instant) -> Duration {
        if now.saturating_duration_since(self.last_activity_at)
            >= GROUP_COMPLETION_IDLE_LOG_INTERVAL
        {
            return GROUP_COMPLETION_IDLE_POLL_INTERVAL;
        }
        GROUP_COMPLETION_POLL_INTERVAL
    }
}

/// Probe session raw events via [`TurnCompletionProjector`] (single multi-end writer).
async fn find_completed_agent_reply(
    store: &impl crate::store::AsStorePool,
    session_id: &str,
    agent_name: AgentName,
    trigger_seq: u64,
    session_terminal: bool,
    seq_stable: bool,
) -> Result<crate::turn_completion::CompletionProbe, crate::error::BackendError> {
    let rows = crate::store::raw_events::read_range(store, session_id, 1, 10_000).await?;
    let row_refs: Vec<(u64, &serde_json::Value)> = rows
        .iter()
        .map(|row| (u64::try_from(row.seq).unwrap_or_default(), &row.payload))
        .collect();
    crate::turn_completion::TurnCompletionProjector::probe(
        agent_name,
        session_id,
        &row_refs,
        trigger_seq,
        session_terminal,
        seq_stable,
    )
    .map_err(|message| crate::error::BackendError::ForwardRpc {
        method: "turn_completion_projector".into(),
        message,
    })
}

fn agent_name_for_row(agent: &crate::store::social::AgentRow) -> AgentName {
    match agent.runtime_agent.as_str() {
        "claude" => AgentName::Claude,
        "gemini" => AgentName::Gemini,
        "grok" => AgentName::Grok,
        "opencode" => AgentName::Opencode,
        _ => AgentName::Codex,
    }
}

#[allow(clippy::too_many_arguments)]
async fn post_agent_social_message(
    state: &BackendState,
    conversation_id: &str,
    agent: &crate::store::social::AgentRow,
    session_id: Option<&str>,
    reply_to_message_id: &str,
    text: &str,
    mentioned_account_ids: &[String],
    // Stable idempotency key from TurnCompletionProjector (agent-result:…).
    client_message_id: Option<&str>,
) -> Result<(), (StatusCode, Json<ErrorEnvelope>)> {
    let session_id = session_id.map(str::trim).filter(|s| !s.is_empty());
    let row = crate::store::social::insert_agent_message_with_session(
        &state.store,
        conversation_id,
        &agent.agent_id,
        text,
        chrono::Utc::now().timestamp_millis(),
        Some(reply_to_message_id),
        session_id,
        mentioned_account_ids,
        client_message_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    let mut hydrated = crate::conversations::use_case::hydrate_messages(&state.store, vec![row])
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let message = hydrated.remove(0);
    fan_out_social_message(state, &message).await;
    Ok(())
}

fn extract_mentioned_account_ids(
    text: &str,
    sender_account_id: &str,
    members: &[crate::store::social::ProfileRow],
) -> Vec<String> {
    let by_minos_id = members
        .iter()
        .map(|member| (member.minos_id.as_str(), member.account_id.as_str()))
        .collect::<HashMap<_, _>>();
    let mut mentions = BTreeSet::<String>::new();

    for token in collect_mention_tokens(text) {
        let Some(account_id) = by_minos_id.get(token) else {
            continue;
        };
        if *account_id == sender_account_id {
            continue;
        }
        mentions.insert((*account_id).to_string());
    }

    mentions.into_iter().collect()
}

fn collect_mention_tokens(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        if index > 0 && bytes[index - 1].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }

        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-') {
            end += 1;
        }
        if end > start {
            tokens.push(&text[start..end]);
            index = end;
            continue;
        }
        index += 1;
    }

    tokens
}

// ─── Agent Handlers ────────────────────────────────────────────────────

async fn register_agent(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RegisterAgentRequest>,
) -> Result<Json<AgentSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(err("bad_request", "agent name is required"));
    }
    let valid_runtimes = ["codex", "claude", "gemini", "opencode", "grok"];
    if !valid_runtimes.contains(&req.runtime_agent.as_str()) {
        return Err(err("bad_request", "invalid runtime_agent"));
    }
    let workspace_path = normalize_workspace_path(req.workspace_path.as_deref());
    if let Some(path) = workspace_path.as_deref() {
        if !is_valid_workspace_path(path) {
            return Err(err(
                "bad_request",
                "workspace_path must be an absolute host path or ~/ path",
            ));
        }
    }
    let row = crate::store::social::register_agent(
        &state.store,
        &account_id,
        name,
        req.description.trim(),
        &req.runtime_agent,
        req.model.trim(),
        workspace_path.as_deref(),
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(Json(agent_row_to_summary(&row)))
}

async fn ensure_host_runtime_agent(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<EnsureHostRuntimeAgentRequest>,
) -> Result<Json<AgentSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let runtime = req.runtime_agent.trim().to_ascii_lowercase();
    let valid_runtimes = ["codex", "claude", "gemini", "opencode", "grok"];
    if !valid_runtimes.contains(&runtime.as_str()) {
        return Err(err("bad_request", "invalid runtime_agent"));
    }
    let display_name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let mut chars = runtime.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => runtime.clone(),
            }
        });
    let workspace_path = normalize_workspace_path(req.workspace_path.as_deref());
    if let Some(path) = workspace_path.as_deref() {
        if !is_valid_workspace_path(path) {
            return Err(err(
                "bad_request",
                "workspace_path must be an absolute host path or ~/ path",
            ));
        }
    }
    let row = crate::store::social::ensure_host_runtime_agent(
        &state.store,
        &account_id,
        &runtime,
        &display_name,
        req.model.trim(),
        workspace_path.as_deref(),
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(Json(agent_row_to_summary(&row)))
}

async fn list_agents(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ListAgentsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let rows = crate::store::social::list_agents_for_owner(&state.store, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let agents = rows.iter().map(agent_row_to_summary).collect();
    Ok(Json(ListAgentsResponse { agents }))
}

async fn update_agent_handler(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<AgentSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(err("bad_request", "agent name is required"));
    }
    let valid_runtimes = ["codex", "claude", "gemini", "opencode", "grok"];
    if !valid_runtimes.contains(&req.runtime_agent.as_str()) {
        return Err(err("bad_request", "invalid runtime_agent"));
    }
    let workspace_path = normalize_workspace_path(req.workspace_path.as_deref());
    if let Some(path) = workspace_path.as_deref() {
        if !is_valid_workspace_path(path) {
            return Err(err(
                "bad_request",
                "workspace_path must be an absolute host path or ~/ path",
            ));
        }
    }
    let row = crate::store::social::update_agent(
        &state.store,
        &agent_id,
        &account_id,
        name,
        req.description.trim(),
        &req.runtime_agent,
        req.model.trim(),
        workspace_path.as_deref(),
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?
    .ok_or_else(|| err("not_found", "agent not found or not owned by you"))?;
    Ok(Json(agent_row_to_summary(&row)))
}

async fn delete_agent_handler(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let deleted = crate::store::social::delete_agent(&state.store, &agent_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    if !deleted {
        return Err(err("not_found", "agent not found or not owned by you"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_conversation_agents_handler(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationAgentMembersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    let rows = crate::store::social::list_conversation_agents(&state.store, &conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let agents = rows.iter().map(agent_row_to_summary).collect();
    Ok(Json(ConversationAgentMembersResponse { agents }))
}

async fn add_agent_to_group(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<AddAgentToGroupRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    // Verify caller is a member
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    // Verify conversation is a group
    let conversation = crate::store::social::get_conversation(&state.store, &conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    if conversation.kind != "group" {
        return Err(err(
            "bad_request",
            "can only add agents to group conversations",
        ));
    }
    // Verify the agent exists
    let _agent = crate::store::social::get_agent(&state.store, &req.agent_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "agent not found"))?;
    crate::store::social::add_agent_to_conversation(
        &state.store,
        &conversation_id,
        &req.agent_id,
        &account_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_agent_from_group(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<RemoveAgentFromGroupRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    // Verify caller is a member
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    let removed = crate::store::social::remove_agent_from_conversation(
        &state.store,
        &conversation_id,
        &req.agent_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    if !removed {
        return Err(err("not_found", "agent not in this conversation"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn send_agent_message(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<SendAgentMessageRequest>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let trimmed = req.text.trim().to_string();
    if trimmed.is_empty() {
        return Err(err("bad_request", "message text is required"));
    }
    // Verify caller is a member
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    // Verify the agent exists and is owned by the caller
    let agent = crate::store::social::get_agent(&state.store, &req.agent_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "agent not found"))?;
    if agent.owner_account_id != account_id {
        return Err(err("forbidden", "you do not own this agent"));
    }
    // Auto-attach owned agents for Desktop dual-write races (roster may lag).
    let now_ms = chrono::Utc::now().timestamp_millis();
    if !crate::store::social::is_agent_in_conversation(
        &state.store,
        &conversation_id,
        &req.agent_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?
    {
        crate::store::social::add_agent_to_conversation(
            &state.store,
            &conversation_id,
            &req.agent_id,
            &account_id,
            now_ms,
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    }
    // Extract mentions from the message
    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let mentioned_account_ids = extract_mentioned_account_ids(&trimmed, &req.agent_id, &members);
    // Agent bubble insert is never a live client dispatch surface. Default and
    // force host_projection semantics: soft-drop invalid reply_to, never re-dispatch.
    // Preferred multi-end writer is TurnCompletionProjector; this endpoint remains
    // for Host-trusted host_projection inserts (optional Outbox uplink).
    let message_source = req
        .message_source
        .unwrap_or(minos_protocol::MessageSource::HostProjection);
    if message_source.allows_agent_dispatch() {
        // Explicit client_live on agents/message is rejected: agents never @-dispatch.
        return Err(err(
            "bad_request",
            "agents/message does not accept message_source=client_live; use host_projection",
        ));
    }
    let reply_to = match req.reply_to_message_id.as_deref() {
        Some(id) => match crate::store::social::get_message(&state.store, id)
            .await
            .map_err(|e| err("internal", e.to_string()))?
        {
            Some(row) if row.conversation_id == conversation_id => Some(id.to_string()),
            _ => {
                tracing::warn!(
                    target: "minos_backend::social",
                    conversation_id = %conversation_id,
                    reply_to_message_id = %id,
                    ?message_source,
                    "dropping invalid reply_to for agent message projection"
                );
                None
            }
        },
        None => None,
    };
    // Server clock is authoritative; ignore client_sent_at_ms / created_at_ms for ordering.
    let _ = req.client_sent_at_ms.or(req.created_at_ms);
    let row = crate::store::social::insert_agent_message_with_session(
        &state.store,
        &conversation_id,
        &req.agent_id,
        &trimmed,
        now_ms,
        reply_to.as_deref(),
        req.agent_session_id.as_deref(),
        &mentioned_account_ids,
        req.client_message_id.as_deref(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    // Full hydrate so reply_to / agent sender match list_messages (fanout + clients).
    let mut hydrated = crate::conversations::use_case::hydrate_messages(&state.store, vec![row])
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let message = hydrated.remove(0);
    // Prefer freshly extracted mentions when hydrate left them empty.
    let message = if message.mentioned_account_ids.is_empty() && !mentioned_account_ids.is_empty() {
        let mut m = message;
        m.mentioned_account_ids = mentioned_account_ids;
        m
    } else {
        message
    };
    fan_out_social_message(&state, &message).await;
    Ok(Json(message))
}

/// Try to dispatch a message to an agent in the conversation (Mobile / client_live).
///
/// Called after the user bubble is already durable. Failures must **not** be silent:
/// they surface as (1) formal StreamEvent `agent_error`, (2) legacy Envelope AgentError,
/// and (3) when an agent is known, a visible agent chat bubble in the timeline.
pub async fn try_agent_dispatch(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    message: &ChatMessageSummary,
    reply_to_message_id: Option<&str>,
    trimmed_text: &str,
) -> Result<(), crate::error::BackendError> {
    // Auto-attach @codex/@grok/… host-runtime agents so Mobile mentions work
    // even when Desktop never upserted the roster for this conversation.
    ensure_host_runtime_agents_for_mentions(state, account_id, conversation_id, trimmed_text)
        .await?;

    let reply_target = match reply_to_message_id {
        Some(message_id) => crate::store::social::get_message(&state.store, message_id).await?,
        None => None,
    };
    let dispatch_plan =
        build_agent_dispatch_plan(state, conversation_id, trimmed_text, reply_target.as_ref())
            .await
            .map_err(|(_, body)| crate::error::BackendError::StoreQuery {
                operation: "social::try_agent_dispatch.plan".into(),
                message: body.0.error.message,
            })?;

    let Some(plan) = dispatch_plan else {
        if let Some((code, detail)) =
            unmatched_agent_intent_error(state, conversation_id, trimmed_text).await?
        {
            tracing::warn!(
                target: "minos_backend::social",
                conversation_id = %conversation_id,
                account_id = %account_id,
                code = %code,
                detail = %detail,
                "agent dispatch skipped with user-visible intent"
            );
            notify_agent_dispatch_failure(
                state,
                account_id,
                conversation_id,
                &message.message_id,
                None,
                None,
                code,
                detail,
            )
            .await;
        }
        return Ok(());
    };

    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, conversation_id)
            .await?;
    let sender_minos_id = members
        .iter()
        .find(|m| m.account_id == account_id)
        .map(|m| m.minos_id.clone())
        .unwrap_or_default();
    let mention_sender = plan.mention_sender;
    let agent_for_error = plan.agent.clone();
    let session_hint = plan.session_id.clone();

    match forward_agent_dispatch(
        state,
        account_id,
        &plan.agent,
        plan.session_id.clone(),
        &plan.forwarded_text,
        conversation_id,
        &message.message_id,
    )
    .await
    {
        Ok(dispatch) => {
            tracing::info!(
                target: "minos_backend::social",
                conversation_id = %conversation_id,
                session_id = %dispatch.session_id,
                agent_id = %plan.agent.agent_id,
                runtime = %plan.agent.runtime_agent,
                "agent dispatch started"
            );
            crate::store::social::bind_session_to_message_for_agent(
                &state.store,
                &message.message_id,
                &plan.agent.agent_id,
                &dispatch.session_id,
            )
            .await?;

            spawn_group_completion_watcher(
                state.clone(),
                conversation_id.to_string(),
                message.message_id.clone(),
                dispatch.session_id,
                plan.agent,
                dispatch.watcher_from_seq,
                if mention_sender {
                    Some(account_id.to_string())
                } else {
                    None
                },
                if mention_sender {
                    Some(sender_minos_id)
                } else {
                    None
                },
            );
        }
        Err(error) => {
            let (code, detail) = agent_error_from_backend_error(&error);
            tracing::warn!(
                target: "minos_backend::social",
                conversation_id = %conversation_id,
                agent_id = %agent_for_error.agent_id,
                code = %code,
                detail = %detail,
                "agent dispatch failed after user message"
            );
            notify_agent_dispatch_failure(
                state,
                account_id,
                conversation_id,
                &message.message_id,
                Some(&agent_for_error),
                session_hint.as_deref(),
                code,
                detail,
            )
            .await;
        }
    }

    Ok(())
}

/// Attach host-runtime agents for `@codex` / `@grok` / … tokens missing from roster.
async fn ensure_host_runtime_agents_for_mentions(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    text: &str,
) -> Result<(), crate::error::BackendError> {
    let mut runtimes: Vec<String> = collect_mention_tokens(text)
        .into_iter()
        .map(str::to_ascii_lowercase)
        .filter(|token| HOST_RUNTIME_MENTIONS.contains(&token.as_str()))
        .collect();
    runtimes.sort();
    runtimes.dedup();
    if runtimes.is_empty() {
        return Ok(());
    }

    let existing =
        crate::store::social::list_conversation_agents(&state.store, conversation_id).await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    for runtime in runtimes {
        if existing
            .iter()
            .any(|agent| agent.runtime_agent.eq_ignore_ascii_case(&runtime))
        {
            continue;
        }
        let display = {
            let mut chars = runtime.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => runtime.clone(),
            }
        };
        let agent = crate::store::social::ensure_host_runtime_agent(
            &state.store,
            account_id,
            &runtime,
            &display,
            "",
            None,
            now_ms,
        )
        .await?;
        crate::store::social::add_agent_to_conversation(
            &state.store,
            conversation_id,
            &agent.agent_id,
            account_id,
            now_ms,
        )
        .await?;
        tracing::info!(
            target: "minos_backend::social",
            conversation_id = %conversation_id,
            agent_id = %agent.agent_id,
            runtime = %runtime,
            "auto-attached host-runtime agent for @mention dispatch"
        );
    }
    Ok(())
}

/// When dispatch plan is empty but the text looks like agent intent, explain why.
async fn unmatched_agent_intent_error(
    state: &BackendState,
    conversation_id: &str,
    text: &str,
) -> Result<Option<(&'static str, String)>, crate::error::BackendError> {
    let agents =
        crate::store::social::list_conversation_agents(&state.store, conversation_id).await?;
    let tokens: Vec<String> = collect_mention_tokens(text)
        .into_iter()
        .map(str::to_ascii_lowercase)
        .collect();
    let agentish: Vec<&String> = tokens
        .iter()
        .filter(|t| {
            HOST_RUNTIME_MENTIONS.contains(&t.as_str())
                || t.starts_with("bot-")
                || agents.iter().any(|a| {
                    a.agent_id.eq_ignore_ascii_case(t)
                        || a.runtime_agent.eq_ignore_ascii_case(t)
                        || a.name.eq_ignore_ascii_case(t)
                })
        })
        .collect();
    if agentish.is_empty() {
        return Ok(None);
    }
    if agents.is_empty() {
        return Ok(Some((
            "no_agents_in_conversation",
            format!(
                "会话中还没有可用的 Agent（提到了 @{}）。请把 Agent 加进成员后再试。",
                agentish[0]
            ),
        )));
    }
    // Mentions exist but none matched roster (e.g. @codex when only claude is member).
    if first_mentioned_agent(text, &agents).is_none() {
        return Ok(Some((
            "agent_not_in_conversation",
            format!(
                "未匹配到会话成员里的 Agent（@{}）。请确认 Agent 已加入本会话。",
                agentish[0]
            ),
        )));
    }
    Ok(None)
}

/// Surface dispatch failure to Mobile/Desktop: StreamEvent + Envelope + optional chat bubble.
async fn notify_agent_dispatch_failure(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    origin_message_id: &str,
    agent: Option<&crate::store::social::AgentRow>,
    session_id: Option<&str>,
    code: &str,
    detail: String,
) {
    let user_message = match code {
        "peer_offline" | "no_live_host" => {
            "⚠️ 无法启动 Agent：当前没有在线 Host。请打开本机 Desktop/daemon 并确保已链接云端。"
                .to_string()
        }
        "no_agents_in_conversation" | "agent_not_in_conversation" => detail.clone(),
        other => format!("⚠️ Agent 未能启动（{other}）：{detail}"),
    };

    // Formal gateway path (Mobile RealtimeSession).
    let topic = RealtimeTopic::Account(account_id.to_string());
    state.realtime.fanout_stream_event(
        &topic,
        "agent_error",
        None,
        json!({
            "code": code,
            "message": user_message,
            "conversation_id": conversation_id,
            "origin_message_id": origin_message_id,
            "session_id": session_id,
            "agent_id": agent.map(|a| a.agent_id.as_str()),
        }),
    );

    // Legacy envelope path (older clients).
    fan_out_agent_error(
        state,
        account_id,
        session_id.map(str::to_string),
        code,
        user_message.clone(),
    );

    // Visible timeline bubble when we know which agent failed.
    if let Some(agent) = agent {
        let client_id = format!("agent-dispatch-error:{origin_message_id}");
        if let Err(error) = post_agent_social_message(
            state,
            conversation_id,
            agent,
            session_id,
            origin_message_id,
            &user_message,
            &[],
            Some(client_id.as_str()),
        )
        .await
        {
            tracing::warn!(
                target: "minos_backend::social",
                conversation_id = %conversation_id,
                error = ?error,
                "failed to post agent dispatch error bubble"
            );
        }
    }
}

fn agent_row_to_summary(row: &crate::store::social::AgentRow) -> AgentSummary {
    AgentSummary {
        agent_id: row.agent_id.clone(),
        owner_account_id: row.owner_account_id.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        runtime_agent: row.runtime_agent.clone(),
        model: row.model.clone(),
        workspace_path: row.workspace_path.clone(),
        created_at_ms: row.created_at_ms,
        updated_at_ms: row.updated_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_runtime_mention_tokens_are_recognized() {
        assert!(HOST_RUNTIME_MENTIONS.contains(&"grok"));
        assert!(HOST_RUNTIME_MENTIONS.contains(&"codex"));
        let tokens = collect_mention_tokens("@grok 你好 and @codex please");
        assert!(tokens.iter().any(|t| t.eq_ignore_ascii_case("grok")));
        assert!(tokens.iter().any(|t| t.eq_ignore_ascii_case("codex")));
    }

    #[test]
    fn agent_error_code_detects_no_live_host() {
        assert_eq!(
            agent_error_code_for_message("no live host paired to account abc"),
            Some("no_live_host")
        );
        assert_eq!(agent_error_code_for_message("other failure"), None);
    }

    #[test]
    fn completion_watch_cursor_resets_idle_log_on_agent_activity() {
        let start = tokio::time::Instant::now();
        let mut cursor = CompletionWatchCursor::new(10, start);

        let almost_idle = start + GROUP_COMPLETION_IDLE_LOG_INTERVAL - Duration::from_millis(1);
        assert!(!cursor.should_log_idle(almost_idle));

        cursor.observe(11, almost_idle);
        assert_eq!(cursor.last_observed_seq, 11);
        assert!(!cursor.should_log_idle(start + GROUP_COMPLETION_IDLE_LOG_INTERVAL));
        assert!(cursor.should_log_idle(almost_idle + GROUP_COMPLETION_IDLE_LOG_INTERVAL));
    }

    #[test]
    fn completion_watch_cursor_logs_and_stops_after_long_idle() {
        let start = tokio::time::Instant::now();
        let mut cursor = CompletionWatchCursor::new(10, start);

        // Non-increasing seq does not refresh activity (still start).
        cursor.observe(10, start + Duration::from_secs(1));
        cursor.observe(9, start + Duration::from_secs(2));
        assert_eq!(cursor.last_observed_seq, 10);

        // Advance seq → activity clock moves.
        let active = start + Duration::from_secs(3);
        cursor.observe(11, active);
        assert_eq!(cursor.last_observed_seq, 11);

        assert!(!cursor.should_stop_after_long_idle(
            active + GROUP_COMPLETION_IDLE_LOG_INTERVAL - Duration::from_millis(1)
        ));
        assert!(cursor.should_log_idle(active + GROUP_COMPLETION_IDLE_LOG_INTERVAL));
        assert!(cursor.should_stop_after_long_idle(active + GROUP_COMPLETION_IDLE_LOG_INTERVAL));
        assert_eq!(
            cursor.next_poll_delay(active + GROUP_COMPLETION_IDLE_LOG_INTERVAL),
            GROUP_COMPLETION_IDLE_POLL_INTERVAL
        );
    }
}
