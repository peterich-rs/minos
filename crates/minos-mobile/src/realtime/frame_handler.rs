use minos_protocol::realtime::ServerFrame;
use minos_protocol::ChatMessageSummary;

#[derive(Debug, Clone)]
pub enum RealtimeEvent {
    DurableEvent {
        topic: String,
        topic_seq: i64,
        kind: String,
        payload: serde_json::Value,
        event_id: String,
    },
    StreamEvent {
        topic: String,
        kind: String,
        seq: Option<i64>,
        payload: serde_json::Value,
    },
    SnapshotRequired {
        topic: String,
        last_known_seq: i64,
        retention_floor_seq: i64,
    },
    SubscriptionDenied {
        topic: String,
        reason: String,
    },
    /// Gateway refused further Subscribe (topic cap 128). Surface to UI.
    SubscriptionLimitExceeded {
        limit: usize,
        current: usize,
    },
    ForceClose {
        reason: String,
        close_code: u16,
    },
    /// Gateway confirmed Subscribe for these topics.
    SubscribeAck {
        topics: Vec<String>,
    },
    /// Account AppendMessage committed (after TX success).
    ChatSendAck {
        client_operation_id: String,
        conversation_id: String,
        message_id: String,
        message_seq: i64,
        message: Option<ChatMessageSummary>,
    },
    /// Account AppendMessage rejected.
    ChatSendNack {
        client_operation_id: String,
        conversation_id: String,
        code: String,
        message: String,
    },
}

pub fn handle_server_frame(frame: ServerFrame) -> Option<RealtimeEvent> {
    match frame {
        ServerFrame::Hello { .. } => None,
        ServerFrame::SubscribeAck { topics, .. } => Some(RealtimeEvent::SubscribeAck { topics }),
        ServerFrame::SubscriptionDenied { topic, reason } => {
            tracing::warn!(topic, reason, "subscription denied");
            Some(RealtimeEvent::SubscriptionDenied { topic, reason })
        }
        ServerFrame::SubscriptionLimitExceeded { limit, current } => {
            tracing::warn!(limit, current, "subscription limit exceeded");
            Some(RealtimeEvent::SubscriptionLimitExceeded { limit, current })
        }
        ServerFrame::DurableEvent {
            topic,
            topic_seq,
            kind,
            payload,
            event_id,
        } => Some(RealtimeEvent::DurableEvent {
            topic,
            topic_seq,
            kind,
            payload,
            event_id,
        }),
        ServerFrame::StreamEvent {
            topic,
            kind,
            seq,
            payload,
        } => Some(RealtimeEvent::StreamEvent {
            topic,
            kind,
            seq,
            payload,
        }),
        ServerFrame::SnapshotRequired {
            topic,
            last_known_seq,
            retention_floor_seq,
        } => Some(RealtimeEvent::SnapshotRequired {
            topic,
            last_known_seq,
            retention_floor_seq,
        }),
        ServerFrame::HostForceClose { reason, close_code } => {
            Some(RealtimeEvent::ForceClose { reason, close_code })
        }
        ServerFrame::Pong { .. } => None,
        ServerFrame::HostIngestAck { .. }
        | ServerFrame::PullIngestRange { .. }
        | ServerFrame::PullAck { .. } => None,
        // Account collaboration ack/nack — resolved by send waiters in MobileClient.
        ServerFrame::ChatSendAck {
            client_operation_id,
            conversation_id,
            message_id,
            message_seq,
            message,
        } => Some(RealtimeEvent::ChatSendAck {
            client_operation_id,
            conversation_id,
            message_id,
            message_seq,
            message,
        }),
        ServerFrame::ChatSendNack {
            client_operation_id,
            conversation_id,
            code,
            message,
        } => Some(RealtimeEvent::ChatSendNack {
            client_operation_id,
            conversation_id,
            code,
            message,
        }),
        // Host-only mailbox frames must never arrive on /ws/client.
        ServerFrame::BotInboxDelivery { .. } | ServerFrame::CancelDelivery { .. } => None,
        ServerFrame::Error { code, message, .. } => {
            tracing::warn!(code, message, "server error frame");
            None
        }
    }
}
