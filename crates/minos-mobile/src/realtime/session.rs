use std::sync::Arc;

use futures_util::StreamExt;
use minos_domain::ConnectionState;
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use minos_protocol::ChatMessageSummary;
use minos_ui_protocol::{DisplayPayload, UiEventMessage};
use openwire_core::websocket::Message;
use tokio::sync::{broadcast, mpsc, watch};

use crate::client::{SocialEventFrame, UiEventFrame};

use super::chat_send_waiters::SharedChatSendWaiters;
use super::frame_handler::{handle_server_frame, RealtimeEvent};
use super::subscription::SubscriptionManager;

pub struct RealtimeSession;

impl RealtimeSession {
    pub async fn run(
        ws: openwire::websocket::WebSocket,
        account_id: String,
        subscription_mgr: Arc<SubscriptionManager>,
        ui_events_tx: broadcast::Sender<UiEventFrame>,
        social_events_tx: broadcast::Sender<SocialEventFrame>,
        state_tx: watch::Sender<ConnectionState>,
        mut inbound_client_frames: mpsc::Receiver<ClientFrame>,
        chat_send_waiters: SharedChatSendWaiters,
    ) {
        let (write, mut read) = ws.split();

        // Wait for Hello
        if wait_for_hello(&mut read).await.is_none() {
            let _ = state_tx.send(ConnectionState::Disconnected);
            return;
        }

        // Subscribe to account topic plus any topic the app requested before
        // this WebSocket was established. Hello is register-only on the gateway;
        // catch-up uses resume_after from persisted-in-process cursors (never
        // force account resume to 0 on reconnect).
        let account_topic = format!("account:{account_id}");
        // Desire account + any topics the app requested before this socket.
        let _ = subscription_mgr.desire_topic(&account_topic, 0).await;
        let mut topics = subscription_mgr.desired_topics().await;
        topics.sort();
        topics.dedup();
        let resume_after = subscription_mgr.resume_after_map().await;
        // Omit zero cursors so gateway treats missing keys as after=0 only when
        // truly unknown; non-zero values filter replay.
        let resume_after: std::collections::HashMap<String, i64> = resume_after
            .into_iter()
            .filter(|(_, seq)| *seq > 0)
            .collect();
        let subscribe = ClientFrame::Subscribe {
            topics: topics.clone(),
            resume_after: if resume_after.is_empty() {
                None
            } else {
                Some(resume_after)
            },
            client_request_id: None,
        };
        let subscribe_json = match serde_json::to_string(&subscribe) {
            Ok(json) => json,
            Err(_) => {
                let _ = state_tx.send(ConnectionState::Disconnected);
                return;
            }
        };
        if write.send_text(subscribe_json).await.is_err() {
            let _ = state_tx.send(ConnectionState::Disconnected);
            return;
        }
        subscription_mgr.mark_subscribe_sent(&topics).await;

        // Main loop. The optional app-to-WS command channel may be absent for
        // read-only clients; closing it must not tear down the socket.
        let mut inbound_frames_closed = false;
        loop {
            tokio::select! {
                maybe_msg = read.next() => {
                    match maybe_msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(frame) = serde_json::from_str::<ServerFrame>(text.as_ref()) {
                                if let Some(event) = handle_server_frame(frame) {
                                    dispatch_event(
                                        &event,
                                        &subscription_mgr,
                                        &ui_events_tx,
                                        &social_events_tx,
                                        &chat_send_waiters,
                                    )
                                    .await;
                                }
                            }
                        }
                        Some(Ok(Message::Close { .. })) | None => break,
                        Some(Err(_)) => break,
                        _ => {}
                    }
                }
                maybe_frame = inbound_client_frames.recv(), if !inbound_frames_closed => {
                    let Some(frame) = maybe_frame else {
                        inbound_frames_closed = true;
                        continue;
                    };
                    let json = match serde_json::to_string(&frame) {
                        Ok(j) => j,
                        Err(_) => continue,
                    };
                    if write.send_text(json).await.is_err() {
                        break;
                    }
                }
            }
        }

        // Drop pending AppendMessage waiters so outbox can retry WS later.
        chat_send_waiters.fail_all_socket();
        subscription_mgr.on_disconnect().await;
        let _ = state_tx.send(ConnectionState::Disconnected);
    }
}

async fn wait_for_hello<S>(read: &mut S) -> Option<()>
where
    S: StreamExt<Item = Result<Message, openwire_core::websocket::WebSocketError>> + Unpin,
{
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(ServerFrame::Hello {
                    conn_id,
                    heartbeat_interval_ms,
                    ..
                }) = serde_json::from_str::<ServerFrame>(text.as_ref())
                {
                    tracing::info!(
                        conn_id,
                        heartbeat_interval_ms,
                        "realtime session established"
                    );
                    return Some(());
                }
            }
            _ => return None,
        }
    }
    None
}

/// Outcome of applying a durable event into the client pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyOutcome {
    /// Parse failed or delivery failed — do not advance cursor.
    Hold,
    /// Intentional no-op / UI-only fanout complete — advance now.
    AdvanceNow,
    /// Social frame delivered to Dart; advance only after `ack_durable_applied`.
    AwaitDartAck,
}

fn send_social(
    social_events_tx: &broadcast::Sender<SocialEventFrame>,
    mut frame: SocialEventFrame,
    topic: &str,
    topic_seq: i64,
) -> ApplyOutcome {
    frame.topic = topic.to_string();
    frame.topic_seq = topic_seq;
    match social_events_tx.send(frame) {
        Ok(n) if n > 0 => ApplyOutcome::AwaitDartAck,
        Ok(_) => {
            tracing::warn!(
                topic,
                topic_seq,
                "social durable event has no subscriber; hold cursor"
            );
            ApplyOutcome::Hold
        }
        Err(_) => {
            // broadcast::error::SendError when no receivers (channel closed).
            tracing::warn!(topic, topic_seq, "social broadcast closed; hold cursor");
            ApplyOutcome::Hold
        }
    }
}

/// Apply a durable event.
///
/// Social kinds: fan out with topic/topic_seq; cursor advances only when Dart
/// calls `ack_durable_applied` after cache commit.
/// UI-only / unknown: advance after successful local fanout (or intentional no-op).
fn apply_durable_event(
    topic: &str,
    topic_seq: i64,
    kind: &str,
    payload: &serde_json::Value,
    ui_events_tx: &broadcast::Sender<UiEventFrame>,
    social_events_tx: &broadcast::Sender<SocialEventFrame>,
) -> ApplyOutcome {
    match kind {
        "conversation_message_appended" | "conversation_message_recalled" => {
            let Some(message) = parse_chat_message(payload) else {
                tracing::warn!(kind, topic, "parse_chat_message failed; hold cursor");
                return ApplyOutcome::Hold;
            };
            let conv_id = message.conversation_id.clone();
            send_social(
                social_events_tx,
                SocialEventFrame {
                    conversation_id: conv_id,
                    kind: "message".into(),
                    message,
                    topic: String::new(),
                    topic_seq: 0,
                },
                topic,
                topic_seq,
            )
        }
        "account_conversation_message_appended" => {
            let Some(frame) = parse_account_inbox_digest(payload, false) else {
                tracing::warn!(kind, topic, "parse inbox digest failed; hold cursor");
                return ApplyOutcome::Hold;
            };
            send_social(social_events_tx, frame, topic, topic_seq)
        }
        "account_conversation_message_recalled" => {
            let Some(frame) = parse_account_inbox_digest(payload, true) else {
                tracing::warn!(kind, topic, "parse inbox recall failed; hold cursor");
                return ApplyOutcome::Hold;
            };
            send_social(social_events_tx, frame, topic, topic_seq)
        }
        "conversation_message_reaction_updated" => {
            let Some(frame) = parse_reaction_updated(payload) else {
                tracing::warn!(kind, topic, "parse reaction_updated failed; hold cursor");
                return ApplyOutcome::Hold;
            };
            send_social(social_events_tx, frame, topic, topic_seq)
        }
        "approval_requested" | "approval_resolved" => {
            let _ = ui_events_tx.send(UiEventFrame {
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                seq: 0,
                ui: UiEventMessage::Raw {
                    kind: kind.to_string(),
                    payload_json: payload.to_string(),
                },
                ts_ms: chrono::Utc::now().timestamp_millis(),
            });
            ApplyOutcome::AdvanceNow
        }
        "host_linked" | "host_unlinked" | "friend_request_updated" => {
            let _ = ui_events_tx.send(UiEventFrame {
                session_id: String::new(),
                seq: 0,
                ui: UiEventMessage::Raw {
                    kind: kind.to_string(),
                    payload_json: payload.to_string(),
                },
                ts_ms: chrono::Utc::now().timestamp_millis(),
            });
            ApplyOutcome::AdvanceNow
        }
        _ => {
            tracing::debug!(kind, topic, "unhandled durable event (consumed)");
            ApplyOutcome::AdvanceNow
        }
    }
}

async fn surface_snapshot_required(
    topic: &str,
    last_known_seq: i64,
    ui_events_tx: &broadcast::Sender<UiEventFrame>,
    subscription_mgr: &Arc<SubscriptionManager>,
) {
    subscription_mgr.clear_cursor(topic).await;
    let _ = ui_events_tx.send(UiEventFrame {
        session_id: String::new(),
        seq: 0,
        ui: UiEventMessage::Raw {
            kind: "snapshot_required".into(),
            payload_json: serde_json::json!({
                "topic": topic,
                "last_known_seq": last_known_seq,
            })
            .to_string(),
        },
        ts_ms: chrono::Utc::now().timestamp_millis(),
    });
}

async fn dispatch_event(
    event: &RealtimeEvent,
    subscription_mgr: &Arc<SubscriptionManager>,
    ui_events_tx: &broadcast::Sender<UiEventFrame>,
    social_events_tx: &broadcast::Sender<SocialEventFrame>,
    chat_send_waiters: &SharedChatSendWaiters,
) {
    match event {
        RealtimeEvent::SubscribeAck { topics } => {
            subscription_mgr.mark_subscribe_acked(topics).await;
        }
        RealtimeEvent::ChatSendAck {
            client_operation_id,
            conversation_id,
            message_id,
            message_seq,
            message,
        } => {
            chat_send_waiters.resolve_ack(
                client_operation_id,
                conversation_id.clone(),
                message_id.clone(),
                *message_seq,
                message.clone(),
            );
        }
        RealtimeEvent::ChatSendNack {
            client_operation_id,
            conversation_id,
            code,
            message,
        } => {
            chat_send_waiters.resolve_nack(
                client_operation_id,
                conversation_id.clone(),
                code.clone(),
                message.clone(),
            );
        }
        RealtimeEvent::DurableEvent {
            topic,
            topic_seq,
            kind,
            payload,
            ..
        } => {
            // Continuity: never silently jump past a hole.
            let applied = subscription_mgr.applied_seq(topic).await;
            if applied > 0 && *topic_seq > applied + 1 {
                tracing::warn!(
                    topic,
                    topic_seq,
                    expected = applied + 1,
                    "durable seq hole; requesting snapshot"
                );
                surface_snapshot_required(topic, applied, ui_events_tx, subscription_mgr).await;
                return;
            }

            let outcome = apply_durable_event(
                topic,
                *topic_seq,
                kind,
                payload,
                ui_events_tx,
                social_events_tx,
            );
            match outcome {
                ApplyOutcome::AdvanceNow => {
                    // Do not leapfrog a smaller seq held for Dart apply.
                    if subscription_mgr.has_pending_hold(topic).await {
                        tracing::debug!(
                            topic,
                            topic_seq,
                            "skip AdvanceNow while topic has pending Dart hold"
                        );
                    } else if let crate::realtime::subscription::CursorAdvance::Hole { expected } =
                        subscription_mgr.update_seq(topic, *topic_seq).await
                    {
                        tracing::warn!(
                            topic,
                            topic_seq,
                            expected,
                            "durable seq hole on AdvanceNow; requesting snapshot"
                        );
                        surface_snapshot_required(
                            topic,
                            expected - 1,
                            ui_events_tx,
                            subscription_mgr,
                        )
                        .await;
                    }
                }
                ApplyOutcome::AwaitDartAck => {
                    subscription_mgr.mark_pending_hold(topic, *topic_seq).await;
                }
                ApplyOutcome::Hold => {}
            }
        }
        RealtimeEvent::StreamEvent {
            topic,
            kind,
            seq,
            payload,
            ..
        } => match kind.as_str() {
            "agent_text_delta"
            | "agent_text_replace"
            | "agent_reasoning_delta"
            | "agent_reasoning_replace"
            | "agent_tool_call"
            | "agent_tool_result"
            | "agent_tool_completed"
            | "agent_error"
            | "ui_event" => {
                let _ = ui_events_tx.send(UiEventFrame {
                    session_id: topic
                        .strip_prefix("agent_session:")
                        .unwrap_or(topic)
                        .to_string(),
                    seq: seq.and_then(|value| u64::try_from(value).ok()).unwrap_or(0),
                    ui: stream_event_to_ui(kind, payload),
                    ts_ms: chrono::Utc::now().timestamp_millis(),
                });
            }
            "presence" => {
                // IM presence: host device online (account:{id} topic) or
                // peer account-client online (host:{id}). Surface as Raw UI
                // event so Flutter can patch host lists without a full refresh.
                tracing::info!(
                    topic,
                    online = payload.get("online").and_then(|v| v.as_bool()),
                    device_id = payload
                        .get("device_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    principal_kind = payload
                        .get("principal_kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    "presence stream event"
                );
                let _ = ui_events_tx.send(UiEventFrame {
                    session_id: String::new(),
                    seq: 0,
                    ui: UiEventMessage::Raw {
                        kind: "presence".into(),
                        payload_json: payload.to_string(),
                    },
                    ts_ms: chrono::Utc::now().timestamp_millis(),
                });
            }
            _ => {
                tracing::debug!(kind, topic, "unhandled stream event");
            }
        },
        RealtimeEvent::SnapshotRequired { topic, .. } => {
            tracing::warn!(topic, "snapshot required — need REST rebuild");
            // Keep topic registered; reset cursor so next Subscribe catch-up is clean.
            let mgr = subscription_mgr.clone();
            let topic_clear = topic.clone();
            tokio::spawn(async move {
                mgr.clear_cursor(&topic_clear).await;
            });
            // Surface as Raw UI so Flutter can invalidate caches and re-query.
            let _ = ui_events_tx.send(UiEventFrame {
                session_id: String::new(),
                seq: 0,
                ui: UiEventMessage::Raw {
                    kind: "snapshot_required".into(),
                    payload_json: serde_json::json!({ "topic": topic }).to_string(),
                },
                ts_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        RealtimeEvent::SubscriptionDenied { topic, reason } => {
            tracing::warn!(topic, reason, "subscription denied");
            subscription_mgr.mark_subscription_denied(topic).await;
            let _ = ui_events_tx.send(UiEventFrame {
                session_id: String::new(),
                seq: 0,
                ui: UiEventMessage::Raw {
                    kind: "subscription_denied".into(),
                    payload_json: serde_json::json!({
                        "topic": topic,
                        "reason": reason,
                    })
                    .to_string(),
                },
                ts_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        RealtimeEvent::SubscriptionLimitExceeded { limit, current } => {
            tracing::warn!(limit, current, "subscription limit exceeded");
            let _ = ui_events_tx.send(UiEventFrame {
                session_id: String::new(),
                seq: 0,
                ui: UiEventMessage::Raw {
                    kind: "subscription_limit_exceeded".into(),
                    payload_json: serde_json::json!({
                        "limit": limit,
                        "current": current,
                    })
                    .to_string(),
                },
                ts_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        RealtimeEvent::ForceClose { reason, close_code } => {
            tracing::warn!(reason, close_code, "force close from server");
        }
    }
}

/// Parse durable chat payload into a real summary. Returns `None` on
/// deserialize failure or empty ids — callers must not insert shells.
fn parse_chat_message(payload: &serde_json::Value) -> Option<ChatMessageSummary> {
    let message_payload = payload.get("message").unwrap_or(payload);
    match serde_json::from_value::<ChatMessageSummary>(message_payload.clone()) {
        Ok(msg) if !msg.message_id.trim().is_empty() && !msg.conversation_id.trim().is_empty() => {
            Some(msg)
        }
        Ok(_) => {
            tracing::warn!("parse_chat_message: empty message_id or conversation_id; drop");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "parse_chat_message failed; drop (no empty shell)");
            None
        }
    }
}

/// Parse account thin digest into an inbox-only SocialEventFrame.
///
/// Builds a stub [`ChatMessageSummary`] (`text` = preview) so existing Dart
/// inbox patch paths can reuse preview/unread without treating this as
/// timeline-authoritative (kind is `inbox_digest` / `inbox_recall`).
fn parse_account_inbox_digest(
    payload: &serde_json::Value,
    is_recall: bool,
) -> Option<SocialEventFrame> {
    use minos_protocol::MessageSender;

    let conversation_id = payload
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let message_id = payload
        .get("message_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let at_ms = payload
        .get("at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let preview = if is_recall {
        payload
            .get("preview")
            .and_then(|v| v.as_str())
            .unwrap_or("Message recalled")
            .to_string()
    } else {
        payload
            .get("preview")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let sender_display_name = payload
        .get("sender_display_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let account_id = payload
        .get("account_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mentioned = payload
        .get("mentioned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let message_seq = payload
        .get("message_seq")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // SenderRef on durable: `{ kind: user|agent, account_id|agent_id }`.
    let sender = match payload.get("sender") {
        Some(s)
            if s.get("agent_id").is_some()
                || s.get("kind").and_then(|k| k.as_str()) == Some("agent") =>
        {
            let bot_id = s
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            MessageSender::Bot {
                bot_id,
                display_name: sender_display_name,
                runtime_agent: String::new(),
                name: None,
                avatar_url: None,
            }
        }
        Some(s) => MessageSender::Account {
            account_id: s
                .get("account_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            minos_id: String::new(),
            display_name: sender_display_name,
        },
        None => MessageSender::Account {
            account_id: String::new(),
            minos_id: String::new(),
            display_name: sender_display_name,
        },
    };
    let sender_type = ChatMessageSummary::sender_type_from(&sender);

    // When mentioned=true for this account, seed mentioned_account_ids so
    // inbox mention badge can flip without full message body.
    let mentioned_account_ids = if mentioned && !account_id.is_empty() {
        vec![account_id]
    } else {
        Vec::new()
    };

    Some(SocialEventFrame {
        conversation_id: conversation_id.clone(),
        kind: if is_recall {
            "inbox_recall".into()
        } else {
            "inbox_digest".into()
        },
        message: ChatMessageSummary {
            message_id,
            conversation_id,
            sender,
            text: preview,
            created_at_ms: at_ms,
            message_seq,
            reply_to: None,
            recalled_at_ms: if is_recall { Some(at_ms) } else { None },
            mentioned_account_ids,
            mentioned_agent_ids: vec![],
            sender_type,
            reactions: vec![],
            attachments: vec![],
        },
        topic: String::new(),
        topic_seq: 0,
    })
}

/// Parse reaction durable into a reaction_updated social frame.
fn parse_reaction_updated(payload: &serde_json::Value) -> Option<SocialEventFrame> {
    use minos_protocol::{MessageSender, ReactionGroup};

    let conversation_id = payload
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let message_id = payload
        .get("message_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let at_ms = payload
        .get("at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let reactions: Vec<ReactionGroup> = payload
        .get("reactions")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    Some(SocialEventFrame {
        conversation_id: conversation_id.clone(),
        kind: "reaction_updated".into(),
        message: {
            let sender = MessageSender::Account {
                account_id: String::new(),
                minos_id: String::new(),
                display_name: String::new(),
            };
            ChatMessageSummary {
                message_id,
                conversation_id,
                sender: sender.clone(),
                text: String::new(),
                created_at_ms: at_ms,
                message_seq: 0,
                reply_to: None,
                recalled_at_ms: None,
                mentioned_account_ids: vec![],
                mentioned_agent_ids: vec![],
                sender_type: ChatMessageSummary::sender_type_from(&sender),
                reactions,
                attachments: vec![],
            }
        },
        topic: String::new(),
        topic_seq: 0,
    })
}

fn stream_event_to_ui(kind: &str, payload: &serde_json::Value) -> UiEventMessage {
    let message_id = payload
        .get("message_id")
        .or_else(|| payload.get("msg_id"))
        .or_else(|| payload.get("turn_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("agent_message")
        .to_string();
    match kind {
        "ui_event" => {
            serde_json::from_value::<UiEventMessage>(payload.clone()).unwrap_or_else(|_| {
                UiEventMessage::Raw {
                    kind: kind.to_string(),
                    payload_json: payload.to_string(),
                }
            })
        }
        "agent_text_delta" => event_text(payload)
            .map(|text| UiEventMessage::TextDelta {
                message_id,
                text: DisplayPayload::inline(text),
            })
            .unwrap_or_else(|| UiEventMessage::Raw {
                kind: kind.to_string(),
                payload_json: payload.to_string(),
            }),
        "agent_text_replace" => event_text(payload)
            .map(|text| UiEventMessage::TextReplace {
                message_id,
                text: DisplayPayload::inline(text),
            })
            .unwrap_or_else(|| UiEventMessage::Raw {
                kind: kind.to_string(),
                payload_json: payload.to_string(),
            }),
        "agent_reasoning_delta" => event_text(payload)
            .map(|text| UiEventMessage::ReasoningDelta {
                message_id,
                text: DisplayPayload::inline(text),
            })
            .unwrap_or_else(|| UiEventMessage::Raw {
                kind: kind.to_string(),
                payload_json: payload.to_string(),
            }),
        "agent_reasoning_replace" => event_text(payload)
            .map(|text| UiEventMessage::ReasoningReplace {
                message_id,
                text: DisplayPayload::inline(text),
            })
            .unwrap_or_else(|| UiEventMessage::Raw {
                kind: kind.to_string(),
                payload_json: payload.to_string(),
            }),
        "agent_tool_call" => UiEventMessage::ToolCallPlaced {
            message_id,
            tool_call_id: payload
                .get("tool_call_id")
                .or_else(|| payload.get("id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool_call")
                .to_string(),
            name: payload
                .get("name")
                .or_else(|| payload.get("tool_name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool")
                .to_string(),
            args_json: DisplayPayload::inline(
                payload
                    .get("args_json")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or_else(|| payload.get("args").map(serde_json::Value::to_string))
                    .unwrap_or_else(|| "{}".into()),
            ),
        },
        "agent_tool_result" | "agent_tool_completed" => UiEventMessage::ToolCallCompleted {
            tool_call_id: payload
                .get("tool_call_id")
                .or_else(|| payload.get("id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool_call")
                .to_string(),
            output: DisplayPayload::inline(
                payload
                    .get("output")
                    .or_else(|| payload.get("result"))
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string())
                    })
                    .unwrap_or_default(),
            ),
            is_error: payload
                .get("is_error")
                .or_else(|| payload.get("error"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        },
        "agent_error" => UiEventMessage::Error {
            code: payload
                .get("code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("agent_error")
                .to_string(),
            message: payload
                .get("message")
                .or_else(|| payload.get("detail"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("agent error")
                .to_string(),
            message_id: Some(message_id),
        },
        _ => UiEventMessage::Raw {
            kind: kind.to_string(),
            payload_json: payload.to_string(),
        },
    }
}

fn event_text(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("text")
        .or_else(|| payload.get("delta"))
        .or_else(|| payload.get("content"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_durable_holds_cursor_on_parse_failure() {
        let (ui_tx, _) = broadcast::channel(8);
        let (social_tx, _) = broadcast::channel(8);
        assert_eq!(
            apply_durable_event(
                "conversation:c1",
                1,
                "conversation_message_appended",
                &serde_json::json!({}),
                &ui_tx,
                &social_tx,
            ),
            ApplyOutcome::Hold
        );
    }

    #[test]
    fn apply_durable_advances_on_unknown_kind() {
        let (ui_tx, _) = broadcast::channel(8);
        let (social_tx, _) = broadcast::channel(8);
        assert_eq!(
            apply_durable_event(
                "account:a1",
                1,
                "future_kind_xyz",
                &serde_json::json!({}),
                &ui_tx,
                &social_tx,
            ),
            ApplyOutcome::AdvanceNow
        );
    }

    #[test]
    fn apply_social_awaits_dart_ack_when_subscriber_present() {
        let (ui_tx, _) = broadcast::channel(8);
        let (social_tx, mut social_rx) = broadcast::channel(8);
        let msg = serde_json::json!({
            "message": {
                "message_id": "m1",
                "conversation_id": "c1",
                "text": "hello",
                "created_at_ms": 10,
                "message_seq": 3,
                "sender": {
                    "kind": "account",
                    "account_id": "a",
                    "minos_id": "m",
                    "display_name": "n"
                },
                "sender_type": "user",
                "mentioned_account_ids": []
            }
        });
        assert_eq!(
            apply_durable_event(
                "conversation:c1",
                7,
                "conversation_message_appended",
                &msg,
                &ui_tx,
                &social_tx,
            ),
            ApplyOutcome::AwaitDartAck
        );
        let frame = social_rx.try_recv().unwrap();
        assert_eq!(frame.topic, "conversation:c1");
        assert_eq!(frame.topic_seq, 7);
    }

    #[test]
    fn parse_chat_message_rejects_empty_shell() {
        assert!(parse_chat_message(&serde_json::json!({})).is_none());
        assert!(parse_chat_message(&serde_json::json!({
            "message_id": "",
            "conversation_id": "c1",
            "text": "x",
            "created_at_ms": 1,
            "message_seq": 1,
            "sender": {
                "kind": "account",
                "account_id": "a",
                "minos_id": "m",
                "display_name": "n"
            },
            "sender_type": "user",
            "mentioned_account_ids": []
        }))
        .is_none());
    }

    #[test]
    fn parse_chat_message_accepts_valid_payload() {
        let msg = parse_chat_message(&serde_json::json!({
            "message": {
                "message_id": "m1",
                "conversation_id": "c1",
                "text": "hello",
                "created_at_ms": 10,
                "message_seq": 3,
                "sender": {
                    "kind": "account",
                    "account_id": "a",
                    "minos_id": "m",
                    "display_name": "n"
                },
                "sender_type": "user",
                "mentioned_account_ids": []
            }
        }))
        .expect("valid message");
        assert_eq!(msg.message_id, "m1");
        assert_eq!(msg.conversation_id, "c1");
        assert_eq!(msg.message_seq, 3);
    }

    #[test]
    fn parse_account_inbox_digest_builds_preview_stub() {
        let frame = parse_account_inbox_digest(
            &serde_json::json!({
                "account_id": "viewer",
                "conversation_id": "c1",
                "message_id": "m1",
                "at_ms": 99,
                "preview": "hi digest",
                "sender_display_name": "Other",
                "mentioned": true,
                "message_seq": 7,
                "sender": { "kind": "user", "account_id": "other" }
            }),
            false,
        )
        .expect("digest");
        assert_eq!(frame.kind, "inbox_digest");
        assert_eq!(frame.conversation_id, "c1");
        assert_eq!(frame.message.text, "hi digest");
        assert_eq!(frame.message.sender.display_name(), "Other");
        assert_eq!(frame.message.sender.account_id(), Some("other"));
        assert_eq!(frame.message.message_seq, 7);
        assert_eq!(
            frame.message.mentioned_account_ids,
            vec!["viewer".to_string()]
        );
        assert!(frame.message.recalled_at_ms.is_none());
    }

    #[test]
    fn parse_account_inbox_recall_sets_kind_and_recalled() {
        let frame = parse_account_inbox_digest(
            &serde_json::json!({
                "account_id": "viewer",
                "conversation_id": "c1",
                "message_id": "m1",
                "at_ms": 50,
                "preview": "Message recalled",
                "message_seq": 2
            }),
            true,
        )
        .expect("recall digest");
        assert_eq!(frame.kind, "inbox_recall");
        assert_eq!(frame.message.recalled_at_ms, Some(50));
        assert_eq!(frame.message.text, "Message recalled");
    }

    #[test]
    fn stream_event_to_ui_maps_formal_text_delta() {
        let ui = stream_event_to_ui(
            "agent_text_delta",
            &serde_json::json!({
                "turn_id": "turn-1",
                "message_id": "msg-1",
                "delta": "hello"
            }),
        );

        assert!(matches!(
            ui,
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "msg-1" && text == "hello"
        ));
    }

    #[test]
    fn stream_event_to_ui_maps_formal_ui_event() {
        let ui = stream_event_to_ui(
            "ui_event",
            &serde_json::json!({
                "kind": "tool_call_placed",
                "message_id": "msg-1",
                "tool_call_id": "tool-1",
                "name": "shell",
                "args_json": {
                    "kind": "inline",
                    "text": "{}"
                }
            }),
        );

        assert!(matches!(
            ui,
            UiEventMessage::ToolCallPlaced {
                message_id,
                tool_call_id,
                name,
                ..
            } if message_id == "msg-1" && tool_call_id == "tool-1" && name == "shell"
        ));
    }
}
