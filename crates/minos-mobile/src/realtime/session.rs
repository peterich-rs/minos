use std::sync::Arc;

use futures_util::StreamExt;
use minos_domain::ConnectionState;
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use minos_protocol::ChatMessageSummary;
use minos_ui_protocol::{DisplayPayload, UiEventMessage};
use openwire_core::websocket::Message;
use tokio::sync::{broadcast, mpsc, watch};

use crate::client::{SocialEventFrame, UiEventFrame};

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
        // Only seed the topic if absent; keep existing seq for reconnect catch-up.
        subscription_mgr.add_topic(&account_topic, 0).await;
        let mut topics = subscription_mgr.subscribed_topics().await;
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
            topics,
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
                                    );
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

fn dispatch_event(
    event: &RealtimeEvent,
    subscription_mgr: &Arc<SubscriptionManager>,
    ui_events_tx: &broadcast::Sender<UiEventFrame>,
    social_events_tx: &broadcast::Sender<SocialEventFrame>,
) {
    match event {
        RealtimeEvent::DurableEvent {
            topic,
            topic_seq,
            kind,
            payload,
            ..
        } => {
            let subscription_mgr = Arc::clone(subscription_mgr);
            let topic = topic.clone();
            let topic_for_update = topic.clone();
            let topic_seq = *topic_seq;
            tokio::spawn(async move {
                subscription_mgr
                    .update_seq(&topic_for_update, topic_seq)
                    .await;
            });
            match kind.as_str() {
                "conversation_message_appended"
                | "conversation_message_recalled"
                | "account_conversation_message_appended"
                | "account_conversation_message_recalled" => {
                    if let Some(conv_id) = payload.get("conversation_id").and_then(|v| v.as_str()) {
                        // Fail closed: never fan out empty-shell ChatMessageSummary.
                        if let Some(message) = parse_chat_message(payload) {
                            let _ = social_events_tx.send(SocialEventFrame {
                                conversation_id: conv_id.to_string(),
                                kind: "message".into(),
                                message,
                            });
                        }
                    }
                }
                "conversation_message_reaction_updated" => {
                    if let Some(frame) = parse_reaction_updated(payload) {
                        let _ = social_events_tx.send(frame);
                    }
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
                            kind: kind.clone(),
                            payload_json: payload.to_string(),
                        },
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
                _ => {
                    tracing::debug!(kind, topic, "unhandled durable event");
                }
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
                    installation_id = payload
                        .get("installation_id")
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
        Ok(msg)
            if !msg.message_id.trim().is_empty() && !msg.conversation_id.trim().is_empty() =>
        {
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

/// Parse reaction durable into a reaction_updated social frame.
fn parse_reaction_updated(payload: &serde_json::Value) -> Option<SocialEventFrame> {
    use minos_protocol::{ReactionGroup, SenderType, UserSummary};

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
        message: ChatMessageSummary {
            message_id,
            conversation_id,
            sender: UserSummary {
                account_id: String::new(),
                minos_id: String::new(),
                display_name: String::new(),
            },
            text: String::new(),
            created_at_ms: at_ms,
            message_seq: 0,
            reply_to: None,
            recalled_at_ms: None,
            mentioned_account_ids: vec![],
            sender_type: SenderType::User,
            reactions,
        },
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
    fn parse_chat_message_rejects_empty_shell() {
        assert!(parse_chat_message(&serde_json::json!({})).is_none());
        assert!(parse_chat_message(&serde_json::json!({
            "message_id": "",
            "conversation_id": "c1",
            "text": "x",
            "created_at_ms": 1,
            "message_seq": 1,
            "sender": {
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
