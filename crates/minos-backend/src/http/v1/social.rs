use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use serde::Deserialize;

/// Quiet-window settle after host ingest before treating turn as complete
/// (replaces 100ms poll + seq_stable latch).
const GROUP_COMPLETION_SEQ_STABLE: Duration = Duration::from_secs(2);
use crate::app::tx::Storage;
use crate::auth::bearer;
use crate::http::error_response::{err_response, ErrorBody, ErrorEnvelope};
use crate::http::BackendState;
use axum::{Json, Router};
use minos_domain::AgentName;
use minos_protocol::{
    AddAgentToGroupRequest, AgentSummary, ChatMessageSummary, ConversationAgentMembersResponse,
    EnsureHostRuntimeAgentRequest, Envelope, EventKind, ListAgentsResponse, RealtimeTopic,
    RegisterAgentRequest, RemoveAgentFromGroupRequest, SendAgentMessageRequest, SenderType,
    UpdateAgentRequest,
};
use serde_json::json;

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
    #[serde(default)]
    pub attachment_blob_ids: Vec<String>,
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

/// Publish social message delivery after the business row is durable.
///
/// Prefer writers that already committed `chat_messages` + durable + outbox in
/// one transaction (Transactional Outbox). This helper:
/// 1. best-effort legacy envelope fanout
/// 2. **repairs** missing durable/outbox rows if a prior crash left a hole
/// 3. publishes durable events (outbox dispatcher remains the reliability backstop)
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

    match publish_social_message_delivery(state, message, &members).await {
        Ok(()) => {}
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::social",
                conversation_id = %message.conversation_id,
                message_id = %message.message_id,
                error = %error,
                "failed to publish social message durable events; outbox will retry if enqueued"
            );
        }
    }
}

/// Ensure durable+outbox exist (idempotent), then publish. Used by fan-out and
/// as a post-commit publish path after transactional writers.
pub async fn publish_social_message_delivery(
    state: &BackendState,
    message: &ChatMessageSummary,
    member_account_ids: &[String],
) -> Result<(), crate::error::BackendError> {
    let mut tx = Storage::begin(&state.store).await?;
    let pending = crate::store::social::ensure_social_message_delivery_in_tx(
        &mut tx,
        message,
        member_account_ids,
    )
    .await?;
    tx.commit().await?;
    // Pipeline wake: do not wait solely on outbox poll floor.
    if pending.iter().any(|p| p.outbox_id.is_some()) {
        state.wake_outbox();
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    for item in pending {
        state
            .realtime
            .publish_durable_event_by_id(&item.topic_kind, &item.event_id)
            .await?;
        if let Some(outbox_id) = item.outbox_id.as_deref() {
            crate::store::outbox_events::ack(&state.store, outbox_id, now_ms).await?;
        }
    }
    Ok(())
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

/// Build zero or more dispatch plans for a user message.
///
/// Multi-@ fan-out: every unique roster agent mentioned in the body gets its own
/// plan (parallel host sessions). Reply-to-agent and single-agent rooms stay
/// single-plan.
async fn build_agent_dispatch_plans(
    state: &BackendState,
    conversation_id: &str,
    text: &str,
    reply_target: Option<&crate::store::social::ChatMessageRow>,
) -> Result<Vec<AgentDispatchPlan>, (StatusCode, Json<ErrorEnvelope>)> {
    let conversation = crate::store::social::get_conversation(&state.store, conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    let agents = crate::store::social::list_conversation_agents(&state.store, conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    if agents.is_empty() {
        return Ok(Vec::new());
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
                    return Ok(vec![AgentDispatchPlan {
                        agent,
                        session_id: Some(session_id),
                        forwarded_text: text.to_string(),
                        mention_sender,
                    }]);
                }
            }
        }
    }

    // Explicit multi-@ / @agent#short fan-out (order = first appearance).
    let routes = all_mentioned_agent_routes(text, &agents);
    if !routes.is_empty() {
        let mut plans = Vec::with_capacity(routes.len());
        for route in routes {
            let session_id = resolve_dispatch_session_id(
                state,
                conversation_id,
                &route.agent,
                route.session_short_id.as_deref(),
            )
            .await
            .map_err(|e| err("internal", e.to_string()))?;
            plans.push(AgentDispatchPlan {
                agent: route.agent.clone(),
                session_id,
                // Keep full body so each agent sees co-mentions (Buzz-style).
                forwarded_text: text.to_string(),
                mention_sender: true,
            });
        }
        return Ok(plans);
    }

    // Bare text: single-agent rooms auto-route; multi-agent rooms need explicit @.
    if conversation.kind == "group" && human_members.len() == 1 && agents.len() == 1 {
        let agent = agents[0].clone();
        let session_id = resolve_dispatch_session_id(state, conversation_id, &agent, None)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        return Ok(vec![AgentDispatchPlan {
            agent,
            session_id,
            forwarded_text: text.to_string(),
            mention_sender: false,
        }]);
    }

    Ok(Vec::new())
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
    attachments: Vec<minos_protocol::DispatchAttachment>,
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
                origin_message_id: Some(origin_message_id.to_string()),
                // Include agent so multi-@ fan-out send idempotency never collides.
                client_request_id: format!("social-send-{origin_message_id}:{}", agent.agent_id),
                caller_account_id: account_id.to_string(),
                attachments,
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
            origin_message_id: Some(origin_message_id.to_string()),
            // Multi-@ cold start: must key formal session id by origin×agent.
            client_request_id: format!("social-start-{origin_message_id}:{}", agent.agent_id),
            caller_account_id: account_id.to_string(),
            conversation_title,
            attachments,
        })
        .await
        .map_err(|error| map_agent_session_dispatch_error("agent_session.start", error))?;
    Ok(ForwardedAgentDispatch {
        session_id: output.session_id,
        watcher_from_seq: 0,
    })
}

/// Build host-downloadable attachment descriptors for an origin Hub message.
async fn dispatch_attachments_for_origin(
    state: &BackendState,
    account_id: &str,
    origin_message_id: &str,
) -> Result<Vec<minos_protocol::DispatchAttachment>, crate::error::BackendError> {
    let joins = crate::store::message_attachments::list_for_messages(
        &state.store,
        &[origin_message_id.to_string()],
    )
    .await?;
    if joins.is_empty() {
        return Ok(Vec::new());
    }
    if !state.media.is_configured() {
        tracing::warn!(
            target: "minos_backend::social",
            origin_message_id,
            "message has attachments but media store is not configured"
        );
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(joins.len());
    for row in joins {
        if row.status != "ready" {
            continue;
        }
        match state.media.get_download(account_id, &row.blob_id).await {
            Ok(dl) => out.push(minos_protocol::DispatchAttachment {
                blob_id: row.blob_id,
                content_type: row.content_type,
                byte_size: row.byte_size,
                original_filename: row.original_filename,
                download_url: dl.download_url,
            }),
            Err(e) => {
                tracing::warn!(
                    target: "minos_backend::social",
                    blob_id = %row.blob_id,
                    error = %e,
                    "failed to sign attachment download for host dispatch"
                );
            }
        }
    }
    Ok(out)
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
}

fn first_mentioned_agent(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Option<crate::store::social::AgentRow> {
    first_mentioned_agent_route(text, agents).map(|r| r.agent)
}

/// First `@agent` / `@agent#short` route (compat helper for intent errors).
fn first_mentioned_agent_route(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Option<MentionedAgentRoute> {
    all_mentioned_agent_routes(text, agents).into_iter().next()
}

/// All unique `@agent` / `@agent#short` routes in appearance order (multi-@ fan-out).
fn all_mentioned_agent_routes(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Vec<MentionedAgentRoute> {
    let mut out = Vec::new();
    let mut seen_agent_ids = std::collections::HashSet::new();

    // Leading full token first (preserves `#short` when present).
    if let Some(route) = parse_leading_agent_route(text, agents) {
        seen_agent_ids.insert(route.agent.agent_id.clone());
        out.push(route);
    }

    for token in collect_mention_tokens(text) {
        let t = token.trim();
        if t.is_empty() {
            continue;
        }
        let (name_part, short) = split_agent_session_token(t);
        let Some(agent) = match_agent_token(name_part, agents) else {
            continue;
        };
        if !seen_agent_ids.insert(agent.agent_id.clone()) {
            continue;
        }
        out.push(MentionedAgentRoute {
            agent: agent.clone(),
            session_short_id: short.map(str::to_string),
        });
    }
    out
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
    if token.is_empty() {
        return None;
    }
    let (name_part, short) = split_agent_session_token(token);
    let agent = match_agent_token(name_part, agents)?;
    Some(MentionedAgentRoute {
        agent: agent.clone(),
        session_short_id: short.map(str::to_string),
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

/// Arm TurnCompletionProjector for one origin user message (event-driven).
/// Completion is projected on host ingest via [`try_project_completion_for_session`].
/// Persists to `completion_watches` then updates the in-memory cache.
#[allow(clippy::too_many_arguments)]
async fn arm_completion_watch(
    state: &BackendState,
    dispatch_id: String,
    origin_message_id: String,
    conversation_id: String,
    session_id: String,
    agent: crate::store::social::AgentRow,
    raw_seq_floor: u64,
    mention_account_id: Option<String>,
    mention_minos_id: Option<String>,
) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    // Watch TTL is enforced by SessionLifecycle (B5); arm with a long ceiling.
    let deadline_at_ms = now_ms + 30 * 60 * 1000;
    let watch = crate::completion_watch::CompletionWatch {
        dispatch_id: dispatch_id.clone(),
        origin_message_id: origin_message_id.clone(),
        conversation_id: conversation_id.clone(),
        session_id: session_id.clone(),
        agent: agent.clone(),
        raw_seq_floor,
        armed_at_ms: now_ms,
        deadline_at_ms,
        mention_account_id: mention_account_id.clone(),
        mention_minos_id: mention_minos_id.clone(),
    };
    let watch_key = watch.watch_key();
    let durable = crate::store::completion_watches::CompletionWatchRow {
        watch_key: watch_key.clone(),
        dispatch_id: dispatch_id.clone(),
        origin_message_id: origin_message_id.clone(),
        conversation_id,
        session_id: session_id.clone(),
        agent_id: agent.agent_id.clone(),
        raw_seq_floor: raw_seq_floor as i64,
        armed_at_ms: now_ms,
        deadline_at_ms,
        status: crate::store::completion_watches::STATUS_ARMED.to_string(),
        projected_message_id: None,
        mention_account_id,
        mention_minos_id,
    };
    if let Err(error) =
        crate::store::completion_watches::upsert_armed(&state.store, &durable).await
    {
        tracing::error!(
            target: "minos_backend::social",
            session_id = %session_id,
            origin_message_id = %origin_message_id,
            dispatch_id = %dispatch_id,
            error = %error,
            "failed to persist CompletionWatch; arming memory-only (restart risk)"
        );
    }
    state.completion_watches.arm(watch);
    tracing::info!(
        target: "minos_backend::social",
        session_id = %session_id,
        origin_message_id = %origin_message_id,
        dispatch_id = %dispatch_id,
        raw_seq_floor,
        "armed TurnCompletionProjector watch for host-ingest projection"
    );
}

/// Called from host ingest after raw events land. Projects agent bubble(s) when ready.
///
/// Multi-watch: every unfinished (origin, session) on this session is probed.
pub async fn try_project_completion_for_session(state: &BackendState, session_id: &str) {
    let watches = state.completion_watches.list_for_session(session_id);
    if watches.is_empty() {
        return;
    }
    for watch in watches {
        let key = watch.watch_key();
        match try_project_completion_for_watch(state, &key, false).await {
            ProjectOutcome::Pending => {
                // Quiet-window settle. One-shot delayed recheck; post is
                // idempotent via client_message_id.
                let state = state.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(GROUP_COMPLETION_SEQ_STABLE).await;
                    let _ = try_project_completion_for_watch(&state, &key, true).await;
                });
            }
            ProjectOutcome::Done | ProjectOutcome::NotWatching => {}
        }
    }
}

enum ProjectOutcome {
    NotWatching,
    Pending,
    Done,
}

async fn try_project_completion_for_watch(
    state: &BackendState,
    watch_key: &str,
    seq_stable: bool,
) -> ProjectOutcome {
    let Some(watch) = state.completion_watches.get(watch_key) else {
        return ProjectOutcome::NotWatching;
    };
    let session_id = watch.session_id.clone();
    let origin_message_id = watch.origin_message_id.clone();

    let session_terminal = match crate::store::agent_sessions::get(&state.store, &session_id).await
    {
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
                origin_message_id = %origin_message_id,
                "completion projection failed to load agent session"
            );
            false
        }
    };

    match find_completed_agent_reply(
        &state.store,
        &session_id,
        agent_name_for_row(&watch.agent),
        watch.raw_seq_floor,
        session_terminal,
        seq_stable,
    )
    .await
    {
        Ok(crate::turn_completion::CompletionProbe::Ready(text)) => {
            let final_text = watch
                .mention_minos_id
                .as_deref()
                .map_or(text.clone(), |minos_id| format!("@{minos_id} {text}"));
            let mentions = watch.mention_account_id.iter().cloned().collect::<Vec<_>>();
            let client_message_id =
                crate::turn_completion::TurnCompletionProjector::agent_result_client_message_id(
                    &watch.conversation_id,
                    &session_id,
                    &watch.origin_message_id,
                );
            match post_agent_social_message(
                state,
                &watch.conversation_id,
                &watch.agent,
                Some(session_id.as_str()),
                &watch.origin_message_id,
                &final_text,
                &mentions,
                Some(client_message_id.as_str()),
            )
            .await
            {
                Ok(()) => {
                    state.completion_watches.remove(watch_key);
                    let _ = crate::store::completion_watches::mark_projected(
                        &state.store,
                        watch_key,
                        Some(client_message_id.as_str()),
                    )
                    .await;
                    tracing::info!(
                        target: "minos_backend::social",
                        conversation_id = %watch.conversation_id,
                        session_id = %session_id,
                        origin_message_id = %origin_message_id,
                        client_message_id = %client_message_id,
                        "TurnCompletionProjector posted agent bubble (ingest-driven)"
                    );
                    ProjectOutcome::Done
                }
                Err((_, body)) => {
                    tracing::warn!(
                        target: "minos_backend::social",
                        conversation_id = %watch.conversation_id,
                        session_id = %session_id,
                        origin_message_id = %origin_message_id,
                        error = %body.0.error.message,
                        "TurnCompletionProjector post failed; will retry on next ingest"
                    );
                    ProjectOutcome::Pending
                }
            }
        }
        Ok(crate::turn_completion::CompletionProbe::DoneWithoutText) => {
            state.completion_watches.remove(watch_key);
            let _ = crate::store::completion_watches::mark_projected(
                &state.store,
                watch_key,
                None,
            )
            .await;
            tracing::info!(
                target: "minos_backend::social",
                conversation_id = %watch.conversation_id,
                session_id = %session_id,
                origin_message_id = %origin_message_id,
                raw_seq_floor = watch.raw_seq_floor,
                "TurnCompletionProjector finished without clean final text"
            );
            ProjectOutcome::Done
        }
        Ok(crate::turn_completion::CompletionProbe::Pending) => ProjectOutcome::Pending,
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::social",
                error = %error,
                session_id = %session_id,
                origin_message_id = %origin_message_id,
                "TurnCompletionProjector probe failed"
            );
            ProjectOutcome::Pending
        }
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
    let (_message, members) = persist_agent_message_with_delivery(
        state,
        conversation_id,
        &agent.agent_id,
        text,
        Some(reply_to_message_id),
        session_id,
        mentioned_account_ids,
        client_message_id,
    )
    .await?;
    let _ = members;
    Ok(())
}

/// Insert agent message + durable/outbox in one transaction, then publish.
#[allow(clippy::too_many_arguments)]
async fn persist_agent_message_with_delivery(
    state: &BackendState,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
    reply_to_message_id: Option<&str>,
    agent_session_id: Option<&str>,
    mentioned_account_ids: &[String],
    client_message_id: Option<&str>,
) -> Result<(ChatMessageSummary, Vec<String>), (StatusCode, Json<ErrorEnvelope>)> {
    let member_ids = crate::store::social::list_conversation_members(&state.store, conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let agent = crate::store::social::get_agent(&state.store, agent_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "agent not found"))?;
    // Preload reply before opening the write tx (no nested pool checkout).
    let reply_to = match reply_to_message_id {
        Some(id) => {
            let reply_row = crate::store::social::get_message(&state.store, id)
                .await
                .map_err(|e| err("internal", e.to_string()))?;
            match reply_row {
                Some(row) => {
                    let mut hydrated =
                        crate::conversations::use_case::hydrate_messages(&state.store, vec![row])
                            .await
                            .map_err(|e| err("internal", e.to_string()))?;
                    let parent = hydrated.remove(0);
                    Some(minos_protocol::ChatMessageReplySummary {
                        message_id: parent.message_id,
                        sender: parent.sender,
                        text: parent.text,
                        recalled_at_ms: parent.recalled_at_ms,
                    })
                }
                None => None,
            }
        }
        None => None,
    };
    let now_ms = chrono::Utc::now().timestamp_millis();

    let mut tx = Storage::begin(&state.store)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let outcome = crate::store::social::insert_agent_message_with_session_in_tx(
        &mut tx,
        &agent,
        conversation_id,
        text,
        now_ms,
        reply_to_message_id,
        agent_session_id,
        mentioned_account_ids,
        client_message_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;

    let message = if outcome.inserted {
        let sender = minos_protocol::UserSummary {
            account_id: agent.agent_id.clone(),
            minos_id: agent.agent_id.clone(),
            display_name: format!("🤖 {}", agent.name),
        };
        let message = ChatMessageSummary {
            message_id: outcome.row.message_id.clone(),
            conversation_id: outcome.row.conversation_id.clone(),
            sender,
            text: outcome.row.text.clone(),
            created_at_ms: outcome.row.created_at_ms,
            message_seq: outcome.row.message_seq,
            reply_to,
            recalled_at_ms: None,
            mentioned_account_ids: mentioned_account_ids.to_vec(),
            sender_type: SenderType::Agent,
            reactions: vec![],
            attachments: vec![],
        };
        crate::store::social::ensure_social_message_delivery_in_tx(&mut tx, &message, &member_ids)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        // Re-hydrate for API parity with list_messages.
        let mut hydrated =
            crate::conversations::use_case::hydrate_messages(&state.store, vec![outcome.row])
                .await
                .map_err(|e| err("internal", e.to_string()))?;
        let mut full = hydrated.remove(0);
        if full.mentioned_account_ids.is_empty() && !mentioned_account_ids.is_empty() {
            full.mentioned_account_ids = mentioned_account_ids.to_vec();
        }
        full
    } else {
        // Idempotent hit: abandon empty insert tx, hydrate SSOT, repair durable.
        drop(tx);
        let mut hydrated =
            crate::conversations::use_case::hydrate_messages(&state.store, vec![outcome.row])
                .await
                .map_err(|e| err("internal", e.to_string()))?;
        let message = hydrated.remove(0);
        let mut repair_tx = Storage::begin(&state.store)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        crate::store::social::ensure_social_message_delivery_in_tx(
            &mut repair_tx,
            &message,
            &member_ids,
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?;
        repair_tx
            .commit()
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        message
    };

    if let Err(error) = publish_social_message_delivery(state, &message, &member_ids).await {
        tracing::warn!(
            target: "minos_backend::social",
            conversation_id = %conversation_id,
            message_id = %message.message_id,
            error = %error,
            "failed to publish agent social message; outbox will retry if enqueued"
        );
    }
    let frame = Envelope::Event {
        version: 1,
        event: EventKind::SocialMessage {
            conversation_id: message.conversation_id.clone(),
            message: message.clone(),
        },
    };
    state
        .realtime
        .fanout_social_message(&member_ids, &frame)
        .await;
    Ok((message, member_ids))
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
        // Allow `#short` so mid-body `@codex#abcd` keeps session targeting
        // (Desktop `parseAllAgentRoutings` parity).
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric()
                || bytes[end] == b'-'
                || bytes[end] == b'_'
                || bytes[end] == b'#')
        {
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
    let _ = now_ms;
    let (message, _) = persist_agent_message_with_delivery(
        &state,
        &conversation_id,
        &req.agent_id,
        &trimmed,
        reply_to.as_deref(),
        req.agent_session_id.as_deref(),
        &mentioned_account_ids,
        req.client_message_id.as_deref(),
    )
    .await?;
    Ok(Json(message))
}

/// Try to dispatch a message to an agent in the conversation (Mobile / client_live).
/// Plan + enqueue agent dispatch after the user bubble is durable.
///
/// HTTP path returns after this; host RPC runs on [`process_agent_dispatch_batch`].
/// Immediate user-visible errors only for plan-time intent failures (no agent
/// match). Host offline / RPC failures are queued with backoff, then terminal.
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
    let plans =
        build_agent_dispatch_plans(state, conversation_id, trimmed_text, reply_target.as_ref())
            .await
            .map_err(|(_, body)| crate::error::BackendError::StoreQuery {
                operation: "social::try_agent_dispatch.plan".into(),
                message: body.0.error.message,
            })?;

    if plans.is_empty() {
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
    }

    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, conversation_id)
            .await?;
    let sender_minos_id = members
        .iter()
        .find(|m| m.account_id == account_id)
        .map(|m| m.minos_id.clone());
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut any_inserted = false;
    for plan in plans {
        let dispatch_id = uuid::Uuid::new_v4().to_string();
        let row = crate::store::agent_dispatch_queue::AgentDispatchRow {
            dispatch_id,
            origin_message_id: message.message_id.clone(),
            conversation_id: conversation_id.to_string(),
            account_id: account_id.to_string(),
            agent_id: plan.agent.agent_id.clone(),
            session_id: plan.session_id.clone(),
            forwarded_text: plan.forwarded_text,
            mention_sender: plan.mention_sender,
            sender_minos_id: sender_minos_id.clone(),
            status: crate::store::agent_dispatch_queue::STATUS_PENDING.to_string(),
            attempts: 0,
            next_attempt_at_ms: now_ms,
            last_error: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        let inserted = crate::store::agent_dispatch_queue::enqueue(&state.store, &row).await?;
        if inserted {
            any_inserted = true;
            tracing::info!(
                target: "minos_backend::social",
                conversation_id = %conversation_id,
                origin_message_id = %message.message_id,
                agent_id = %plan.agent.agent_id,
                "agent dispatch enqueued"
            );
        } else {
            tracing::debug!(
                target: "minos_backend::social",
                origin_message_id = %message.message_id,
                agent_id = %plan.agent.agent_id,
                "agent dispatch already queued for origin+agent (idempotent)"
            );
        }
    }
    if any_inserted {
        state.wake_agent_dispatch();
    }

    Ok(())
}

/// Host online edge: force due dispatches for linked accounts, then wake worker.
///
/// Production path used from the host WS gateway (and tests) — does **not**
/// require faking `next_attempt_at_ms` via requeue.
///
/// P6: also the multi-instance recovery edge for process-local
/// [`crate::completion_watch::CompletionWatchRegistry`] — re-dispatch re-arms
/// watches on the instance that claims the work (same process as the host WS
/// when workers are co-located).
pub async fn on_host_online_force_agent_dispatch(
    state: &BackendState,
    host_device_id: minos_domain::DeviceId,
) -> Result<u32, crate::error::BackendError> {
    let pairs =
        crate::store::host_links::list_accounts_for_host(&state.store, host_device_id).await?;
    let account_ids: Vec<String> = pairs.into_iter().map(|p| p.mobile_account_id).collect();
    if account_ids.is_empty() {
        state.wake_agent_dispatch();
        return Ok(0);
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let n = crate::store::agent_dispatch_queue::force_due_for_accounts(
        &state.store,
        &account_ids,
        now_ms,
    )
    .await?;
    tracing::info!(
        target: "minos_backend::social",
        host_device_id = %host_device_id,
        accounts = account_ids.len(),
        forced = n,
        "host online: forced agent dispatch queue due"
    );
    state.wake_agent_dispatch();
    Ok(n)
}

/// Drain due AgentDispatchQueue rows: host RPC + arm CompletionWatch.
///
/// Called by [`crate::jobs::agent_dispatch_worker`] and tests.
pub async fn process_agent_dispatch_batch(
    state: &BackendState,
) -> Result<u32, crate::error::BackendError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let claimed = crate::store::agent_dispatch_queue::claim_due(&state.store, now_ms, 16).await?;
    if claimed.is_empty() {
        return Ok(0);
    }
    let mut processed = 0u32;
    for row in claimed {
        match execute_claimed_dispatch(state, &row).await {
            Ok(()) => processed += 1,
            Err(error) => {
                // claim_due already incremented attempts; use post-claim value.
                let attempts = row.attempts.max(1);
                let now_ms = chrono::Utc::now().timestamp_millis();
                let detail = error.to_string();
                if attempts >= crate::store::agent_dispatch_queue::MAX_ATTEMPTS {
                    tracing::warn!(
                        target: "minos_backend::social",
                        error = %error,
                        dispatch_id = %row.dispatch_id,
                        origin_message_id = %row.origin_message_id,
                        attempts,
                        "agent dispatch post-forward error terminal after retries"
                    );
                    if let Err(term_err) = crate::store::agent_dispatch_queue::mark_failed_terminal(
                        &state.store,
                        &row.dispatch_id,
                        &detail,
                        now_ms,
                    )
                    .await
                    {
                        tracing::error!(
                            target: "minos_backend::social",
                            error = %term_err,
                            dispatch_id = %row.dispatch_id,
                            "failed to mark agent dispatch terminal after post-forward error"
                        );
                    }
                    notify_agent_dispatch_failure(
                        state,
                        &row.account_id,
                        &row.conversation_id,
                        &row.origin_message_id,
                        None,
                        row.session_id.as_deref(),
                        "dispatch_post_forward",
                        detail,
                    )
                    .await;
                } else {
                    tracing::warn!(
                        target: "minos_backend::social",
                        error = %error,
                        dispatch_id = %row.dispatch_id,
                        origin_message_id = %row.origin_message_id,
                        attempts,
                        "agent dispatch post-forward error; requeue with backoff"
                    );
                    let backoff_ms = (1_000i64)
                        .saturating_mul(1i64 << attempts.min(6))
                        .min(60_000);
                    if let Err(requeue_err) = crate::store::agent_dispatch_queue::requeue_pending(
                        &state.store,
                        &row.dispatch_id,
                        attempts,
                        now_ms.saturating_add(backoff_ms),
                        &detail,
                        now_ms,
                    )
                    .await
                    {
                        tracing::error!(
                            target: "minos_backend::social",
                            error = %requeue_err,
                            dispatch_id = %row.dispatch_id,
                            "failed to requeue agent dispatch after post-forward error"
                        );
                    }
                }
                processed += 1;
            }
        }
    }
    Ok(processed)
}

async fn execute_claimed_dispatch(
    state: &BackendState,
    row: &crate::store::agent_dispatch_queue::AgentDispatchRow,
) -> Result<(), crate::error::BackendError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let Some(agent) = crate::store::social::get_agent(&state.store, &row.agent_id).await? else {
        crate::store::agent_dispatch_queue::mark_failed_terminal(
            &state.store,
            &row.dispatch_id,
            "agent_not_found",
            now_ms,
        )
        .await?;
        notify_agent_dispatch_failure(
            state,
            &row.account_id,
            &row.conversation_id,
            &row.origin_message_id,
            None,
            row.session_id.as_deref(),
            "agent_not_found",
            format!("Agent {} no longer exists", row.agent_id),
        )
        .await;
        return Ok(());
    };

    let attachments =
        dispatch_attachments_for_origin(state, &row.account_id, &row.origin_message_id).await?;
    match forward_agent_dispatch(
        state,
        &row.account_id,
        &agent,
        row.session_id.clone(),
        &row.forwarded_text,
        &row.conversation_id,
        &row.origin_message_id,
        attachments,
    )
    .await
    {
        Ok(dispatch) => {
            tracing::info!(
                target: "minos_backend::social",
                conversation_id = %row.conversation_id,
                session_id = %dispatch.session_id,
                origin_message_id = %row.origin_message_id,
                agent_id = %agent.agent_id,
                runtime = %agent.runtime_agent,
                "agent dispatch started"
            );
            // Single-agent bind only. Multi-@ would last-writer-win on
            // chat_messages.agent_session_id; completion + queue already key
            // by origin×agent/session.
            let multi_for_origin = crate::store::agent_dispatch_queue::count_by_origin(
                &state.store,
                &row.origin_message_id,
            )
            .await
            .unwrap_or(1);
            if multi_for_origin <= 1 {
                crate::store::social::bind_session_to_message_for_agent(
                    &state.store,
                    &row.origin_message_id,
                    &agent.agent_id,
                    &dispatch.session_id,
                )
                .await?;
            }
            crate::store::agent_dispatch_queue::mark_succeeded(
                &state.store,
                &row.dispatch_id,
                &dispatch.session_id,
                now_ms,
            )
            .await?;
            arm_completion_watch(
                state,
                row.dispatch_id.clone(),
                row.origin_message_id.clone(),
                row.conversation_id.clone(),
                dispatch.session_id,
                agent,
                dispatch.watcher_from_seq,
                if row.mention_sender {
                    Some(row.account_id.clone())
                } else {
                    None
                },
                if row.mention_sender {
                    row.sender_minos_id.clone()
                } else {
                    None
                },
            )
            .await;
        }
        Err(error) => {
            let (code, detail) = agent_error_from_backend_error(&error);
            let attempts = row.attempts;
            if attempts >= crate::store::agent_dispatch_queue::MAX_ATTEMPTS {
                tracing::warn!(
                    target: "minos_backend::social",
                    conversation_id = %row.conversation_id,
                    agent_id = %agent.agent_id,
                    origin_message_id = %row.origin_message_id,
                    attempts,
                    code = %code,
                    detail = %detail,
                    "agent dispatch failed terminal after retries"
                );
                crate::store::agent_dispatch_queue::mark_failed_terminal(
                    &state.store,
                    &row.dispatch_id,
                    &detail,
                    now_ms,
                )
                .await?;
                notify_agent_dispatch_failure(
                    state,
                    &row.account_id,
                    &row.conversation_id,
                    &row.origin_message_id,
                    Some(&agent),
                    row.session_id.as_deref(),
                    code,
                    detail,
                )
                .await;
            } else {
                let delay = crate::store::agent_dispatch_queue::backoff_delay_ms(attempts);
                let next = now_ms + delay;
                tracing::info!(
                    target: "minos_backend::social",
                    conversation_id = %row.conversation_id,
                    origin_message_id = %row.origin_message_id,
                    attempts,
                    next_attempt_at_ms = next,
                    code = %code,
                    "agent dispatch requeued (transient)"
                );
                crate::store::agent_dispatch_queue::requeue_pending(
                    &state.store,
                    &row.dispatch_id,
                    attempts,
                    next,
                    &detail,
                    now_ms,
                )
                .await?;
            }
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

/// Expire CompletionWatch rows past `deadline_at_ms`: user-visible failure + remove.
///
/// Called by SessionLifecycle (B5). Returns the number of watches drained.
pub async fn expire_completion_watches(
    state: &BackendState,
    now_ms: i64,
) -> Result<u32, crate::error::BackendError> {
    let expired = state.completion_watches.drain_expired(now_ms);
    if expired.is_empty() {
        return Ok(0);
    }
    let mut n = 0u32;
    for watch in expired {
        let key = watch.watch_key();
        let _ = crate::store::completion_watches::mark_expired(&state.store, &key).await;
        let account_id = watch
            .mention_account_id
            .clone()
            .unwrap_or_else(|| watch.agent.owner_account_id.clone());
        tracing::warn!(
            target: "minos_backend::social",
            origin_message_id = %watch.origin_message_id,
            session_id = %watch.session_id,
            conversation_id = %watch.conversation_id,
            deadline_at_ms = watch.deadline_at_ms,
            "completion watch TTL expired; projecting failure bubble"
        );
        notify_agent_dispatch_failure(
            state,
            &account_id,
            &watch.conversation_id,
            &watch.origin_message_id,
            Some(&watch.agent),
            Some(&watch.session_id),
            "completion_timeout",
            format!(
                "agent turn did not complete before watch deadline ({})",
                watch.deadline_at_ms
            ),
        )
        .await;
        n = n.saturating_add(1);
    }
    Ok(n)
}

/// Hydrate in-memory CompletionWatch registry from durable `armed` rows.
pub async fn hydrate_completion_watches(
    state: &BackendState,
) -> Result<u32, crate::error::BackendError> {
    let rows = crate::store::completion_watches::list_armed(&state.store).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let mut agent_cache: std::collections::HashMap<String, crate::store::social::AgentRow> =
        std::collections::HashMap::new();
    let mut hydrated = Vec::with_capacity(rows.len());
    for row in rows {
        let agent = if let Some(a) = agent_cache.get(&row.agent_id) {
            a.clone()
        } else {
            match crate::store::social::get_agent(&state.store, &row.agent_id).await? {
                Some(a) => {
                    agent_cache.insert(row.agent_id.clone(), a.clone());
                    a
                }
                None => {
                    tracing::warn!(
                        target: "minos_backend::social",
                        watch_key = %row.watch_key,
                        agent_id = %row.agent_id,
                        "skip hydrate CompletionWatch: agent missing"
                    );
                    let _ =
                        crate::store::completion_watches::mark_expired(&state.store, &row.watch_key)
                            .await;
                    continue;
                }
            }
        };
        hydrated.push(crate::completion_watch::CompletionWatch {
            dispatch_id: row.dispatch_id,
            origin_message_id: row.origin_message_id,
            conversation_id: row.conversation_id,
            session_id: row.session_id,
            agent,
            raw_seq_floor: row.raw_seq_floor as u64,
            armed_at_ms: row.armed_at_ms,
            deadline_at_ms: row.deadline_at_ms,
            mention_account_id: row.mention_account_id,
            mention_minos_id: row.mention_minos_id,
        });
    }
    let n = hydrated.len() as u32;
    state.completion_watches.hydrate(hydrated);
    tracing::info!(
        target: "minos_backend::social",
        count = n,
        "hydrated CompletionWatch registry from durable store"
    );
    Ok(n)
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
        "completion_timeout" => {
            "⚠️ Agent 回合超时：Host 未在时限内完成结果投影。请重试或检查 Desktop/daemon。"
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
    // Multi-@ requires agent/session in the id so failures never collide.
    if let Some(agent) = agent {
        let client_id = if code == "completion_timeout" {
            format!(
                "agent-completion-timeout:{}:{}:{}",
                conversation_id,
                session_id.unwrap_or("none"),
                origin_message_id
            )
        } else {
            format!(
                "agent-dispatch-error:{}:{}:{}",
                conversation_id, agent.agent_id, origin_message_id
            )
        };
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
    fn completion_watch_registry_arms_and_removes_by_watch_key() {
        let reg = crate::completion_watch::CompletionWatchRegistry::new();
        let agent = crate::store::social::AgentRow {
            agent_id: "a1".into(),
            owner_account_id: "acc".into(),
            name: "Codex".into(),
            description: String::new(),
            source: "host_runtime".into(),
            runtime_agent: "codex".into(),
            model: String::new(),
            workspace_path: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        reg.arm(crate::completion_watch::CompletionWatch {
            dispatch_id: "d1".into(),
            origin_message_id: "m1".into(),
            conversation_id: "c1".into(),
            session_id: "sess-1".into(),
            agent,
            raw_seq_floor: 3,
            armed_at_ms: 0,
            deadline_at_ms: 0,
            mention_account_id: None,
            mention_minos_id: None,
        });
        let key = crate::completion_watch::watch_key("m1", "sess-1");
        assert_eq!(reg.get(&key).map(|w| w.raw_seq_floor), Some(3));
        assert_eq!(reg.list_for_session("sess-1").len(), 1);
        assert!(reg.remove(&key).is_some());
        assert!(reg.get(&key).is_none());
    }

    #[test]
    fn multi_mention_routes_unique_agents_in_order() {
        let agents = vec![
            crate::store::social::AgentRow {
                agent_id: "bot-codex".into(),
                owner_account_id: "acc".into(),
                name: "Codex".into(),
                description: String::new(),
                source: "host_runtime".into(),
                runtime_agent: "codex".into(),
                model: String::new(),
                workspace_path: None,
                created_at_ms: 0,
                updated_at_ms: 0,
            },
            crate::store::social::AgentRow {
                agent_id: "bot-claude".into(),
                owner_account_id: "acc".into(),
                name: "Claude".into(),
                description: String::new(),
                source: "host_runtime".into(),
                runtime_agent: "claude".into(),
                model: String::new(),
                workspace_path: None,
                created_at_ms: 0,
                updated_at_ms: 0,
            },
        ];
        let routes = all_mentioned_agent_routes("@codex @claude @codex count off 1 2", &agents);
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].agent.runtime_agent, "codex");
        assert_eq!(routes[1].agent.runtime_agent, "claude");
    }
}
