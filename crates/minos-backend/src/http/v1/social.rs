use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use serde::Deserialize;

const GROUP_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const GROUP_COMPLETION_IDLE_LOG_INTERVAL: Duration = Duration::from_mins(5);
const GROUP_COMPLETION_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(5);
use crate::app::tx::Storage;
use crate::auth::bearer;
use crate::http::error_response::{err_response, ErrorBody, ErrorEnvelope};
use crate::http::BackendState;
use axum::{Json, Router};
use minos_domain::AgentName;
use minos_protocol::{
    AddAgentToGroupRequest, AgentSummary, ChatMessageSummary, ConversationAgentMembersResponse,
    DurableEvent, Envelope, EventKind, ListAgentsResponse, RegisterAgentRequest,
    RemoveAgentFromGroupRequest, SendAgentMessageRequest, SenderRef, SenderType,
    UpdateAgentRequest, UserSummary,
};
use uuid::Uuid;

pub fn router() -> Router<BackendState> {
    Router::new()
        // ─── Agent routes ───
        .route("/agents", post(register_agent))
        .route("/agents/query", post(list_agents))
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
        let session_id = crate::store::social::lookup_latest_session_id_for_conversation(
            &state.store,
            conversation_id,
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?;
        return Ok(Some(AgentDispatchPlan {
            agent: agents[0].clone(),
            session_id,
            forwarded_text: text.to_string(),
            mention_sender: false,
        }));
    }

    if conversation.kind == "group" {
        if let Some(agent) = first_mentioned_agent(text, &agents) {
            let session_id = crate::store::social::lookup_latest_session_id_for_conversation_agent(
                &state.store,
                conversation_id,
                &agent.agent_id,
            )
            .await
            .map_err(|e| err("internal", e.to_string()))?;
            return Ok(Some(AgentDispatchPlan {
                agent: agent.clone(),
                session_id,
                forwarded_text: strip_agent_mention_once(text, &agent.agent_id),
                mention_sender: true,
            }));
        }
    }

    Ok(None)
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
    let output = state
        .agent_sessions
        .start(crate::agent_sessions::StartAgentSessionInput {
            conversation_id: conversation_id.to_string(),
            project_id: None,
            agent_id: agent.agent_id.clone(),
            host_installation_id: Some(host_device_id.to_string()),
            workspace_path: agent.workspace_path.clone(),
            initial_user_message: Some(text.to_string()),
            client_request_id: format!("social-start-{origin_message_id}"),
            caller_account_id: account_id.to_string(),
        })
        .await
        .map_err(|error| map_agent_session_dispatch_error("agent_session.start", error))?;
    Ok(ForwardedAgentDispatch {
        session_id: output.session_id,
        watcher_from_seq: 0,
    })
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
    let hosts =
        crate::store::account_host_pairings::list_hosts_for_account(&state.store, account_id)
            .await?;
    for host in hosts {
        if state.registry.get(host.host_device_id).is_some() {
            return Ok(host.host_device_id);
        }
    }
    Err(crate::error::BackendError::ForwardRpc {
        method: "agent_session.start".into(),
        message: format!("no live host paired to account {account_id}"),
    })
}

fn first_mentioned_agent(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Option<crate::store::social::AgentRow> {
    let by_id = agents
        .iter()
        .map(|agent| (agent.agent_id.as_str(), agent))
        .collect::<HashMap<_, _>>();
    collect_mention_tokens(text)
        .into_iter()
        .find_map(|token| by_id.get(token).copied().cloned())
}

fn strip_agent_mention_once(text: &str, agent_id: &str) -> String {
    let stripped = text.replacen(&format!("@{agent_id}"), "", 1);
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
            ("dispatch_failed", message.clone())
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
            match crate::store::raw_events::last_seq(&state.store, &session_id).await {
                Ok(latest_seq) => {
                    cursor.observe(latest_seq, tokio::time::Instant::now());
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

            match find_completed_agent_reply(
                &state.store,
                &session_id,
                agent_name_for_row(&agent),
                trigger_seq,
            )
            .await
            {
                Ok(Some(text)) => {
                    let final_text = mention_minos_id
                        .as_deref()
                        .map_or(text.clone(), |minos_id| format!("@{minos_id} {text}"));
                    let mentions = mention_account_id.iter().cloned().collect::<Vec<_>>();
                    let _ = post_agent_social_message(
                        &state,
                        &conversation_id,
                        &agent,
                        &session_id,
                        &reply_to_message_id,
                        &final_text,
                        &mentions,
                    )
                    .await;
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::social",
                        error = %error,
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        "group completion watcher failed to translate session state"
                    );
                }
            }

            let now = tokio::time::Instant::now();
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

    fn next_poll_delay(self, now: tokio::time::Instant) -> Duration {
        if now.saturating_duration_since(self.last_activity_at)
            >= GROUP_COMPLETION_IDLE_LOG_INTERVAL
        {
            return GROUP_COMPLETION_IDLE_POLL_INTERVAL;
        }
        GROUP_COMPLETION_POLL_INTERVAL
    }
}

async fn find_completed_agent_reply(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    agent_name: AgentName,
    trigger_seq: u64,
) -> Result<Option<String>, crate::error::BackendError> {
    let rows = crate::store::raw_events::read_range(pool, session_id, 1, 10_000).await?;

    match agent_name {
        AgentName::Codex => {
            let mut translator =
                minos_ui_protocol::CodexTranslatorState::new(session_id.to_string());
            let mut message_texts = HashMap::<String, String>::new();
            for row in rows {
                let events = minos_ui_protocol::translate_codex(&mut translator, &row.payload)
                    .map_err(|error| crate::error::BackendError::ForwardRpc {
                        method: "group_completion_watcher".into(),
                        message: error.to_string(),
                    })?;
                for event in events {
                    match event {
                        minos_ui_protocol::UiEventMessage::MessageStarted {
                            role: minos_ui_protocol::MessageRole::Assistant,
                            message_id,
                            ..
                        } => {
                            message_texts.entry(message_id).or_default();
                        }
                        minos_ui_protocol::UiEventMessage::TextDelta { message_id, text } => {
                            message_texts
                                .entry(message_id)
                                .or_default()
                                .push_str(&text.render_preview());
                        }
                        minos_ui_protocol::UiEventMessage::MessageCompleted {
                            message_id, ..
                        } if u64::try_from(row.seq).unwrap_or_default() > trigger_seq => {
                            let text = message_texts.remove(&message_id).unwrap_or_default();
                            return Ok(Some(text.trim().to_string()));
                        }
                        _ => {}
                    }
                }
            }
            Ok(None)
        }
        AgentName::Claude | AgentName::Gemini | AgentName::Opencode | AgentName::Grok => Ok(None),
    }
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

async fn post_agent_social_message(
    state: &BackendState,
    conversation_id: &str,
    agent: &crate::store::social::AgentRow,
    session_id: &str,
    reply_to_message_id: &str,
    text: &str,
    mentioned_account_ids: &[String],
) -> Result<(), (StatusCode, Json<ErrorEnvelope>)> {
    let row = crate::store::social::insert_agent_message_with_session(
        &state.store,
        conversation_id,
        &agent.agent_id,
        text,
        chrono::Utc::now().timestamp_millis(),
        Some(reply_to_message_id),
        Some(session_id),
        mentioned_account_ids,
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
    // Verify the agent is in this conversation
    if !crate::store::social::is_agent_in_conversation(
        &state.store,
        &conversation_id,
        &req.agent_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err(
            "bad_request",
            "agent is not a member of this conversation",
        ));
    }
    // Extract mentions from the message
    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let mentioned_account_ids = extract_mentioned_account_ids(&trimmed, &req.agent_id, &members);
    let row = crate::store::social::insert_agent_message(
        &state.store,
        &conversation_id,
        &req.agent_id,
        &trimmed,
        chrono::Utc::now().timestamp_millis(),
        req.reply_to_message_id.as_deref(),
        &mentioned_account_ids,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    // Hydrate the agent message with agent info as sender
    let message = ChatMessageSummary {
        message_id: row.message_id,
        conversation_id: row.conversation_id,
        sender: UserSummary {
            account_id: agent.agent_id.clone(),
            minos_id: agent.agent_id.clone(),
            display_name: format!("🤖 {}", agent.name),
        },
        text: row.text,
        created_at_ms: row.created_at_ms,
        reply_to: None,
        recalled_at_ms: row.recalled_at_ms,
        mentioned_account_ids,
        sender_type: SenderType::Agent,
    };
    fan_out_social_message(&state, &message).await;
    Ok(Json(message))
}

/// Try to dispatch a message to an agent in the conversation.
/// Called by the conversations handler after a message is sent.
/// Returns Ok(()) if no agent was dispatched or if dispatch succeeded,
/// or Err if something went wrong.
pub async fn try_agent_dispatch(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    message: &ChatMessageSummary,
    reply_to_message_id: Option<&str>,
    trimmed_text: &str,
) -> Result<(), crate::error::BackendError> {
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
            fan_out_agent_error(state, account_id, plan.session_id, code, detail);
        }
    }

    Ok(())
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
    fn completion_watch_cursor_logs_but_does_not_timeout_after_idle() {
        let start = tokio::time::Instant::now();
        let mut cursor = CompletionWatchCursor::new(10, start);

        cursor.observe(10, start + Duration::from_secs(1));
        cursor.observe(9, start + Duration::from_secs(2));

        assert_eq!(cursor.last_observed_seq, 10);
        assert!(!cursor.should_log_idle(
            start + GROUP_COMPLETION_IDLE_LOG_INTERVAL - Duration::from_millis(1)
        ));
        assert!(cursor.should_log_idle(start + GROUP_COMPLETION_IDLE_LOG_INTERVAL));
        assert!(!cursor.should_log_idle(start + GROUP_COMPLETION_IDLE_LOG_INTERVAL));
        assert_eq!(
            cursor.next_poll_delay(start + GROUP_COMPLETION_IDLE_LOG_INTERVAL),
            GROUP_COMPLETION_IDLE_POLL_INTERVAL
        );
        assert!(cursor.should_log_idle(start + GROUP_COMPLETION_IDLE_LOG_INTERVAL * 2));
    }
}
