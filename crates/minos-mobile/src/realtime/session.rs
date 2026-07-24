use std::sync::Arc;

use futures_util::StreamExt;
use minos_domain::ConnectionState;
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use minos_protocol::{ChatMessageSummary, SenderType, UserSummary};
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

        // Auto-subscribe to account topic plus any topic the app requested
        // before this WebSocket was established.
        let account_topic = format!("account:{account_id}");
        subscription_mgr.add_topic(&account_topic, 0).await;
        let mut topics = subscription_mgr.subscribed_topics().await;
        topics.sort();
        topics.dedup();
        let resume_after = subscription_mgr.resume_after_map().await;
        let subscribe = ClientFrame::Subscribe {
            topics,
            resume_after: Some(resume_after),
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
                        let _ = social_events_tx.send(SocialEventFrame {
                            conversation_id: conv_id.to_string(),
                            message: parse_chat_message(payload),
                        });
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
            _ => {
                tracing::debug!(kind, topic, "unhandled stream event");
            }
        },
        RealtimeEvent::SnapshotRequired { topic, .. } => {
            tracing::warn!(topic, "snapshot required — need REST rebuild");
        }
        RealtimeEvent::SubscriptionDenied { topic, reason } => {
            tracing::warn!(topic, reason, "subscription denied");
        }
        RealtimeEvent::ForceClose { reason, close_code } => {
            tracing::warn!(reason, close_code, "force close from server");
        }
    }
}

fn parse_chat_message(payload: &serde_json::Value) -> ChatMessageSummary {
    let message_payload = payload.get("message").unwrap_or(payload);
    serde_json::from_value(message_payload.clone()).unwrap_or_else(|_| ChatMessageSummary {
        message_id: String::new(),
        conversation_id: String::new(),
        sender: UserSummary {
            account_id: String::new(),
            minos_id: String::new(),
            display_name: String::new(),
        },
        text: String::new(),
        created_at_ms: 0,
        reply_to: None,
        recalled_at_ms: None,
        mentioned_account_ids: Vec::new(),
        sender_type: SenderType::User,
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
