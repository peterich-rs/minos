use std::collections::{HashMap, HashSet};

use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::Response,
};
use futures::StreamExt;
use minos_domain::{AgentName, DeviceId, DeviceRole};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::Instrument;
use uuid::Uuid;

use crate::auth::realtime_ticket::RealtimeTicketConsumeError;
use crate::error::BackendError;
use crate::http::BackendState;
use crate::ingest::use_case::IngestCommand;
use crate::realtime::auth::{self, SubscriptionAuthError, SubscriptionDenied};
use crate::realtime::subscription::ConnectionState;
use crate::session::{ServerFrame as LegacySessionFrame, SessionHandle, SessionRevocation};
use crate::store::{
    agent_sessions, agent_turn_events, agent_turns, durable_event_log, host_commands,
    outbox_events, raw_events, thread_sync_state, sessions,
};
use minos_protocol::realtime::{
    ClientFrame, ConnectionPrincipal, HostGapManifest, HostIngestChunk, HostIngestLiveBatch,
    HostIngestPullResponse, PullPriority, PullReason, RealtimeTopic, ServerFrame,
};
use minos_ui_protocol::UiEventMessage;

const PUSH_CHANNEL_CAPACITY: usize = 256;
const SUBSCRIBE_LIMIT_PER_REQUEST: usize = 32;
const LIVE_SUBSCRIPTION_LIMIT: usize = 128;
const REPLAY_BATCH_SIZE: u32 = 256;
const PULL_INGEST_MAX_BYTES: u64 = 0;
const HEARTBEAT_INTERVAL_MS: i64 = 25_000;
const CLOSE_CODE_AUTH_REVOKED: u16 = 4401;
const CLOSE_CODE_INTERNAL_ERROR: u16 = 1011;

#[derive(Debug, Deserialize, Default)]
pub struct GatewayWsQuery {
    pub ws_ticket: Option<String>,
    pub ticket: Option<String>,
}

impl GatewayWsQuery {
    fn ticket(&self) -> Option<&str> {
        self.ticket.as_deref().or(self.ws_ticket.as_deref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayRail {
    Client,
    Host,
}

#[derive(Debug)]
enum ActivationAuthError {
    Unauthorized(String),
    Internal(String),
}

pub async fn upgrade_client(
    State(state): State<BackendState>,
    Query(query): Query<GatewayWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let result = upgrade_with_ticket(state, query, ws, GatewayRail::Client).await;
    if result.is_err() {
        crate::telemetry::record_ws_connect("preauth", crate::telemetry::OUTCOME_UNAUTHORIZED);
    }
    result
}

pub async fn upgrade_host(
    State(state): State<BackendState>,
    Query(query): Query<GatewayWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let result = upgrade_with_ticket(state, query, ws, GatewayRail::Host).await;
    if result.is_err() {
        crate::telemetry::record_ws_connect("preauth", crate::telemetry::OUTCOME_UNAUTHORIZED);
    }
    result
}

async fn upgrade_with_ticket(
    state: BackendState,
    query: GatewayWsQuery,
    ws: WebSocketUpgrade,
    rail: GatewayRail,
) -> Result<Response, (StatusCode, String)> {
    let ticket = query
        .ticket()
        .ok_or((StatusCode::UNAUTHORIZED, "ticket required".to_string()))?;
    let claims =
        crate::auth::jwt::verify_ws_ticket(state.auth.jwt_secret(), ticket).map_err(|error| {
            tracing::debug!(
                target: "minos_backend::realtime",
                error = %error,
                "websocket ws_ticket verify failed"
            );
            (StatusCode::UNAUTHORIZED, "invalid ws_ticket".to_string())
        })?;
    let device_id = Uuid::parse_str(&claims.did)
        .map(DeviceId)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid ws_ticket".to_string()))?;

    match rail {
        GatewayRail::Client if !claims.role.is_account_client() => {
            return Err((StatusCode::UNAUTHORIZED, "invalid ws_ticket".to_string()));
        }
        GatewayRail::Host if claims.role != DeviceRole::AgentHost => {
            return Err((StatusCode::UNAUTHORIZED, "invalid ws_ticket".to_string()));
        }
        _ => {}
    }

    state
        .auth
        .consume_ws_ticket(&claims)
        .await
        .map_err(|error| match error {
            RealtimeTicketConsumeError::Missing
            | RealtimeTicketConsumeError::Expired
            | RealtimeTicketConsumeError::Mismatch => {
                tracing::debug!(
                    target: "minos_backend::realtime",
                    error = ?error,
                    jti = %claims.jti,
                    "websocket ws_ticket consume failed"
                );
                (StatusCode::UNAUTHORIZED, "invalid ws_ticket".to_string())
            }
            RealtimeTicketConsumeError::Store(message) => {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    error = %message,
                    jti = %claims.jti,
                    "websocket ws_ticket store unavailable"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "ws_ticket unavailable".to_string(),
                )
            }
        })?;

    let principal = if claims.role == DeviceRole::AgentHost {
        ConnectionPrincipal::Host {
            host_installation_id: claims.did.clone(),
        }
    } else {
        ConnectionPrincipal::Account {
            account_id: claims.sub.clone(),
        }
    };
    let request_span = tracing::Span::current();
    Ok(ws.on_upgrade(move |mut socket| {
        let state = state.clone();
        async move {
            match revalidate_ws_ticket_auth(&state.store, &claims).await {
                Ok(()) => {
                    run_session(
                        socket,
                        state,
                        GatewayUpgrade {
                            device_id,
                            role: claims.role,
                            principal,
                        },
                    )
                    .await;
                }
                Err(ActivationAuthError::Unauthorized(message)) => {
                    tracing::info!(
                        target: "minos_backend::realtime",
                        device_id = %device_id,
                        reason = %message,
                        "websocket auth changed before activation; closing 4401"
                    );
                    close_with_directive(
                        &mut socket,
                        claims.role,
                        CloseDirective {
                            code: CLOSE_CODE_AUTH_REVOKED,
                            reason: "auth_revoked",
                        },
                    )
                    .await;
                }
                Err(ActivationAuthError::Internal(message)) => {
                    tracing::warn!(
                        target: "minos_backend::realtime",
                        device_id = %device_id,
                        error = %message,
                        "websocket activation revalidation failed"
                    );
                    close_with_directive(
                        &mut socket,
                        claims.role,
                        CloseDirective {
                            code: CLOSE_CODE_INTERNAL_ERROR,
                            reason: "activation_revalidate_failed",
                        },
                    )
                    .await;
                }
            }
        }
        .instrument(request_span)
    }))
}

async fn revalidate_ws_ticket_auth(
    store: &crate::store::StoreHandle,
    claims: &crate::auth::jwt::WsTicketClaims,
) -> Result<(), ActivationAuthError> {
    let device_id = Uuid::parse_str(&claims.did)
        .map(DeviceId)
        .map_err(|_| ActivationAuthError::Unauthorized("invalid ws_ticket device_id".into()))?;
    let row = crate::store::devices::get_device(store, device_id)
        .await
        .map_err(|error| ActivationAuthError::Internal(error.to_string()))?
        .ok_or_else(|| {
            ActivationAuthError::Unauthorized(
                "device row missing during websocket activation".to_string(),
            )
        })?;

    if row.role != claims.role {
        return Err(ActivationAuthError::Unauthorized(format!(
            "device role changed during websocket activation: expected {}, got {}",
            claims.role, row.role
        )));
    }
    if claims.role.is_account_client() && row.account_id.as_deref() != Some(claims.sub.as_str()) {
        return Err(ActivationAuthError::Unauthorized(
            "device account changed during websocket activation".to_string(),
        ));
    }
    if claims.role == DeviceRole::AgentHost && claims.sub != claims.did {
        return Err(ActivationAuthError::Unauthorized(
            "host ws_ticket subject mismatch".to_string(),
        ));
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct GatewayUpgrade {
    pub device_id: DeviceId,
    pub role: DeviceRole,
    pub principal: ConnectionPrincipal,
}

impl GatewayUpgrade {
    #[must_use]
    fn account_id(&self) -> Option<&str> {
        self.principal.account_id()
    }

    #[must_use]
    fn default_topic(&self) -> RealtimeTopic {
        match &self.principal {
            ConnectionPrincipal::Account { account_id } => {
                RealtimeTopic::Account(account_id.clone())
            }
            ConnectionPrincipal::Host {
                host_installation_id,
            } => RealtimeTopic::Host(host_installation_id.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CloseDirective {
    code: u16,
    reason: &'static str,
}

pub async fn run_session(mut ws: WebSocket, state: BackendState, upgrade: GatewayUpgrade) {
    let role_label = crate::envelope::role_metric_label(upgrade.role);
    crate::telemetry::record_ws_connect(role_label, crate::telemetry::OUTCOME_OK);
    crate::telemetry::record_session_role_open(role_label);

    let (push_tx, mut push_rx) = mpsc::channel(PUSH_CHANNEL_CAPACITY);
    let created_at_ms = chrono::Utc::now().timestamp_millis();
    let conn = std::sync::Arc::new(ConnectionState::new(
        upgrade.principal.clone(),
        upgrade.device_id,
        upgrade.role,
        push_tx,
        created_at_ms,
    ));
    state
        .subscription_mgr
        .add_connection(std::sync::Arc::clone(&conn));

    // The registry still owns per-device replacement/revocation state while
    // the rest of the backend migrates off legacy envelope-side sessions.
    let (legacy_handle, mut legacy_outbox_rx) = SessionHandle::new(upgrade.device_id, upgrade.role);
    if let Some(account_id) = upgrade.account_id() {
        legacy_handle.set_account_id(account_id.to_string());
    }
    if let Some(previous) = state.registry.insert(legacy_handle.clone()) {
        previous.revoke(SessionRevocation::Superseded);
    }
    let mut revocation_rx = legacy_handle.subscribe_revocation();

    touch_connection_last_seen(&state, &upgrade.device_id, "ws.open").await;

    let close_reason = run_session_inner(
        &mut ws,
        &state,
        &upgrade,
        &conn,
        &mut legacy_outbox_rx,
        &mut push_rx,
        &mut revocation_rx,
    )
    .await;

    state.subscription_mgr.remove_connection(conn.conn_id);
    let _ = state.registry.remove_current(&legacy_handle);
    crate::telemetry::record_ws_close(role_label, close_reason);
    crate::telemetry::record_session_role_close(role_label);
}

#[allow(clippy::too_many_arguments)]
async fn run_session_inner(
    ws: &mut WebSocket,
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    conn: &std::sync::Arc<ConnectionState>,
    legacy_outbox_rx: &mut mpsc::Receiver<LegacySessionFrame>,
    push_rx: &mut mpsc::Receiver<ServerFrame>,
    revocation_rx: &mut tokio::sync::watch::Receiver<Option<SessionRevocation>>,
) -> &'static str {
    if send_server_frame(
        ws,
        &ServerFrame::Hello {
            conn_id: conn.conn_id.to_string(),
            server_time_ms: chrono::Utc::now().timestamp_millis(),
            heartbeat_interval_ms: HEARTBEAT_INTERVAL_MS,
        },
    )
    .await
    .is_err()
    {
        return "write_failed";
    }

    if let Err(error) = auto_subscribe_default_topic(ws, state, conn, upgrade).await {
        tracing::warn!(
            target: "minos_backend::realtime",
            error = %error,
            device_id = %upgrade.device_id,
            "failed to auto-subscribe default realtime topic"
        );
        let _ = send_error_frame(ws, conn, "internal", error.to_string()).await;
    }

    loop {
        let current_revocation = { *revocation_rx.borrow() };
        if let Some(reason) = current_revocation {
            let directive = match reason {
                SessionRevocation::Superseded => CloseDirective {
                    code: CLOSE_CODE_AUTH_REVOKED,
                    reason: "session_superseded",
                },
                SessionRevocation::AuthRevoked => CloseDirective {
                    code: CLOSE_CODE_AUTH_REVOKED,
                    reason: "auth_revoked",
                },
            };
            close_with_directive(ws, upgrade.role, directive).await;
            return directive.reason;
        }

        tokio::select! {
            maybe_msg = ws.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => {
                        match handle_text_frame(ws, state, upgrade, conn, &text).await {
                            Ok(Some(directive)) => {
                                close_with_directive(ws, upgrade.role, directive).await;
                                return directive.reason;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                tracing::warn!(
                                    target: "minos_backend::realtime",
                                    error = %error,
                                    device_id = %upgrade.device_id,
                                    "text frame handling failed"
                                );
                                let _ = send_error_frame(ws, conn, "internal", error.to_string()).await;
                            }
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        let _ = send_error_frame(ws, conn, "validation_format", "binary frames are not supported").await;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        touch_connection_last_seen(state, &upgrade.device_id, "ws.ping").await;
                        if ws.send(Message::Pong(payload)).await.is_err() {
                            return "write_failed";
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => return "client_close",
                    Some(Err(error)) => {
                        if websocket_read_error_is_client_reset(&error) {
                            tracing::debug!(
                                target: "minos_backend::realtime",
                                error = %error,
                                device_id = %upgrade.device_id,
                                "formal gateway websocket reset by peer"
                            );
                            return "client_reset";
                        }
                        tracing::warn!(
                            target: "minos_backend::realtime",
                            error = %error,
                            device_id = %upgrade.device_id,
                            "formal gateway websocket read failed"
                        );
                        return "read_error";
                    }
                    None => return "stream_ended",
                }
            }
            maybe_frame = push_rx.recv() => {
                let Some(frame) = maybe_frame else {
                    return "outbox_closed";
                };
                if send_server_frame(ws, &frame).await.is_err() {
                    return "write_failed";
                }
            }
            maybe_legacy_frame = legacy_outbox_rx.recv() => {
                let Some(legacy_frame) = maybe_legacy_frame else {
                    continue;
                };
                tracing::debug!(
                    target: "minos_backend::realtime",
                    device_id = %upgrade.device_id,
                    frame = ?legacy_frame,
                    "dropping legacy session-registry frame on topic gateway"
                );
            }
            changed = revocation_rx.changed() => {
                if changed.is_ok() {
                    let updated_revocation = { *revocation_rx.borrow_and_update() };
                    if let Some(reason) = updated_revocation {
                        let directive = match reason {
                            SessionRevocation::Superseded => CloseDirective {
                                code: CLOSE_CODE_AUTH_REVOKED,
                                reason: "session_superseded",
                            },
                            SessionRevocation::AuthRevoked => CloseDirective {
                                code: CLOSE_CODE_AUTH_REVOKED,
                                reason: "auth_revoked",
                            },
                        };
                        close_with_directive(ws, upgrade.role, directive).await;
                        return directive.reason;
                    }
                }
            }
        }
    }
}

async fn touch_connection_last_seen(
    state: &BackendState,
    device_id: &DeviceId,
    operation: &'static str,
) {
    if let Err(error) = crate::store::devices::touch_last_seen(
        &state.store,
        device_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    {
        tracing::debug!(
            target: "minos_backend::realtime",
            error = %error,
            device_id = %device_id,
            operation,
            "failed to touch device last_seen_at",
        );
    }
}

async fn auto_subscribe_default_topic(
    ws: &mut WebSocket,
    state: &BackendState,
    conn: &std::sync::Arc<ConnectionState>,
    upgrade: &GatewayUpgrade,
) -> Result<(), BackendError> {
    let topic = upgrade.default_topic();
    let _ = state
        .subscription_mgr
        .add_topics(conn.conn_id, std::slice::from_ref(&topic));
    replay_topic(ws, state, conn, &topic, 0).await
}

async fn handle_text_frame(
    ws: &mut WebSocket,
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    conn: &std::sync::Arc<ConnectionState>,
    text: &str,
) -> Result<Option<CloseDirective>, BackendError> {
    match serde_json::from_str::<ClientFrame>(text) {
        Ok(frame) => handle_formal_frame(ws, state, upgrade, conn, frame).await,
        Err(_) => {
            let _ = send_error_frame(
                ws,
                conn,
                "validation_format",
                "unrecognized websocket frame",
            )
            .await;
            Ok(None)
        }
    }
}

async fn handle_formal_frame(
    ws: &mut WebSocket,
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    conn: &std::sync::Arc<ConnectionState>,
    frame: ClientFrame,
) -> Result<Option<CloseDirective>, BackendError> {
    match frame {
        ClientFrame::Subscribe {
            topics,
            resume_after,
            client_request_id,
        } => {
            handle_subscribe(ws, state, conn, topics, resume_after, client_request_id).await?;
            Ok(None)
        }
        ClientFrame::Unsubscribe { topics } => {
            let parsed = topics
                .into_iter()
                .filter_map(|topic| RealtimeTopic::parse(&topic).ok())
                .collect::<Vec<_>>();
            state.subscription_mgr.remove_topics(conn.conn_id, &parsed);
            Ok(None)
        }
        ClientFrame::Ping { ts } => {
            touch_connection_last_seen(state, &upgrade.device_id, "ws.client_ping").await;
            let _ = send_server_frame(
                ws,
                &ServerFrame::Pong {
                    ts,
                    server_time_ms: chrono::Utc::now().timestamp_millis(),
                },
            )
            .await;
            Ok(None)
        }
        ClientFrame::HostCommandAck {
            command_id,
            ack_at_ms,
        } => {
            if upgrade.role != DeviceRole::AgentHost {
                let _ = send_error_frame(
                    ws,
                    conn,
                    "realtime_subscription_denied",
                    "host-only frame on client gateway",
                )
                .await;
                return Ok(None);
            }
            match host_commands::ack(&state.store, &command_id, ack_at_ms).await {
                Ok(_) => {
                    if let Err(error) = outbox_events::ack_pending_host_command_events(
                        &state.store,
                        &command_id,
                        ack_at_ms,
                    )
                    .await
                    {
                        tracing::warn!(
                            target: "minos_backend::realtime::gateway",
                            error = %error,
                            command_id,
                            "failed to ack pending host command outbox events"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::realtime::gateway",
                        error = %error,
                        command_id,
                        "failed to persist host command ack"
                    );
                }
            }
            Ok(None)
        }
        ClientFrame::HostCommandResult {
            command_id,
            status,
            result,
            error,
            finished_at_ms,
        } => {
            if upgrade.role != DeviceRole::AgentHost {
                let _ = send_error_frame(
                    ws,
                    conn,
                    "realtime_subscription_denied",
                    "host-only frame on client gateway",
                )
                .await;
                return Ok(None);
            }
            let succeeded = error.is_none()
                && matches!(
                    status.as_str(),
                    "ok" | "succeeded" | "success" | "completed"
                );
            let response_value = if succeeded { result } else { None };
            let error_value = if succeeded {
                None
            } else {
                Some(error.unwrap_or_else(|| serde_json::json!({ "status": status })))
            };
            let finish_result = host_commands::finish(
                &state.store,
                &command_id,
                if succeeded {
                    host_commands::HostCommandTerminalStatus::Succeeded
                } else {
                    host_commands::HostCommandTerminalStatus::Failed
                },
                response_value.as_ref(),
                error_value.as_ref(),
                finished_at_ms,
            )
            .await;
            match finish_result {
                Ok(_) => {
                    if let Err(error) = outbox_events::ack_pending_host_command_events(
                        &state.store,
                        &command_id,
                        finished_at_ms,
                    )
                    .await
                    {
                        tracing::warn!(
                            target: "minos_backend::realtime::gateway",
                            error = %error,
                            command_id,
                            "failed to ack pending host command outbox events"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::realtime::gateway",
                        error = %error,
                        command_id,
                        "failed to persist host command result"
                    );
                }
            }
            Ok(None)
        }
        ClientFrame::HostStreamEvent {
            topic,
            kind,
            payload,
        } => {
            if upgrade.role != DeviceRole::AgentHost {
                let _ = send_error_frame(
                    ws,
                    conn,
                    "realtime_subscription_denied",
                    "host-only frame on client gateway",
                )
                .await;
                return Ok(None);
            }
            on_host_stream_event(state, upgrade, &topic, &kind, payload).await?;
            Ok(None)
        }
        ClientFrame::HostIngestLiveBatch { batch } => {
            if upgrade.role != DeviceRole::AgentHost {
                let _ = send_error_frame(
                    ws,
                    conn,
                    "realtime_subscription_denied",
                    "host-only frame on client gateway",
                )
                .await;
                return Ok(None);
            }
            handle_host_ingest_live_batch(ws, state, upgrade, batch).await?;
            Ok(None)
        }
        ClientFrame::HostGapManifest { manifest } => {
            if upgrade.role != DeviceRole::AgentHost {
                let _ = send_error_frame(
                    ws,
                    conn,
                    "realtime_subscription_denied",
                    "host-only frame on client gateway",
                )
                .await;
                return Ok(None);
            }
            handle_host_gap_manifest(ws, state, upgrade, manifest).await?;
            Ok(None)
        }
        ClientFrame::HostIngestPullResponse { response } => {
            if upgrade.role != DeviceRole::AgentHost {
                let _ = send_error_frame(
                    ws,
                    conn,
                    "realtime_subscription_denied",
                    "host-only frame on client gateway",
                )
                .await;
                return Ok(None);
            }
            handle_host_ingest_pull_response(ws, state, upgrade, response).await?;
            Ok(None)
        }
    }
}

async fn handle_subscribe(
    ws: &mut WebSocket,
    state: &BackendState,
    conn: &std::sync::Arc<ConnectionState>,
    topics: Vec<String>,
    resume_after: Option<HashMap<String, i64>>,
    client_request_id: Option<String>,
) -> Result<(), BackendError> {
    if topics.len() > SUBSCRIBE_LIMIT_PER_REQUEST {
        let _ = send_server_frame(
            ws,
            &ServerFrame::SubscriptionLimitExceeded {
                limit: SUBSCRIBE_LIMIT_PER_REQUEST,
                current: topics.len(),
            },
        )
        .await;
        return Ok(());
    }

    let mut authorized_topics = Vec::new();
    for topic_str in &topics {
        let topic = match RealtimeTopic::parse(topic_str) {
            Ok(topic) => topic,
            Err(_) => {
                let _ = send_server_frame(
                    ws,
                    &ServerFrame::SubscriptionDenied {
                        topic: topic_str.clone(),
                        reason: SubscriptionDenied::InvalidTopic.reason().to_string(),
                    },
                )
                .await;
                continue;
            }
        };

        match auth::authorize_subscription(&state.store, &conn.principal, &topic).await {
            Ok(()) => authorized_topics.push(topic),
            Err(SubscriptionAuthError::Denied(reason)) => {
                let _ = send_server_frame(
                    ws,
                    &ServerFrame::SubscriptionDenied {
                        topic: topic_str.clone(),
                        reason: reason.reason().to_string(),
                    },
                )
                .await;
            }
            Err(SubscriptionAuthError::Internal(error)) => return Err(error),
        }
    }

    let unique_new_count = authorized_topics
        .iter()
        .filter(|topic| !conn.is_subscribed(topic))
        .count();
    let next_total = conn.topic_count().saturating_add(unique_new_count);
    if next_total > LIVE_SUBSCRIPTION_LIMIT {
        let _ = send_server_frame(
            ws,
            &ServerFrame::SubscriptionLimitExceeded {
                limit: LIVE_SUBSCRIPTION_LIMIT,
                current: next_total,
            },
        )
        .await;
        return Ok(());
    }

    let newly = state
        .subscription_mgr
        .add_topics(conn.conn_id, &authorized_topics);
    if !authorized_topics.is_empty() {
        let _ = send_server_frame(
            ws,
            &ServerFrame::SubscribeAck {
                topics: authorized_topics
                    .iter()
                    .map(RealtimeTopic::topic_string)
                    .collect(),
                client_request_id,
            },
        )
        .await;
    }

    let resume_after = resume_after.unwrap_or_default();
    for topic in newly {
        let after = resume_after
            .get(&topic.topic_string())
            .copied()
            .unwrap_or(0);
        replay_topic(ws, state, conn, &topic, after).await?;
    }

    Ok(())
}

async fn replay_topic(
    ws: &mut WebSocket,
    state: &BackendState,
    conn: &std::sync::Arc<ConnectionState>,
    topic: &RealtimeTopic,
    after: i64,
) -> Result<(), BackendError> {
    let topic_string = topic.topic_string();
    let topic_kind = topic.kind().as_str();
    let first =
        durable_event_log::read_topic_after(&state.store, topic_kind, &topic_string, 0, 1).await?;
    let retention_floor_seq = first
        .first()
        .map(|row| row.topic_seq.saturating_sub(1))
        .unwrap_or(0);
    if after < retention_floor_seq {
        let _ = send_server_frame(
            ws,
            &ServerFrame::SnapshotRequired {
                topic: topic_string,
                last_known_seq: after,
                retention_floor_seq,
            },
        )
        .await;
        return Ok(());
    }

    let mut next_after = after;
    loop {
        let batch = durable_event_log::read_topic_after(
            &state.store,
            topic_kind,
            &topic.topic_string(),
            next_after,
            REPLAY_BATCH_SIZE,
        )
        .await?;
        if batch.is_empty() {
            break;
        }

        for row in &batch {
            if conn.has_seen_durable_event(&row.event_id) {
                continue;
            }
            let (kind, payload) = durable_event_kind_payload(&row.payload_json);
            if send_server_frame(
                ws,
                &ServerFrame::DurableEvent {
                    topic: row.topic.clone(),
                    topic_seq: row.topic_seq,
                    kind,
                    payload,
                    event_id: row.event_id.clone(),
                },
            )
            .await
            .is_err()
            {
                return Err(BackendError::MessageBus {
                    operation: "gateway.replay.send".into(),
                    message: "websocket closed while replaying durable events".into(),
                });
            }
            let _ = conn.remember_durable_event(&row.event_id);
        }

        next_after = batch.last().map_or(next_after, |row| row.topic_seq);
        if batch.len() < usize::try_from(REPLAY_BATCH_SIZE).unwrap_or(usize::MAX) {
            break;
        }
    }

    Ok(())
}

async fn handle_host_ingest_live_batch(
    ws: &mut WebSocket,
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    batch: HostIngestLiveBatch,
) -> Result<(), BackendError> {
    if batch.host_id != upgrade.device_id {
        return Ok(());
    }
    let mut accepted_threads = HashSet::new();
    let batch_id = batch.batch_id.clone();
    for chunk in batch.chunks {
        let session_id = chunk.session_id.clone();
        if persist_host_ingest_chunk(state, upgrade, chunk).await? {
            accepted_threads.insert(session_id);
        }
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let host_id = upgrade.device_id.to_string();
    for session_id in accepted_threads {
        let accepted_to_seq = backend_accepted_to_seq(state, &host_id, &session_id).await?;
        thread_sync_state::mark_backend_acked(
            &state.store,
            &host_id,
            &session_id,
            accepted_to_seq,
            now_ms,
        )
        .await?;
        let _ = send_server_frame(
            ws,
            &ServerFrame::HostIngestAck {
                session_id,
                accepted_to_seq,
                batch_id: Some(batch_id.clone()),
            },
        )
        .await;
    }
    Ok(())
}

async fn handle_host_gap_manifest(
    ws: &mut WebSocket,
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    manifest: HostGapManifest,
) -> Result<(), BackendError> {
    if manifest.host_id != upgrade.device_id {
        return Ok(());
    }
    thread_sync_state::upsert_manifest(
        &state.store,
        &manifest,
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    for frame in pull_requests_for_manifest(&manifest) {
        let _ = send_server_frame(ws, &frame).await;
    }
    Ok(())
}

async fn handle_host_ingest_pull_response(
    ws: &mut WebSocket,
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    response: HostIngestPullResponse,
) -> Result<(), BackendError> {
    let request_id = response.request_id.clone();
    let session_id = response.session_id.clone();
    let host_id = upgrade.device_id.to_string();
    for chunk in response.chunks {
        persist_host_ingest_chunk(state, upgrade, chunk).await?;
    }
    let accepted_to_seq = backend_accepted_to_seq(state, &host_id, &session_id).await?;
    thread_sync_state::mark_backend_acked(
        &state.store,
        &host_id,
        &session_id,
        accepted_to_seq,
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    let _ = send_server_frame(
        ws,
        &ServerFrame::PullAck {
            request_id,
            session_id,
            accepted_to_seq,
        },
    )
    .await;
    Ok(())
}

async fn backend_accepted_to_seq(
    state: &BackendState,
    host_id: &str,
    session_id: &str,
) -> Result<u64, BackendError> {
    let current = thread_sync_state::backend_acked_seq(&state.store, host_id, session_id).await?;
    raw_events::contiguous_host_seq_after(&state.store, host_id, session_id, current).await
}

fn pull_requests_for_manifest(manifest: &HostGapManifest) -> Vec<ServerFrame> {
    let mut frames = Vec::new();
    for session in &manifest.sessions {
        let ranges = if session.missing_ranges.is_empty() {
            vec![minos_protocol::realtime::SeqRange {
                from: session.local_from_seq,
                to: session.local_to_seq,
            }]
        } else {
            session.missing_ranges.clone()
        };
        let min_from = session.backend_acked_seq.saturating_add(1);
        for range in ranges {
            let from_seq = range.from.max(min_from).max(1);
            if from_seq > range.to {
                continue;
            }
            frames.push(ServerFrame::PullIngestRange {
                request_id: Uuid::new_v4().to_string(),
                session_id: session.session_id.clone(),
                from_seq,
                to_seq: range.to,
                max_bytes: PULL_INGEST_MAX_BYTES,
                priority: PullPriority::LiveCritical,
                reason: PullReason::IdleBackfill,
            });
        }
    }
    frames
}

async fn persist_host_ingest_chunk(
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    chunk: HostIngestChunk,
) -> Result<bool, BackendError> {
    let expected_host_id = upgrade.device_id.to_string();
    let Some(session) = agent_sessions::get(&state.store, &chunk.session_id).await? else {
        tracing::warn!(
            target: "minos_backend::realtime::gateway",
            session_id = %chunk.session_id,
            "dropping host ingest chunk for unknown agent session",
        );
        return Ok(false);
    };
    match session.host_device_id.as_deref() {
        Some(host_id) if host_id != expected_host_id => {
            tracing::warn!(
                target: "minos_backend::realtime::gateway",
                session_id = %chunk.session_id,
                host_device_id = %upgrade.device_id,
                expected_host_id = host_id,
                "dropping host ingest chunk for mismatched host",
            );
            return Ok(false);
        }
        Some(_) => {}
        None => {
            agent_sessions::claim_host_if_empty(&state.store, &chunk.session_id, &expected_host_id)
                .await?;
        }
    }

    sessions::upsert(
        &state.store,
        &chunk.session_id,
        chunk.agent,
        &expected_host_id,
        chunk.last_ts_ms,
    )
    .await?;

    let inserted = raw_events::insert_host_ingest_chunk(
        &state.store,
        &expected_host_id,
        &chunk.session_id,
        chunk.seq,
        &chunk.event_id,
        &chunk.kind,
        chunk.agent,
        &chunk.payload,
        chunk.last_ts_ms,
        &chunk.checksum_sha256,
        chunk.byte_len,
    )
    .await?;

    if inserted == raw_events::HostIngestInsert::Inserted {
        fanout_host_ingest_projection(state, &chunk);
    }
    Ok(true)
}

fn fanout_host_ingest_projection(state: &BackendState, chunk: &HostIngestChunk) {
    let topic = RealtimeTopic::AgentSession(chunk.session_id.clone());
    for ui in &chunk.projection {
        let payload = match serde_json::to_value(ui) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::realtime::gateway",
                    error = %error,
                    session_id = %chunk.session_id,
                    "failed to encode host ingest projection",
                );
                continue;
            }
        };
        let frame = ServerFrame::StreamEvent {
            topic: topic.topic_string(),
            kind: "ui_event".to_string(),
            seq: i64::try_from(chunk.seq).ok(),
            payload,
        };
        for target in state.subscription_mgr.fanout_targets(&topic) {
            let _ = target.send(frame.clone());
        }
    }
}

async fn on_host_stream_event(
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    topic_str: &str,
    kind: &str,
    payload: Value,
) -> Result<(), BackendError> {
    let topic = RealtimeTopic::parse(topic_str).map_err(|error| BackendError::MessageBus {
        operation: "gateway.host_stream.parse_topic".into(),
        message: error.to_string(),
    })?;
    let RealtimeTopic::AgentSession(session_id) = &topic else {
        return Ok(());
    };

    if payload.get("version").and_then(Value::as_u64) == Some(2) {
        handle_projected_ingest_host_stream_event(state, upgrade, &topic, session_id, payload)
            .await?;
    } else if payload.get("turn_id").and_then(Value::as_str).is_some()
        && payload.get("seq").and_then(Value::as_i64).is_some()
    {
        handle_formal_host_stream_event(state, upgrade, &topic, session_id, kind, payload).await?;
    } else {
        handle_raw_ingest_host_stream_event(state, upgrade, session_id, payload).await?;
    }

    Ok(())
}

async fn handle_projected_ingest_host_stream_event(
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    topic: &RealtimeTopic,
    session_id: &str,
    payload: Value,
) -> Result<(), BackendError> {
    let seq =
        payload
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| BackendError::StoreDecode {
                column: "host_stream.payload.seq".into(),
                message: "missing seq".into(),
            })?;
    let agent = payload
        .get("agent")
        .cloned()
        .and_then(|value| serde_json::from_value::<AgentName>(value).ok())
        .unwrap_or(AgentName::Codex);
    let ts_ms = payload
        .get("ts_ms")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let projection = payload
        .get("projection")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<UiEventMessage>>(value).ok())
        .unwrap_or_default();

    let Some(session) = agent_sessions::get(&state.store, session_id).await? else {
        return Ok(());
    };
    let expected_host_id = upgrade.device_id.to_string();
    if session.host_device_id.as_deref() != Some(expected_host_id.as_str()) {
        return Ok(());
    }

    sessions::upsert(&state.store, session_id, agent, &expected_host_id, ts_ms).await?;
    let Some(persisted_seq) =
        raw_events::insert_assigning_seq(&state.store, session_id, seq, agent, &payload, ts_ms)
            .await?
    else {
        return Ok(());
    };

    for ui in projection {
        let payload = match serde_json::to_value(&ui) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::realtime::gateway",
                    error = %error,
                    session_id,
                    "failed to encode projected ui event"
                );
                continue;
            }
        };
        let frame = ServerFrame::StreamEvent {
            topic: topic.topic_string(),
            kind: "ui_event".to_string(),
            seq: i64::try_from(persisted_seq).ok(),
            payload,
        };
        for target in state.subscription_mgr.fanout_targets(topic) {
            let _ = target.send(frame.clone());
        }
    }

    Ok(())
}

async fn handle_formal_host_stream_event(
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    topic: &RealtimeTopic,
    session_id: &str,
    kind: &str,
    payload: Value,
) -> Result<(), BackendError> {
    let session = agent_sessions::get(&state.store, session_id)
        .await?
        .ok_or_else(|| BackendError::PeerOffline {
            peer_device_id: session_id.to_string(),
        })?;
    let expected_host_id = upgrade.device_id.to_string();
    if session.host_device_id.as_deref() != Some(expected_host_id.as_str()) {
        return Ok(());
    }

    let turn_id = payload
        .get("turn_id")
        .and_then(Value::as_str)
        .ok_or_else(|| BackendError::StoreDecode {
            column: "host_stream.payload.turn_id".into(),
            message: "missing turn_id".into(),
        })?;
    let event_seq =
        payload
            .get("seq")
            .and_then(Value::as_i64)
            .ok_or_else(|| BackendError::StoreDecode {
                column: "host_stream.payload.seq".into(),
                message: "missing seq".into(),
            })?;
    let turn = agent_turns::get(&state.store, turn_id)
        .await?
        .ok_or_else(|| BackendError::StoreDecode {
            column: "host_stream.payload.turn_id".into(),
            message: "turn not found".into(),
        })?;
    if turn.agent_session_id != session_id {
        return Ok(());
    }

    let created_at_ms = chrono::Utc::now().timestamp_millis();
    let _ = agent_turn_events::append(
        &state.store,
        turn_id,
        event_seq,
        kind,
        &payload,
        created_at_ms,
    )
    .await?;

    let frame = ServerFrame::StreamEvent {
        topic: topic.topic_string(),
        kind: kind.to_string(),
        seq: Some(event_seq),
        payload,
    };
    for target in state.subscription_mgr.fanout_targets(&topic) {
        let _ = target.send(frame.clone());
    }

    Ok(())
}

async fn handle_raw_ingest_host_stream_event(
    state: &BackendState,
    upgrade: &GatewayUpgrade,
    session_id: &str,
    payload: Value,
) -> Result<(), BackendError> {
    let Some(seq) = payload.get("seq").and_then(Value::as_u64) else {
        tracing::warn!(
            target: "minos_backend::realtime::gateway",
            session_id,
            "dropping raw host stream event without seq metadata"
        );
        return Ok(());
    };
    let agent = payload
        .get("agent")
        .cloned()
        .and_then(|value| serde_json::from_value::<AgentName>(value).ok())
        .unwrap_or(AgentName::Codex);
    let ts_ms = payload
        .get("ts_ms")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    let Some(session) = agent_sessions::get(&state.store, session_id).await? else {
        tracing::warn!(
            target: "minos_backend::realtime::gateway",
            session_id,
            host_device_id = %upgrade.device_id,
            "dropping raw host stream event for unknown formal session"
        );
        return Ok(());
    };
    let expected_host_id = upgrade.device_id.to_string();
    if session.host_device_id.as_deref() != Some(expected_host_id.as_str()) {
        tracing::warn!(
            target: "minos_backend::realtime::gateway",
            session_id,
            host_device_id = %upgrade.device_id,
            "dropping raw host stream event for mismatched host"
        );
        return Ok(());
    }

    ensure_raw_approval_turn(state, session_id, &payload, ts_ms).await?;

    state
        .ingest
        .execute(IngestCommand {
            agent,
            session_id: session_id.to_string(),
            seq,
            payload,
            ts_ms,
            owner_device_id: upgrade.device_id,
        })
        .await
}

async fn ensure_raw_approval_turn(
    state: &BackendState,
    session_id: &str,
    payload: &Value,
    now_ms: i64,
) -> Result<(), BackendError> {
    let Some(turn_id) = raw_approval_turn_id(payload) else {
        return Ok(());
    };

    if agent_turns::get(&state.store, turn_id).await?.is_some() {
        return Ok(());
    }

    let existing_turns =
        agent_turns::list_for_session(&state.store, session_id, None, u32::MAX).await?;
    let turn_seq = existing_turns.last().map_or(1, |turn| turn.turn_seq + 1);

    let _ = agent_turns::create(
        &state.store,
        turn_id,
        session_id,
        turn_seq,
        "assistant",
        "streaming",
        now_ms,
        None,
        None,
        None,
    )
    .await?;

    Ok(())
}

fn raw_approval_turn_id(payload: &Value) -> Option<&str> {
    if payload.get("method").and_then(Value::as_str) != Some("approval/request") {
        return None;
    }
    payload
        .get("params")
        .and_then(|params| params.get("turn_id"))
        .and_then(Value::as_str)
        .filter(|turn_id| !turn_id.is_empty())
}

fn durable_event_kind_payload(value: &Value) -> (String, Value) {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let mut payload = value.clone();
    if let Value::Object(map) = &mut payload {
        map.remove("kind");
    }
    (kind, payload)
}

fn websocket_read_error_is_client_reset(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string();
    message.contains("Connection reset without closing handshake")
        || message.contains("connection reset by peer")
        || message.contains("Broken pipe")
}

async fn send_server_frame(ws: &mut WebSocket, frame: &ServerFrame) -> Result<(), axum::Error> {
    let json = serde_json::to_string(frame).map_err(|error| axum::Error::new(error))?;
    ws.send(Message::Text(json.into())).await
}

async fn send_error_frame(
    ws: &mut WebSocket,
    conn: &ConnectionState,
    code: &str,
    message: impl Into<String>,
) -> Result<(), axum::Error> {
    send_server_frame(
        ws,
        &ServerFrame::Error {
            code: code.to_string(),
            message: message.into(),
            request_id: conn.conn_id.to_string(),
        },
    )
    .await
}

async fn close_with_directive(ws: &mut WebSocket, role: DeviceRole, directive: CloseDirective) {
    if role == DeviceRole::AgentHost {
        let _ = send_server_frame(
            ws,
            &ServerFrame::HostForceClose {
                reason: directive.reason.to_string(),
                close_code: directive.code,
            },
        )
        .await;
    }

    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code: directive.code,
            reason: directive.reason.into(),
        })))
        .await;
}

#[cfg(test)]
mod tests {
    use super::{
        pull_requests_for_manifest, websocket_read_error_is_client_reset, PULL_INGEST_MAX_BYTES,
    };
    use minos_domain::DeviceId;
    use minos_protocol::realtime::{
        HostGapManifest, PullPriority, PullReason, SeqRange, ServerFrame, SessionGapManifest,
    };

    #[test]
    fn websocket_read_error_classifies_client_reset() {
        assert!(websocket_read_error_is_client_reset(
            &"WebSocket protocol error: Connection reset without closing handshake"
        ));
        assert!(websocket_read_error_is_client_reset(
            &"io error: connection reset by peer"
        ));
        assert!(!websocket_read_error_is_client_reset(
            &"WebSocket protocol error: invalid opcode"
        ));
    }

    #[test]
    fn manifest_pull_requests_cover_reported_ranges() {
        let manifest = HostGapManifest {
            manifest_id: "manifest-1".into(),
            host_id: DeviceId::new(),
            sessions: vec![SessionGapManifest {
                session_id: "thr-sync".into(),
                backend_acked_seq: 10,
                local_from_seq: 11,
                local_to_seq: 20,
                missing_ranges: vec![SeqRange { from: 9, to: 12 }, SeqRange { from: 15, to: 16 }],
                bytes: 0,
                event_count: 0,
                first_ts_ms: 0,
                last_ts_ms: 0,
                running: true,
            }],
        };

        let frames = pull_requests_for_manifest(&manifest);

        assert_eq!(frames.len(), 2);
        let ServerFrame::PullIngestRange {
            request_id,
            session_id,
            from_seq,
            to_seq,
            max_bytes,
            priority,
            reason,
        } = &frames[0]
        else {
            panic!("expected PullIngestRange");
        };
        assert!(!request_id.is_empty());
        assert_eq!(session_id, "thr-sync");
        assert_eq!((*from_seq, *to_seq), (11, 12));
        assert_eq!(*max_bytes, PULL_INGEST_MAX_BYTES);
        assert_eq!(*priority, PullPriority::LiveCritical);
        assert_eq!(*reason, PullReason::IdleBackfill);
        let ServerFrame::PullIngestRange {
            from_seq, to_seq, ..
        } = &frames[1]
        else {
            panic!("expected PullIngestRange");
        };
        assert_eq!((*from_seq, *to_seq), (15, 16));
    }
}
