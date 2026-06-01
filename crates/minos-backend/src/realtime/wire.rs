use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Subscribe {
        topics: Vec<String>,
        resume_after: Option<HashMap<String, i64>>,
        client_request_id: Option<String>,
    },
    Unsubscribe {
        topics: Vec<String>,
    },
    Ping {
        ts: i64,
    },
    HostCommandAck {
        command_id: String,
        ack_at_ms: i64,
    },
    HostCommandResult {
        command_id: String,
        status: String,
        result: Option<Value>,
        error: Option<Value>,
        finished_at_ms: i64,
    },
    HostStreamEvent {
        topic: String,
        kind: String,
        payload: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello {
        conn_id: String,
        server_time_ms: i64,
        heartbeat_interval_ms: i64,
    },
    SubscribeAck {
        topics: Vec<String>,
        client_request_id: Option<String>,
    },
    SubscriptionDenied {
        topic: String,
        reason: String,
    },
    SubscriptionLimitExceeded {
        limit: usize,
        current: usize,
    },
    DurableEvent {
        topic: String,
        topic_seq: i64,
        kind: String,
        payload: Value,
        event_id: String,
    },
    StreamEvent {
        topic: String,
        kind: String,
        seq: Option<i64>,
        payload: Value,
    },
    SnapshotRequired {
        topic: String,
        last_known_seq: i64,
        retention_floor_seq: i64,
    },
    HostForceClose {
        reason: String,
        close_code: u16,
    },
    Pong {
        ts: i64,
        server_time_ms: i64,
    },
    Error {
        code: String,
        message: String,
        request_id: String,
    },
}