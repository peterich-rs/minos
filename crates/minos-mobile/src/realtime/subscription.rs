use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Debug, Default)]
pub struct SubscriptionManager {
    inner: RwLock<SubscriptionState>,
}

#[derive(Debug, Default)]
struct SubscriptionState {
    topics: HashMap<String, i64>,
}

impl SubscriptionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn add_topic(&self, topic: &str, resume_after: i64) -> bool {
        let mut state = self.inner.write().await;
        if state.topics.contains_key(topic) {
            false
        } else {
            state.topics.insert(topic.to_string(), resume_after);
            true
        }
    }

    pub async fn remove_topic(&self, topic: &str) {
        let mut state = self.inner.write().await;
        state.topics.remove(topic);
    }

    pub async fn update_seq(&self, topic: &str, seq: i64) {
        let mut state = self.inner.write().await;
        if let Some(existing) = state.topics.get_mut(topic) {
            if seq > *existing {
                *existing = seq;
            }
        }
    }

    pub async fn resume_after_map(&self) -> HashMap<String, i64> {
        let state = self.inner.read().await;
        state.topics.clone()
    }

    pub async fn subscribed_topics(&self) -> Vec<String> {
        let state = self.inner.read().await;
        state.topics.keys().cloned().collect()
    }
}
