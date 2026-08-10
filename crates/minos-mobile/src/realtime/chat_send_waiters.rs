//! Waiter registry for Account WS `ChatSendAck` / `ChatSendNack`.
//!
//! Mirrors Desktop `hub-realtime.ts` append waiters: success is only after
//! Hub commit ack (or timeout / socket miss), never mere outbound enqueue.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use minos_protocol::ChatMessageSummary;
use tokio::sync::oneshot;

/// Result of waiting for ChatSendAck/Nack after AppendMessage.
#[derive(Debug, Clone)]
pub enum ChatSendWaitResult {
    Ack {
        conversation_id: String,
        message_id: String,
        message_seq: i64,
        /// Boxed to keep the enum small (ChatMessageSummary is large on the wire).
        message: Option<Box<ChatMessageSummary>>,
    },
    Nack {
        conversation_id: String,
        code: String,
        message: String,
    },
    /// Socket not live / try_send failed / duplicate waiter key.
    Socket,
    /// Waited past timeout without Ack/Nack.
    Timeout,
}

type WaiterTx = oneshot::Sender<ChatSendWaitResult>;

/// Shared registry keyed by `client_operation_id` (= client_message_id).
#[derive(Debug, Default)]
pub struct ChatSendWaiterRegistry {
    waiters: DashMap<String, WaiterTx>,
}

impl ChatSendWaiterRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            waiters: DashMap::new(),
        }
    }

    /// Register a waiter. Returns `None` if this op id is already waiting
    /// (caller should treat as socket miss and use REST confirm).
    pub fn register(
        &self,
        client_operation_id: &str,
    ) -> Option<oneshot::Receiver<ChatSendWaitResult>> {
        let key = client_operation_id.trim();
        if key.is_empty() {
            return None;
        }
        if self.waiters.contains_key(key) {
            return None;
        }
        let (tx, rx) = oneshot::channel();
        self.waiters.insert(key.to_string(), tx);
        Some(rx)
    }

    pub fn cancel(&self, client_operation_id: &str) {
        let key = client_operation_id.trim();
        if key.is_empty() {
            return;
        }
        self.waiters.remove(key);
    }

    pub fn resolve_ack(
        &self,
        client_operation_id: &str,
        conversation_id: String,
        message_id: String,
        message_seq: i64,
        message: Option<Box<ChatMessageSummary>>,
    ) {
        self.resolve(
            client_operation_id,
            ChatSendWaitResult::Ack {
                conversation_id,
                message_id,
                message_seq,
                message,
            },
        );
    }

    pub fn resolve_nack(
        &self,
        client_operation_id: &str,
        conversation_id: String,
        code: String,
        message: String,
    ) {
        self.resolve(
            client_operation_id,
            ChatSendWaitResult::Nack {
                conversation_id,
                code,
                message,
            },
        );
    }

    fn resolve(&self, client_operation_id: &str, result: ChatSendWaitResult) {
        let key = client_operation_id.trim();
        if key.is_empty() {
            return;
        }
        if let Some((_, tx)) = self.waiters.remove(key) {
            let _ = tx.send(result);
        }
    }

    /// Fail all pending waiters (socket drop / reconnect). Callers that still
    /// hold the oneshot will observe a closed channel → treat as socket miss.
    pub fn fail_all_socket(&self) {
        self.waiters.clear();
    }
}

/// Wait for a registered oneshot with timeout. On timeout, cancel the registry
/// entry so a late ack does not leak.
pub async fn wait_for_result(
    registry: &ChatSendWaiterRegistry,
    client_operation_id: &str,
    rx: oneshot::Receiver<ChatSendWaitResult>,
    timeout: Duration,
) -> ChatSendWaitResult {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => ChatSendWaitResult::Socket,
        Err(_) => {
            registry.cancel(client_operation_id);
            ChatSendWaitResult::Timeout
        }
    }
}

pub type SharedChatSendWaiters = Arc<ChatSendWaiterRegistry>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_ack_delivers_to_waiter() {
        let registry = ChatSendWaiterRegistry::new();
        let rx = registry.register("op-1").expect("register");
        registry.resolve_ack("op-1", "c1".into(), "m1".into(), 3, None);
        match wait_for_result(&registry, "op-1", rx, Duration::from_secs(1)).await {
            ChatSendWaitResult::Ack {
                message_id,
                message_seq,
                ..
            } => {
                assert_eq!(message_id, "m1");
                assert_eq!(message_seq, 3);
            }
            other => panic!("expected ack, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_cancels_waiter() {
        let registry = ChatSendWaiterRegistry::new();
        let rx = registry.register("op-timeout").expect("register");
        let result = wait_for_result(&registry, "op-timeout", rx, Duration::from_millis(20)).await;
        assert!(matches!(result, ChatSendWaitResult::Timeout));
        // Late resolve must not panic after cancel.
        registry.resolve_ack("op-timeout", "c1".into(), "m1".into(), 1, None);
    }

    #[test]
    fn duplicate_register_returns_none() {
        let registry = ChatSendWaiterRegistry::new();
        assert!(registry.register("op-dup").is_some());
        assert!(registry.register("op-dup").is_none());
    }
}
