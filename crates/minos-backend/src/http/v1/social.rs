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
    ConversationParticipantsResponse, EnsureHostRuntimeAgentRequest, ListAgentsResponse,
    RealtimeTopic, RegisterAgentRequest, RemoveAgentFromGroupRequest, SendAgentMessageRequest,
    SenderType, UpdateAgentRequest,
};
use serde_json::json;

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
            "/conversations/:conversation_id/participants",
            post(list_conversation_participants_handler),
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
    pub attachment_blob_ids: Vec<String>,
    /// Structured mention targets. Body text never invents delivery targets.
    #[serde(default)]
    pub mentions: Vec<minos_protocol::MentionTarget>,
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
/// 1. best-effort envelope fanout
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

    let payload = match serde_json::to_value(message) {
        Ok(v) => v,
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::social",
                conversation_id = %message.conversation_id,
                error = %error,
                "failed to encode social message for stream fan-out"
            );
            return;
        }
    };
    for account_id in &members {
        state.realtime.fanout_stream_event(
            &RealtimeTopic::Account(account_id.clone()),
            "social_message",
            None,
            payload.clone(),
        );
    }

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

struct ForwardedAgentDispatch {
    session_id: String,
    watcher_from_seq: u64,
}

/// Private runtime-port inject: `agent_session.send_input` or `start` via HostCommand.
///
/// **Not** the product collaboration path. Call only from
/// [`execute_runtime_port_adapter`] when mailbox Host WS is unavailable.
async fn runtime_port_inject(
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
            host_device_id: Some(host_device_id.to_string()),
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
        if state.registry.get_host(host.host_device_id).is_some() {
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
    let payload = json!({
        "session_id": session_id,
        "code": code,
        "message": message,
    });
    state.realtime.fanout_stream_event(
        &RealtimeTopic::Account(account_id.to_string()),
        "agent_error",
        None,
        payload,
    );
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
    // Watch TTL is enforced by SessionLifecycle; arm with a long ceiling.
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
    if let Err(error) = crate::store::completion_watches::upsert_armed(&state.store, &durable).await
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
                    // Mailbox delivery success is deferred until bot result is durable.
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let _ = crate::store::agent_dispatch_queue::mark_succeeded(
                        &state.store,
                        &watch.dispatch_id,
                        &session_id,
                        now_ms,
                    )
                    .await;
                    let _ = crate::store::agent_dispatch_queue::clear_lease(
                        &state.store,
                        &watch.dispatch_id,
                        now_ms,
                    )
                    .await;
                    tracing::info!(
                        target: "minos_backend::social",
                        conversation_id = %watch.conversation_id,
                        session_id = %session_id,
                        origin_message_id = %origin_message_id,
                        client_message_id = %client_message_id,
                        delivery_id = %watch.dispatch_id,
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
            let _ = crate::store::completion_watches::mark_projected(&state.store, watch_key, None)
                .await;
            // Still terminal for the delivery (no text bubble, but run finished).
            let now_ms = chrono::Utc::now().timestamp_millis();
            let _ = crate::store::agent_dispatch_queue::mark_succeeded(
                &state.store,
                &watch.dispatch_id,
                &session_id,
                now_ms,
            )
            .await;
            let _ = crate::store::agent_dispatch_queue::clear_lease(
                &state.store,
                &watch.dispatch_id,
                now_ms,
            )
            .await;
            tracing::info!(
                target: "minos_backend::social",
                conversation_id = %watch.conversation_id,
                session_id = %session_id,
                origin_message_id = %origin_message_id,
                raw_seq_floor = watch.raw_seq_floor,
                delivery_id = %watch.dispatch_id,
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
        &[],
        client_message_id,
    )
    .await?;
    let _ = members;
    Ok(())
}

/// Host mailbox uplink: AppendBotMessage / host-trusted agent bubble commit.
///
/// Wraps the same `persist_agent_message_with_delivery` path used by
/// `POST …/agents/message` so WS and HTTP share one SSOT.
pub async fn persist_agent_message_from_host(
    state: &BackendState,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
    reply_to_message_id: Option<&str>,
    agent_session_id: Option<&str>,
    client_message_id: &str,
    structured_mentions: &[minos_protocol::MentionTarget],
) -> Result<ChatMessageSummary, crate::error::BackendError> {
    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, conversation_id)
            .await?;
    let agents =
        crate::store::social::list_conversation_agents_active(&state.store, conversation_id)
            .await?;
    // Bot authors are not accounts; sender_account_id is only used to drop self-account.
    let all_agents =
        crate::store::social::list_conversation_agents(&state.store, conversation_id).await?;
    let mentions = crate::conversations::use_case::validate_structured_mentions(
        structured_mentions,
        agent_id,
        &members,
        &agents,
        &all_agents,
    )
    .map_err(|message| crate::error::BackendError::StoreQuery {
        operation: "validate_structured_mentions".into(),
        message,
    })?;
    // Never persist self-mention as a structured agent target.
    let mentioned_agent_ids: Vec<String> = mentions
        .agent_ids
        .into_iter()
        .filter(|id| id != agent_id)
        .collect();
    let (message, _) = persist_agent_message_with_delivery(
        state,
        conversation_id,
        agent_id,
        text,
        reply_to_message_id,
        agent_session_id,
        &mentions.account_ids,
        &mentioned_agent_ids,
        Some(client_message_id),
    )
    .await
    .map_err(
        |(status, envelope)| crate::error::BackendError::StoreQuery {
            operation: "persist_agent_message_from_host".into(),
            message: format!(
                "status={} code={} message={}",
                status.as_u16(),
                envelope.0.error.code,
                envelope.0.error.message
            ),
        },
    )?;
    let owner = crate::store::social::get_agent(&state.store, agent_id)
        .await?
        .map(|a| a.owner_account_id)
        .unwrap_or_default();
    maybe_enqueue_agent_hops(
        state,
        &owner,
        conversation_id,
        &message,
        reply_to_message_id,
        text,
        agent_id,
    )
    .await;
    Ok(message)
}

/// Insert agent message + durable/outbox in one transaction, then publish.
///
/// When the bot body carries structured other-bot mentions, optionally enqueues
/// agent→agent deliveries with automation hop budget.
#[allow(clippy::too_many_arguments)]
async fn persist_agent_message_with_delivery(
    state: &BackendState,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
    reply_to_message_id: Option<&str>,
    agent_session_id: Option<&str>,
    mentioned_account_ids: &[String],
    mentioned_agent_ids: &[String],
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

    // Plan bot→bot hops before opening the write TX (no store reads while holding it).
    let parent_hop = automation_hop_for_agent_message(&state.store, reply_to_message_id, agent_id)
        .await
        .unwrap_or(0);
    let enqueue_hop = parent_hop.saturating_add(1);
    let hop_plans = if !mentioned_agent_ids.is_empty()
        && enqueue_hop <= crate::agent_inbox::MAX_AUTOMATION_HOP
    {
        let plan_message = ChatMessageSummary {
            message_id: String::new(),
            conversation_id: conversation_id.to_string(),
            sender: minos_protocol::MessageSender::Bot {
                bot_id: agent.agent_id.clone(),
                display_name: String::new(),
                runtime_agent: agent.runtime_agent.clone(),
                name: None,
                avatar_url: None,
            },
            text: text.to_string(),
            created_at_ms: now_ms,
            message_seq: 0,
            reply_to: reply_to.clone(),
            recalled_at_ms: None,
            mentioned_account_ids: mentioned_account_ids.to_vec(),
            mentioned_agent_ids: mentioned_agent_ids.to_vec(),
            sender_type: SenderType::Agent,
            reactions: vec![],
            attachments: vec![],
        };
        crate::agent_inbox::plan_agent_deliveries(
            &state.store,
            conversation_id,
            &plan_message,
            text,
            None,
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?
    } else {
        Vec::new()
    };

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
        mentioned_agent_ids,
        client_message_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;

    let inserted = outcome.inserted;
    let message = if outcome.inserted {
        let label = {
            let d = agent.display_name.trim();
            if d.is_empty() {
                agent.name.trim()
            } else {
                d
            }
        };
        let display_name = if label.is_empty() {
            format!("🤖 {}", agent.agent_id)
        } else if label.starts_with('🤖') {
            label.to_string()
        } else {
            format!("🤖 {label}")
        };
        let sender = minos_protocol::MessageSender::Bot {
            bot_id: agent.agent_id.clone(),
            display_name,
            runtime_agent: agent.runtime_agent.clone(),
            name: {
                let n = agent.name.trim();
                if n.is_empty() {
                    None
                } else {
                    Some(n.to_string())
                }
            },
            avatar_url: agent.avatar_url.clone(),
        };
        let message = ChatMessageSummary {
            message_id: outcome.row.message_id.clone(),
            conversation_id: outcome.row.conversation_id.clone(),
            sender: sender.clone(),
            text: outcome.row.text.clone(),
            created_at_ms: outcome.row.created_at_ms,
            message_seq: outcome.row.message_seq,
            reply_to,
            recalled_at_ms: None,
            mentioned_account_ids: mentioned_account_ids.to_vec(),
            mentioned_agent_ids: mentioned_agent_ids.to_vec(),
            sender_type: ChatMessageSummary::sender_type_from(&sender),
            reactions: vec![],
            attachments: vec![],
        };
        crate::store::social::ensure_social_message_delivery_in_tx(&mut tx, &message, &member_ids)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        let hop_rows = crate::agent_inbox::build_dispatch_rows(
            hop_plans,
            &message,
            conversation_id,
            agent.owner_account_id.as_str(),
            None,
            enqueue_hop,
            now_ms,
        );
        let bot_enqueued = crate::agent_inbox::enqueue_plans_in_tx(&mut tx, &hop_rows)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| err("internal", e.to_string()))?;
        if bot_enqueued {
            state.wake_agent_dispatch();
        }
        // Re-hydrate for API parity with list_messages.
        let mut hydrated =
            crate::conversations::use_case::hydrate_messages(&state.store, vec![outcome.row])
                .await
                .map_err(|e| err("internal", e.to_string()))?;
        let mut full = hydrated.remove(0);
        if full.mentioned_account_ids.is_empty() && !mentioned_account_ids.is_empty() {
            full.mentioned_account_ids = mentioned_account_ids.to_vec();
        }
        if full.mentioned_agent_ids.is_empty() && !mentioned_agent_ids.is_empty() {
            full.mentioned_agent_ids = mentioned_agent_ids.to_vec();
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
    if let Ok(payload) = serde_json::to_value(&message) {
        for account_id in &member_ids {
            state.realtime.fanout_stream_event(
                &RealtimeTopic::Account(account_id.clone()),
                "social_message",
                None,
                payload.clone(),
            );
        }
    }

    // Bot→bot hops co-committed on first insert above. Idempotent re-drive of
    // already-committed bot bubbles can still call maybe_enqueue_agent_hops.
    let _ = inserted;

    Ok((message, member_ids))
}

/// Parent automation hop for a bot bubble: prefer the inbox row that delivered
/// this bot for `reply_to` origin; else 0 (treat as root-adjacent).
async fn automation_hop_for_agent_message(
    store: &crate::store::StoreHandle,
    reply_to_message_id: Option<&str>,
    agent_id: &str,
) -> Result<i32, crate::error::BackendError> {
    let Some(origin_id) = reply_to_message_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(0);
    };
    let rows = crate::store::agent_dispatch_queue::list_by_origin(store, origin_id).await?;
    if let Some(row) = rows.iter().find(|r| r.agent_id == agent_id) {
        return Ok(row.automation_hop);
    }
    Ok(rows.first().map(|r| r.automation_hop).unwrap_or(0))
}

/// After a bot bubble is durable, optionally enqueue other-bot deliveries.
/// Kept separate from persist to avoid async recursion with failure bubbles.
async fn maybe_enqueue_agent_hops(
    state: &BackendState,
    owner_account_id: &str,
    conversation_id: &str,
    message: &ChatMessageSummary,
    reply_to_message_id: Option<&str>,
    text: &str,
    agent_id: &str,
) {
    if message.mentioned_agent_ids.is_empty() {
        return;
    }
    let parent_hop = automation_hop_for_agent_message(&state.store, reply_to_message_id, agent_id)
        .await
        .unwrap_or(0);
    if let Err(e) = try_agent_dispatch_with_hop(
        state,
        owner_account_id,
        conversation_id,
        message,
        reply_to_message_id,
        text,
        parent_hop,
    )
    .await
    {
        tracing::warn!(
            target: "minos_backend::social",
            conversation_id = %conversation_id,
            message_id = %message.message_id,
            error = %e,
            "agent→agent inbox pipeline error after bot message"
        );
    }
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
    let display_name = req
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name);
    let avatar_url = req
        .avatar_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if crate::store::social::find_active_agent_name_conflict(&state.store, &account_id, name, None)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .is_some()
    {
        return Err(err(
            "conflict",
            "an active bot with this name already exists (case-insensitive)",
        ));
    }
    let row = crate::store::social::register_agent_full(
        &state.store,
        crate::store::social::RegisterAgentParams {
            owner_account_id: &account_id,
            name,
            display_name,
            description: req.description.trim(),
            avatar_url,
            source: crate::store::social::AGENT_SOURCE_USER,
            runtime_agent: &req.runtime_agent,
            model: req.model.trim(),
            default_reasoning_effort: req.default_reasoning_effort.trim(),
            system_prompt: req.system_prompt.trim(),
            workspace_path: workspace_path.as_deref(),
            now_ms: chrono::Utc::now().timestamp_millis(),
        },
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
    // Load current row for partial digital-body merge (omitted optional fields
    // must not wipe status/avatar/system_prompt/effort).
    let existing = crate::store::social::get_agent(&state.store, &agent_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "agent not found"))?;
    if existing.owner_account_id != account_id {
        return Err(err("not_found", "agent not found or not owned by you"));
    }

    let status = match req
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) if s == crate::store::social::AGENT_STATUS_ACTIVE => {
            crate::store::social::AGENT_STATUS_ACTIVE
        }
        Some(s) if s == crate::store::social::AGENT_STATUS_DISABLED => {
            crate::store::social::AGENT_STATUS_DISABLED
        }
        Some(_) => {
            return Err(err("bad_request", "status must be active or disabled"));
        }
        None => existing.status.as_str(),
    };

    // Active rename: enforce case-insensitive unique name per owner.
    if status == crate::store::social::AGENT_STATUS_ACTIVE {
        if crate::store::social::find_active_agent_name_conflict(
            &state.store,
            &account_id,
            name,
            Some(&agent_id),
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .is_some()
        {
            return Err(err(
                "conflict",
                "an active bot with this name already exists (case-insensitive)",
            ));
        }
    }

    let workspace_path = match &req.workspace_path {
        Some(path) => {
            let normalized = normalize_workspace_path(Some(path.as_str()));
            if let Some(p) = normalized.as_deref() {
                if !is_valid_workspace_path(p) {
                    return Err(err(
                        "bad_request",
                        "workspace_path must be an absolute host path or ~/ path",
                    ));
                }
            }
            normalized
        }
        // Field present on request as Option: when omitted entirely, serde
        // leaves None — keep existing. Callers that want to clear should send "".
        None => existing.workspace_path.clone(),
    };
    // Note: workspace_path Option with skip_serializing means omit=None=keep above.
    // Empty string after normalize clears to None.

    let display_name = req
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name);

    let avatar_url = match &req.avatar_url {
        None => existing.avatar_url.clone(),
        Some(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
    };

    let default_reasoning_effort = match &req.default_reasoning_effort {
        None => existing.default_reasoning_effort.as_str(),
        Some(s) => s.trim(),
    };
    let system_prompt = match &req.system_prompt {
        None => existing.system_prompt.as_str(),
        Some(s) => s.trim(),
    };

    let avatar_url_ref = avatar_url.as_deref();
    let workspace_path_ref = workspace_path.as_deref();
    let row = crate::store::social::update_agent_full(
        &state.store,
        crate::store::social::UpdateAgentParams {
            agent_id: &agent_id,
            owner_account_id: &account_id,
            name,
            display_name,
            description: req.description.trim(),
            avatar_url: avatar_url_ref,
            status,
            runtime_agent: &req.runtime_agent,
            model: req.model.trim(),
            default_reasoning_effort,
            system_prompt,
            workspace_path: workspace_path_ref,
            now_ms: chrono::Utc::now().timestamp_millis(),
        },
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

/// Unified participants read model: humans ∪ bot agents (ADR 0021).
async fn list_conversation_participants_handler(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationParticipantsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    let human_rows =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let agent_rows = crate::store::social::list_conversation_agents(&state.store, &conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let humans = human_rows
        .iter()
        .map(crate::profiles::use_case::to_user_summary)
        .collect();
    let agents = agent_rows.iter().map(agent_row_to_summary).collect();
    Ok(Json(ConversationParticipantsResponse { humans, agents }))
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
    // Structured mentions only (body never invents hop targets).
    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let agents =
        crate::store::social::list_conversation_agents_active(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let all_agents = crate::store::social::list_conversation_agents(&state.store, &conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let mentions = crate::conversations::use_case::validate_structured_mentions(
        &req.mentions,
        &req.agent_id,
        &members,
        &agents,
        &all_agents,
    )
    .map_err(|message| err("bad_request", message))?;
    let mentioned_agent_ids: Vec<String> = mentions
        .agent_ids
        .into_iter()
        .filter(|id| id != &req.agent_id)
        .collect();
    // Agent bubble insert is never a human client_live surface. Default host_projection.
    // Structured other-bot mentions may hop-gate enqueue after persist (not via
    // message_source=client_live). Preferred multi-end writer is TurnCompletionProjector;
    // this endpoint remains for Host-trusted host_projection inserts.
    let message_source = req
        .message_source
        .unwrap_or(minos_protocol::MessageSource::HostProjection);
    if message_source.allows_agent_dispatch() {
        // Explicit client_live on agents/message is rejected: use host_projection.
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
    // Server clock is authoritative; client_sent_at_ms is display-only and unused for ordering.
    let _ = req.client_sent_at_ms;
    let _ = now_ms;
    let (message, _) = persist_agent_message_with_delivery(
        &state,
        &conversation_id,
        &req.agent_id,
        &trimmed,
        reply_to.as_deref(),
        req.agent_session_id.as_deref(),
        &mentions.account_ids,
        &mentioned_agent_ids,
        req.client_message_id.as_deref(),
    )
    .await?;
    maybe_enqueue_agent_hops(
        &state,
        &agent.owner_account_id,
        &conversation_id,
        &message,
        reply_to.as_deref(),
        &trimmed,
        &req.agent_id,
    )
    .await;
    Ok(Json(message))
}

/// Plan + enqueue Agent inbox items after the bubble is durable (participant delivery).
///
/// HTTP path returns after this; Host runtime port runs on [`process_agent_dispatch_batch`].
/// Immediate user-visible errors for plan-time intent failures (unmatched @agent) and for
/// pipeline failures after the user bubble is already committed. Host offline / RPC failures
/// are queued with backoff, then terminal.
///
/// Membership is explicit (participants/add-agent). There is no silent auto-attach.
///
/// Callers:
/// - Human live sends: gate with `message_source.allows_agent_dispatch()` then
///   `try_agent_dispatch(..., automation_hop = 0)`.
/// - Bot-authored messages (optional agent→agent): pass `automation_hop` from the
///   delivery that produced the bot bubble (or 0 if unknown). Enqueue uses hop+1
///   when origin is agent; skips when hop would exceed agent_inbox::MAX_AUTOMATION_HOP.
/// - `host_projection` / `system` human paths must not call this (anti-loop).
pub async fn try_agent_dispatch(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    message: &ChatMessageSummary,
    reply_to_message_id: Option<&str>,
    trimmed_text: &str,
) -> Result<(), crate::error::BackendError> {
    try_agent_dispatch_with_hop(
        state,
        account_id,
        conversation_id,
        message,
        reply_to_message_id,
        trimmed_text,
        0,
    )
    .await
}

/// Same as [`try_agent_dispatch`] with an explicit automation hop for bot→bot chains.
///
/// Re-drive path for already-committed messages (host online force, hop recovery).
/// Live human/bot send paths co-commit deliveries inside the message write TX.
pub async fn try_agent_dispatch_with_hop(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    message: &ChatMessageSummary,
    reply_to_message_id: Option<&str>,
    trimmed_text: &str,
    automation_hop: i32,
) -> Result<(), crate::error::BackendError> {
    let origin_is_agent = message.sender.is_bot() || message.sender_type == SenderType::Agent;
    // Hop on the *new* inbox rows: human roots stay 0; bot-authored origins increment.
    let enqueue_hop = if origin_is_agent {
        automation_hop.saturating_add(1)
    } else {
        0
    };
    if origin_is_agent && enqueue_hop > crate::agent_inbox::MAX_AUTOMATION_HOP {
        tracing::info!(
            target: "minos_backend::social",
            conversation_id = %conversation_id,
            origin_message_id = %message.message_id,
            automation_hop = enqueue_hop,
            max = crate::agent_inbox::MAX_AUTOMATION_HOP,
            "agent→agent inbox skipped: automation hop budget exhausted"
        );
        return Ok(());
    }
    // Bot origins without structured other-bot mentions never plan (no sole-route).
    if origin_is_agent && message.mentioned_agent_ids.is_empty() {
        return Ok(());
    }

    let reply_target = match reply_to_message_id {
        Some(message_id) => crate::store::social::get_message(&state.store, message_id).await?,
        None => None,
    };
    let plans = crate::agent_inbox::plan_agent_deliveries(
        &state.store,
        conversation_id,
        message,
        trimmed_text,
        reply_target.as_ref(),
    )
    .await?;

    if plans.is_empty() {
        // User-visible unmatched structured bot mentions only (body never invents intent).
        if !origin_is_agent {
            if let Some((code, detail)) = unmatched_structured_agent_intent_error(
                state,
                conversation_id,
                &message.mentioned_agent_ids,
            )
            .await?
            {
                tracing::warn!(
                    target: "minos_backend::social",
                    conversation_id = %conversation_id,
                    account_id = %account_id,
                    code = %code,
                    detail = %detail,
                    "agent inbox skipped with user-visible intent"
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
    let rows = crate::agent_inbox::build_dispatch_rows(
        plans,
        message,
        conversation_id,
        account_id,
        sender_minos_id,
        enqueue_hop,
        now_ms,
    );
    let mut any_inserted = false;
    for row in &rows {
        let inserted = crate::store::agent_dispatch_queue::enqueue(&state.store, row).await?;
        if inserted {
            any_inserted = true;
            tracing::info!(
                target: "minos_backend::social",
                conversation_id = %conversation_id,
                origin_message_id = %message.message_id,
                agent_id = %row.agent_id,
                automation_hop = enqueue_hop,
                "agent inbox enqueued"
            );
        } else {
            tracing::debug!(
                target: "minos_backend::social",
                origin_message_id = %message.message_id,
                agent_id = %row.agent_id,
                "agent inbox already queued for origin+agent (idempotent)"
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

/// Drain due bot-mailbox rows: **primary** [`deliver_bot_inbox`], then private
/// runtime-port adapter ([`runtime_port_inject`]) only when no live Host WS.
///
/// Called by [`crate::jobs::agent_dispatch_worker`] and tests.
/// Collaboration semantics stay message-driven; HostCommand is not a product path.
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

    // ── Product path: bot mailbox (BotInboxDelivery over live /ws/host) ──
    match deliver_bot_inbox(state, row, &agent, now_ms).await {
        Ok(Some(session_id)) => {
            return complete_mailbox_lease(state, row, &agent, session_id, now_ms).await;
        }
        Ok(None) => {
            // No live Host WS for mailbox frame → private runtime-port adapter.
        }
        Err(error) => {
            return handle_dispatch_forward_error(state, row, Some(&agent), error, now_ms).await;
        }
    }

    // ── Private runtime-port adapter (HostCommand start/send_input) ──
    // Not a collaboration primitive. Used only when mailbox push cannot reach
    // a live host connection (tests / transient offline host with registry).
    execute_runtime_port_adapter(state, row, &agent, now_ms).await
}

/// After a successful BotInboxDelivery push: stay **inflight** (lease), bind
/// session, arm completion watch. Terminal success is AppendBotMessage / projector.
async fn complete_mailbox_lease(
    state: &BackendState,
    row: &crate::store::agent_dispatch_queue::AgentDispatchRow,
    agent: &crate::store::social::AgentRow,
    session_id: String,
    now_ms: i64,
) -> Result<(), crate::error::BackendError> {
    tracing::info!(
        target: "minos_backend::social",
        conversation_id = %row.conversation_id,
        session_id = %session_id,
        delivery_id = %row.dispatch_id,
        origin_message_id = %row.origin_message_id,
        agent_id = %agent.agent_id,
        "bot mailbox delivery leased and pushed (awaiting host accept/result)"
    );
    maybe_bind_origin_session(state, row, agent, &session_id).await?;
    crate::store::agent_dispatch_queue::set_session_id(
        &state.store,
        &row.dispatch_id,
        &session_id,
        now_ms,
    )
    .await?;
    let watcher_from_seq = crate::store::raw_events::last_seq(&state.store, &session_id).await?;
    arm_completion_watch_for_row(state, row, agent, session_id, watcher_from_seq).await;
    Ok(())
}

/// Offline / no-mailbox-host adapter: inject via agent_session start|send_input
/// (HostCommand under the hood). Marks mailbox row **succeeded** after command
/// enqueue — host still completes via projector when no AppendBotMessage path.
async fn execute_runtime_port_adapter(
    state: &BackendState,
    row: &crate::store::agent_dispatch_queue::AgentDispatchRow,
    agent: &crate::store::social::AgentRow,
    now_ms: i64,
) -> Result<(), crate::error::BackendError> {
    let attachments =
        dispatch_attachments_for_origin(state, &row.account_id, &row.origin_message_id).await?;
    match runtime_port_inject(
        state,
        &row.account_id,
        agent,
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
                "runtime-port adapter injected (no live mailbox host WS)"
            );
            maybe_bind_origin_session(state, row, agent, &dispatch.session_id).await?;
            crate::store::agent_dispatch_queue::mark_succeeded(
                &state.store,
                &row.dispatch_id,
                &dispatch.session_id,
                now_ms,
            )
            .await?;
            arm_completion_watch_for_row(
                state,
                row,
                agent,
                dispatch.session_id,
                dispatch.watcher_from_seq,
            )
            .await;
            Ok(())
        }
        Err(error) => handle_dispatch_forward_error(state, row, Some(agent), error, now_ms).await,
    }
}

async fn maybe_bind_origin_session(
    state: &BackendState,
    row: &crate::store::agent_dispatch_queue::AgentDispatchRow,
    agent: &crate::store::social::AgentRow,
    session_id: &str,
) -> Result<(), crate::error::BackendError> {
    let multi_for_origin =
        crate::store::agent_dispatch_queue::count_by_origin(&state.store, &row.origin_message_id)
            .await
            .unwrap_or(1);
    if multi_for_origin <= 1 {
        crate::store::social::bind_session_to_message_for_agent(
            &state.store,
            &row.origin_message_id,
            &agent.agent_id,
            session_id,
        )
        .await?;
    }
    Ok(())
}

async fn arm_completion_watch_for_row(
    state: &BackendState,
    row: &crate::store::agent_dispatch_queue::AgentDispatchRow,
    agent: &crate::store::social::AgentRow,
    session_id: String,
    watcher_from_seq: u64,
) {
    arm_completion_watch(
        state,
        row.dispatch_id.clone(),
        row.origin_message_id.clone(),
        row.conversation_id.clone(),
        session_id,
        agent.clone(),
        watcher_from_seq,
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

/// Lease a live host and push `ServerFrame::BotInboxDelivery`.
///
/// Returns `Ok(Some(session_id))` when the frame was enqueued on a host socket,
/// `Ok(None)` when no live host connection is available (caller may use adapter).
async fn deliver_bot_inbox(
    state: &BackendState,
    row: &crate::store::agent_dispatch_queue::AgentDispatchRow,
    agent: &crate::store::social::AgentRow,
    now_ms: i64,
) -> Result<Option<String>, crate::error::BackendError> {
    let existing_session = crate::agent_inbox::resolve_dispatch_session_id(
        &state.store,
        &row.conversation_id,
        agent,
        None,
    )
    .await?;

    let host_device_id = if let Some(ref session_id) = existing_session {
        if let Some(session) = crate::store::agent_sessions::get(&state.store, session_id).await? {
            if let Some(host) = session.host_device_id.as_deref() {
                match uuid::Uuid::parse_str(host) {
                    Ok(id) => minos_domain::DeviceId(id),
                    Err(_) => select_live_host_for_account(state, &agent.owner_account_id).await?,
                }
            } else {
                select_live_host_for_account(state, &agent.owner_account_id).await?
            }
        } else {
            select_live_host_for_account(state, &agent.owner_account_id).await?
        }
    } else {
        select_live_host_for_account(state, &agent.owner_account_id).await?
    };

    let hosts = state
        .subscription_mgr
        .host_connections_for_device(host_device_id);
    if hosts.is_empty() {
        return Ok(None);
    }

    let lease_expires_at_ms =
        now_ms.saturating_add(crate::store::agent_dispatch_queue::DEFAULT_LEASE_TTL_MS);
    crate::store::agent_dispatch_queue::set_lease(
        &state.store,
        &row.dispatch_id,
        &host_device_id.to_string(),
        lease_expires_at_ms,
        now_ms,
    )
    .await?;

    // Freeze digital body at schedule time + record host deployment (capability).
    if let Err(error) = crate::store::social::insert_bot_revision(&state.store, agent, now_ms).await
    {
        tracing::warn!(
            target: "minos_backend::social",
            error = %error,
            agent_id = %agent.agent_id,
            "bot revision snapshot failed (continuing delivery)"
        );
    }
    if let Err(error) = crate::store::social::upsert_bot_deployment(
        &state.store,
        &agent.agent_id,
        &host_device_id.to_string(),
        now_ms,
    )
    .await
    {
        tracing::warn!(
            target: "minos_backend::social",
            error = %error,
            agent_id = %agent.agent_id,
            host = %host_device_id,
            "bot deployment upsert failed (continuing delivery)"
        );
    }

    let origin_row = crate::store::social::get_message(&state.store, &row.origin_message_id)
        .await?
        .ok_or_else(|| crate::error::BackendError::StoreQuery {
            operation: "deliver_bot_inbox.origin_message".into(),
            message: format!("origin message {} missing", row.origin_message_id),
        })?;
    let mut hydrated =
        crate::conversations::use_case::hydrate_messages(&state.store, vec![origin_row])
            .await
            .map_err(|e| crate::error::BackendError::StoreQuery {
                operation: "deliver_bot_inbox.hydrate".into(),
                message: e.to_string(),
            })?;
    let message = hydrated.remove(0);

    // Assign a stable formal session id for cold starts so Hub completion
    // watches and Host inject share the same key (daemon must honor it).
    let session_id = existing_session
        .clone()
        .unwrap_or_else(|| format!("mailbox-{}", row.dispatch_id));
    let frame = minos_protocol::realtime::ServerFrame::BotInboxDelivery {
        delivery_id: row.dispatch_id.clone(),
        conversation_id: row.conversation_id.clone(),
        message,
        bot: minos_protocol::realtime::BotLaunchSnapshot {
            bot_id: agent.agent_id.clone(),
            runtime_agent: agent.runtime_agent.clone(),
            model: agent.model.clone(),
            default_reasoning_effort: agent.default_reasoning_effort.clone(),
            system_prompt: agent.system_prompt.clone(),
            display_name: Some(agent.display_name.clone()),
            workspace_path: agent.workspace_path.clone(),
        },
        session: minos_protocol::realtime::SessionBinding {
            session_id: Some(session_id.clone()),
            create_if_missing: existing_session.is_none(),
        },
        lease_expires_at_ms,
    };

    let mut pushed = false;
    for conn in hosts {
        match conn.send(frame.clone()) {
            Ok(()) => {
                pushed = true;
                break;
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::social",
                    delivery_id = %row.dispatch_id,
                    host_device_id = %host_device_id,
                    error = %error,
                    "failed to push BotInboxDelivery to host connection"
                );
            }
        }
    }
    if !pushed {
        let _ =
            crate::store::agent_dispatch_queue::clear_lease(&state.store, &row.dispatch_id, now_ms)
                .await;
        return Ok(None);
    }

    Ok(Some(session_id))
}

async fn handle_dispatch_forward_error(
    state: &BackendState,
    row: &crate::store::agent_dispatch_queue::AgentDispatchRow,
    agent: Option<&crate::store::social::AgentRow>,
    error: crate::error::BackendError,
    now_ms: i64,
) -> Result<(), crate::error::BackendError> {
    let (code, detail) = agent_error_from_backend_error(&error);
    let attempts = row.attempts;
    if attempts >= crate::store::agent_dispatch_queue::MAX_ATTEMPTS {
        tracing::warn!(
            target: "minos_backend::social",
            conversation_id = %row.conversation_id,
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
            agent,
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
    Ok(())
}

/// When the plan is empty despite structured bot mentions, explain why.
/// Body text is never consulted — only membership-validated mention ids.
async fn unmatched_structured_agent_intent_error(
    state: &BackendState,
    conversation_id: &str,
    mentioned_agent_ids: &[String],
) -> Result<Option<(&'static str, String)>, crate::error::BackendError> {
    if mentioned_agent_ids.is_empty() {
        return Ok(None);
    }
    let all_agents =
        crate::store::social::list_conversation_agents(&state.store, conversation_id).await?;
    let active_agents: Vec<_> = all_agents
        .iter()
        .filter(|a| a.is_active())
        .cloned()
        .collect();
    for agent_id in mentioned_agent_ids {
        if let Some(disabled) = all_agents
            .iter()
            .find(|a| !a.is_active() && a.agent_id == *agent_id)
        {
            let label = if disabled.display_name.trim().is_empty() {
                disabled.name.as_str()
            } else {
                disabled.display_name.as_str()
            };
            return Ok(Some((
                "agent_disabled",
                format!("Agent「{label}」已停用，无法投递。请在 Agents 中重新启用后再试。"),
            )));
        }
    }
    if active_agents.is_empty() {
        return Ok(Some((
            "no_agents_in_conversation",
            "会话中还没有可用的 Agent。请把 Agent 加进成员后再试。".to_string(),
        )));
    }
    // Structured ids present but none planned (stale/removed between write and plan).
    Ok(Some((
        "agent_not_in_conversation",
        format!(
            "未匹配到会话成员里的 Agent（{}）。请确认 Agent 已加入本会话。",
            mentioned_agent_ids[0]
        ),
    )))
}

/// Expire CompletionWatch rows past `deadline_at_ms`: user-visible failure + remove.
///
/// Called by SessionLifecycle. Returns the number of watches drained.
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
                    let _ = crate::store::completion_watches::mark_expired(
                        &state.store,
                        &row.watch_key,
                    )
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

/// Surface a post-commit agent inbox pipeline error (plan/enqueue failure).
///
/// The user bubble is already durable; HTTP still returns 200. This path makes
/// the failure user-visible instead of warn-only.
pub async fn notify_agent_dispatch_pipeline_error(
    state: &BackendState,
    account_id: &str,
    conversation_id: &str,
    origin_message_id: &str,
    error: &crate::error::BackendError,
) {
    let (code, detail) = agent_error_from_backend_error(error);
    notify_agent_dispatch_failure(
        state,
        account_id,
        conversation_id,
        origin_message_id,
        None,
        None,
        code,
        detail,
    )
    .await;
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
        display_name: if row.display_name.is_empty() {
            row.name.clone()
        } else {
            row.display_name.clone()
        },
        description: row.description.clone(),
        avatar_url: row.avatar_url.clone(),
        source: row.source.clone(),
        status: row.status.clone(),
        runtime_agent: row.runtime_agent.clone(),
        model: row.model.clone(),
        default_reasoning_effort: row.default_reasoning_effort.clone(),
        system_prompt: row.system_prompt.clone(),
        workspace_path: row.workspace_path.clone(),
        created_at_ms: row.created_at_ms,
        updated_at_ms: row.updated_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let agent = crate::store::social::AgentRow::test_stub(
            "a1",
            "acc",
            "Codex",
            "host_runtime",
            "codex",
        );
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
    fn host_projection_and_system_never_allow_agent_dispatch() {
        use minos_protocol::MessageSource;
        assert!(MessageSource::ClientLive.allows_agent_dispatch());
        assert!(!MessageSource::HostProjection.allows_agent_dispatch());
        assert!(!MessageSource::System.allows_agent_dispatch());
    }

    #[test]
    fn automation_hop_budget_is_small_positive() {
        // bot-mailbox loop control: human root hop=0; each bot→bot increments;
        // enqueue when hop would exceed MAX is skipped.
        // hop 0 (human) → enqueue 0; hop 2 origin agent → enqueue 3 (allowed);
        // hop 3 origin agent → enqueue 4 (blocked).
        const {
            assert!(crate::agent_inbox::MAX_AUTOMATION_HOP > 0);
            assert!(crate::agent_inbox::MAX_AUTOMATION_HOP == 3);
            assert!(3 <= crate::agent_inbox::MAX_AUTOMATION_HOP);
            assert!(4 > crate::agent_inbox::MAX_AUTOMATION_HOP);
        }
    }
}
