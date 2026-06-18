use minos_protocol::realtime::ServerFrame;

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
    ForceClose {
        reason: String,
        close_code: u16,
    },
}

pub fn handle_server_frame(frame: ServerFrame) -> Option<RealtimeEvent> {
    match frame {
        ServerFrame::Hello { .. } => None,
        ServerFrame::SubscribeAck { .. } => None,
        ServerFrame::SubscriptionDenied { topic, reason } => {
            tracing::warn!(topic, reason, "subscription denied");
            Some(RealtimeEvent::SubscriptionDenied { topic, reason })
        }
        ServerFrame::SubscriptionLimitExceeded { limit, current } => {
            tracing::warn!(limit, current, "subscription limit exceeded");
            None
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
        ServerFrame::Error { code, message, .. } => {
            tracing::warn!(code, message, "server error frame");
            None
        }
    }
}
