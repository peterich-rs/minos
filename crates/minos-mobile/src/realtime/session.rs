use std::sync::Arc;

use futures_util::StreamExt;
use minos_domain::ConnectionState;
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use minos_protocol::{ChatMessageSummary, SenderType, UserSummary};
use minos_ui_protocol::UiEventMessage;
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

        // Auto-subscribe to account topic
        let account_topic = format!("account:{account_id}");
        let resume_after = subscription_mgr.resume_after_map().await;
        let subscribe = ClientFrame::Subscribe {
            topics: vec![account_topic.clone()],
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
        subscription_mgr.add_topic(&account_topic, 0).await;

        // Main loop
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
                maybe_frame = inbound_client_frames.recv() => {
                    let Some(frame) = maybe_frame else { break };
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
                "conversation_message_appended" => {
                    if let Some(conv_id) = payload.get("conversation_id").and_then(|v| v.as_str()) {
                        let _ = social_events_tx.send(SocialEventFrame {
                            conversation_id: conv_id.to_string(),
                            message: parse_chat_message(payload),
                        });
                    }
                }
                "approval_requested" | "approval_resolved" => {
                    let _ = ui_events_tx.send(UiEventFrame {
                        thread_id: payload
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
            payload,
            ..
        } => match kind.as_str() {
            "agent_text_delta" | "agent_tool_call" | "agent_error" => {
                let _ = ui_events_tx.send(UiEventFrame {
                    thread_id: topic
                        .strip_prefix("agent_session:")
                        .unwrap_or(topic)
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
    serde_json::from_value(payload.clone()).unwrap_or_else(|_| ChatMessageSummary {
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
