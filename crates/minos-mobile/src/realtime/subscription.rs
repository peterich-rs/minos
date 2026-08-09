//! Topic subscription state: desired / pending / confirmed + applied cursors.
//!
//! - **desired**: product wants this topic (open chat, account, agent session)
//! - **pending**: Subscribe frame sent, SubscribeAck not yet received
//! - **confirmed**: gateway acked the subscription for this connection
//! - **cursors**: last **applied** durable topic_seq (advance only after apply ack)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Debug, Default)]
pub struct SubscriptionManager {
    inner: RwLock<SubscriptionState>,
}

#[derive(Debug, Default)]
struct SubscriptionState {
    desired: HashSet<String>,
    pending: HashSet<String>,
    confirmed: HashSet<String>,
    /// Last successfully applied durable seq per topic.
    cursors: HashMap<String, i64>,
}

impl SubscriptionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Mark topic as desired. Returns whether a Subscribe should be (re)sent:
    /// true when newly desired, or still desired but not confirmed on this
    /// connection (failed send / reconnect mid-flight).
    pub async fn desire_topic(&self, topic: &str, resume_hint: i64) -> bool {
        let mut state = self.inner.write().await;
        let was_desired = state.desired.contains(topic);
        state.desired.insert(topic.to_string());
        // Seed cursor only if missing; never reset an advanced applied seq.
        state
            .cursors
            .entry(topic.to_string())
            .or_insert(resume_hint.max(0));
        let needs_subscribe = !state.confirmed.contains(topic) && !state.pending.contains(topic);
        // If previously desired but never confirmed, still need subscribe.
        needs_subscribe || (!was_desired && !state.confirmed.contains(topic))
    }

    /// Legacy helper: desire topic with resume 0. Prefer `desire_topic`.
    pub async fn add_topic(&self, topic: &str, resume_after: i64) -> bool {
        self.desire_topic(topic, resume_after).await
    }

    /// Topic left product surface (e.g. leave conversation).
    pub async fn remove_topic(&self, topic: &str) {
        let mut state = self.inner.write().await;
        state.desired.remove(topic);
        state.pending.remove(topic);
        state.confirmed.remove(topic);
        // Keep cursor so re-open can resume; clear only on explicit clear_cursor.
    }

    /// Subscribe frames were written — move desired∩topics into pending.
    pub async fn mark_subscribe_sent(&self, topics: &[String]) {
        let mut state = self.inner.write().await;
        for t in topics {
            if state.desired.contains(t) {
                state.pending.insert(t.clone());
                state.confirmed.remove(t);
            }
        }
    }

    /// SubscribeAck — pending → confirmed for listed topics.
    pub async fn mark_subscribe_acked(&self, topics: &[String]) {
        let mut state = self.inner.write().await;
        for t in topics {
            state.pending.remove(t);
            if state.desired.contains(t) {
                state.confirmed.insert(t.clone());
            }
        }
    }

    /// SubscriptionDenied — drop transport state; remove from desired (fail closed).
    pub async fn mark_subscription_denied(&self, topic: &str) {
        let mut state = self.inner.write().await;
        state.desired.remove(topic);
        state.pending.remove(topic);
        state.confirmed.remove(topic);
    }

    /// Connection dropped: clear pending/confirmed; keep desired + applied cursors.
    pub async fn on_disconnect(&self) {
        let mut state = self.inner.write().await;
        state.pending.clear();
        state.confirmed.clear();
    }

    /// Clear applied cursor after SnapshotRequired (caller rebuilds via REST).
    pub async fn clear_cursor(&self, topic: &str) {
        let mut state = self.inner.write().await;
        if let Some(existing) = state.cursors.get_mut(topic) {
            *existing = 0;
        }
    }

    /// Advance applied cursor only after durable apply commit / intentional consume.
    pub async fn update_seq(&self, topic: &str, seq: i64) {
        let mut state = self.inner.write().await;
        let entry = state.cursors.entry(topic.to_string()).or_insert(0);
        if seq > *entry {
            *entry = seq;
        }
    }

    pub async fn resume_after_map(&self) -> HashMap<String, i64> {
        let state = self.inner.read().await;
        // Resume only for desired topics (product still wants them).
        state
            .desired
            .iter()
            .map(|t| {
                let seq = state.cursors.get(t).copied().unwrap_or(0);
                (t.clone(), seq)
            })
            .collect()
    }

    /// Topics product wants (used to build Subscribe on Hello / open).
    pub async fn desired_topics(&self) -> Vec<String> {
        let state = self.inner.read().await;
        let mut v: Vec<_> = state.desired.iter().cloned().collect();
        v.sort();
        v
    }

    /// Back-compat alias for session Hello path.
    pub async fn subscribed_topics(&self) -> Vec<String> {
        self.desired_topics().await
    }

    pub async fn is_confirmed(&self, topic: &str) -> bool {
        let state = self.inner.read().await;
        state.confirmed.contains(topic)
    }

    pub async fn is_pending(&self, topic: &str) -> bool {
        let state = self.inner.read().await;
        state.pending.contains(topic)
    }

    pub async fn is_desired(&self, topic: &str) -> bool {
        let state = self.inner.read().await;
        state.desired.contains(topic)
    }

    pub async fn applied_seq(&self, topic: &str) -> i64 {
        let state = self.inner.read().await;
        state.cursors.get(topic).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn desire_then_ack_confirms() {
        let mgr = SubscriptionManager::new();
        assert!(mgr.desire_topic("conversation:c1", 0).await);
        // Second desire while not confirmed still wants subscribe if not pending.
        mgr.mark_subscribe_sent(&["conversation:c1".into()]).await;
        assert!(!mgr.desire_topic("conversation:c1", 0).await); // pending
        mgr.mark_subscribe_acked(&["conversation:c1".into()]).await;
        assert!(mgr.is_confirmed("conversation:c1").await);
        // Already confirmed — no re-subscribe needed.
        assert!(!mgr.desire_topic("conversation:c1", 0).await);
    }

    #[tokio::test]
    async fn failed_send_can_retry_subscribe() {
        let mgr = SubscriptionManager::new();
        assert!(mgr.desire_topic("account:a", 0).await);
        // Simulate: desire registered but send never marked pending.
        assert!(mgr.desire_topic("account:a", 0).await);
    }

    #[tokio::test]
    async fn disconnect_clears_confirmed_keeps_desired_and_cursor() {
        let mgr = SubscriptionManager::new();
        mgr.desire_topic("conversation:c1", 0).await;
        mgr.mark_subscribe_sent(&["conversation:c1".into()]).await;
        mgr.mark_subscribe_acked(&["conversation:c1".into()]).await;
        mgr.update_seq("conversation:c1", 42).await;
        mgr.on_disconnect().await;
        assert!(mgr.is_desired("conversation:c1").await);
        assert!(!mgr.is_confirmed("conversation:c1").await);
        assert_eq!(mgr.applied_seq("conversation:c1").await, 42);
        // Needs re-subscribe after reconnect.
        assert!(mgr.desire_topic("conversation:c1", 0).await);
    }

    #[tokio::test]
    async fn denied_removes_desired() {
        let mgr = SubscriptionManager::new();
        mgr.desire_topic("conversation:x", 0).await;
        mgr.mark_subscription_denied("conversation:x").await;
        assert!(!mgr.is_desired("conversation:x").await);
    }
}
