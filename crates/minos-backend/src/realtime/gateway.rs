use std::collections::HashMap;

use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::Response,
};
use futures::StreamExt;
use minos_domain::{DeviceId, DeviceRole};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::Instrument;
use uuid::Uuid;

use crate::auth::realtime_ticket::RealtimeTicketConsumeError;
use crate::error::BackendError;
use crate::http::BackendState;
use crate::realtime::auth::{self, SubscriptionAuthError, SubscriptionDenied};
use crate::realtime::subscription::ConnectionState;
use crate::session::{ServerFrame as LegacySessionFrame, SessionHandle, SessionRevocation};
use crate::store::{
    agent_sessions, agent_turn_events, agent_turns, durable_event_log, host_commands,
};
use minos_protocol::realtime::{ClientFrame, ConnectionPrincipal, RealtimeTopic, ServerFrame};

const PUSH_CHANNEL_CAPACITY: usize = 256;
const SUBSCRIBE_LIMIT_PER_REQUEST: usize = 32;
const LIVE_SUBSCRIPTION_LIMIT: usize = 128;
const REPLAY_BATCH_SIZE: u32 = 256;
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
                        if ws.send(Message::Pong(payload)).await.is_err() {
                            return "write_failed";
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => return "client_close",
                    Some(Err(error)) => {
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
            let _ = host_commands::ack(&state.store, &command_id, ack_at_ms).await?;
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
            let _ = host_commands::finish(
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
            .await?;
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
    if !newly.is_empty() {
        let _ = send_server_frame(
            ws,
            &ServerFrame::SubscribeAck {
                topics: newly.iter().map(RealtimeTopic::topic_string).collect(),
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
            if !conn.remember_durable_event(&row.event_id) {
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
        }

        next_after = batch.last().map_or(next_after, |row| row.topic_seq);
        if batch.len() < usize::try_from(REPLAY_BATCH_SIZE).unwrap_or(usize::MAX) {
            break;
        }
    }

    Ok(())
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
    let RealtimeTopic::AgentSession(session_id) = topic.clone() else {
        return Ok(());
    };

    let session = agent_sessions::get(&state.store, &session_id)
        .await?
        .ok_or_else(|| BackendError::PeerOffline {
            peer_device_id: session_id.clone(),
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
