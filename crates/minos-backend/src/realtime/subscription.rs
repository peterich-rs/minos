use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use dashmap::DashMap;
use minos_domain::{DeviceId, DeviceRole};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::wire::ServerFrame;
use super::RealtimeTopic;

pub use minos_protocol::realtime::ConnectionPrincipal;

const SEEN_DURABLE_EVENT_IDS_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub Uuid);

impl ConnectionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug)]
pub struct ConnectionState {
    pub conn_id: ConnectionId,
    pub principal: ConnectionPrincipal,
    pub device_id: DeviceId,
    pub role: DeviceRole,
    topics: RwLock<HashSet<RealtimeTopic>>,
    seen_durable_event_ids: RwLock<VecDeque<String>>,
    push: mpsc::Sender<ServerFrame>,
    pub created_at_ms: i64,
}

impl ConnectionState {
    #[must_use]
    pub fn new(
        principal: ConnectionPrincipal,
        device_id: DeviceId,
        role: DeviceRole,
        push: mpsc::Sender<ServerFrame>,
        created_at_ms: i64,
    ) -> Self {
        Self {
            conn_id: ConnectionId::new(),
            principal,
            device_id,
            role,
            topics: RwLock::new(HashSet::new()),
            seen_durable_event_ids: RwLock::new(VecDeque::new()),
            push,
            created_at_ms,
        }
    }

    #[must_use]
    pub fn topic_count(&self) -> usize {
        read_lock(&self.topics).len()
    }

    #[must_use]
    pub fn is_subscribed(&self, topic: &RealtimeTopic) -> bool {
        read_lock(&self.topics).contains(topic)
    }

    #[must_use]
    pub fn topics(&self) -> Vec<RealtimeTopic> {
        read_lock(&self.topics).iter().cloned().collect()
    }

    #[must_use]
    pub fn remember_durable_event(&self, event_id: &str) -> bool {
        let mut seen = write_lock(&self.seen_durable_event_ids);
        if seen.iter().any(|existing| existing == event_id) {
            return false;
        }
        seen.push_back(event_id.to_string());
        if seen.len() > SEEN_DURABLE_EVENT_IDS_CAPACITY {
            let _ = seen.pop_front();
        }
        true
    }

    pub fn send(&self, frame: ServerFrame) -> Result<(), mpsc::error::TrySendError<ServerFrame>> {
        self.push.try_send(frame)
    }
}

#[derive(Debug, Default)]
pub struct SubscriptionManager {
    by_topic: DashMap<RealtimeTopic, HashSet<ConnectionId>>,
    by_conn: DashMap<ConnectionId, Arc<ConnectionState>>,
}

impl SubscriptionManager {
    pub fn add_connection(&self, conn: Arc<ConnectionState>) {
        self.by_conn.insert(conn.conn_id, conn);
    }

    pub fn remove_connection(&self, conn_id: ConnectionId) {
        let Some((_, conn)) = self.by_conn.remove(&conn_id) else {
            return;
        };
        for topic in conn.topics() {
            let remove_topic = if let Some(mut entry) = self.by_topic.get_mut(&topic) {
                entry.remove(&conn_id);
                entry.is_empty()
            } else {
                false
            };
            if remove_topic {
                self.by_topic.remove(&topic);
            }
        }
    }

    #[must_use]
    pub fn add_topics(
        &self,
        conn_id: ConnectionId,
        topics: &[RealtimeTopic],
    ) -> Vec<RealtimeTopic> {
        let Some(conn) = self.by_conn.get(&conn_id) else {
            return Vec::new();
        };

        let mut newly = Vec::new();
        let mut conn_topics = write_lock(&conn.topics);
        for topic in topics {
            if conn_topics.insert(topic.clone()) {
                newly.push(topic.clone());
                self.by_topic
                    .entry(topic.clone())
                    .or_default()
                    .insert(conn_id);
            }
        }
        newly
    }

    pub fn remove_topics(&self, conn_id: ConnectionId, topics: &[RealtimeTopic]) {
        let Some(conn) = self.by_conn.get(&conn_id) else {
            return;
        };

        let mut conn_topics = write_lock(&conn.topics);
        for topic in topics {
            if !conn_topics.remove(topic) {
                continue;
            }
            let remove_topic = if let Some(mut entry) = self.by_topic.get_mut(topic) {
                entry.remove(&conn_id);
                entry.is_empty()
            } else {
                false
            };
            if remove_topic {
                self.by_topic.remove(topic);
            }
        }
    }

    #[must_use]
    pub fn fanout_targets(&self, topic: &RealtimeTopic) -> Vec<Arc<ConnectionState>> {
        let Some(entry) = self.by_topic.get(topic) else {
            return Vec::new();
        };
        entry
            .iter()
            .filter_map(|conn_id| {
                self.by_conn
                    .get(conn_id)
                    .map(|conn| Arc::clone(conn.value()))
            })
            .collect()
    }
}

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_manager_adds_and_removes_topics() {
        let manager = SubscriptionManager::default();
        let (push, _rx) = mpsc::channel(4);
        let conn = Arc::new(ConnectionState::new(
            ConnectionPrincipal::Account {
                account_id: "acct-1".into(),
            },
            DeviceId::new(),
            DeviceRole::MobileClient,
            push,
            1,
        ));

        manager.add_connection(Arc::clone(&conn));
        let topic = RealtimeTopic::AgentSession("sess-1".into());
        let newly = manager.add_topics(conn.conn_id, std::slice::from_ref(&topic));
        assert_eq!(newly, vec![topic.clone()]);
        assert_eq!(manager.fanout_targets(&topic).len(), 1);

        manager.remove_topics(conn.conn_id, std::slice::from_ref(&topic));
        assert!(manager.fanout_targets(&topic).is_empty());

        manager.remove_connection(conn.conn_id);
        assert!(manager.fanout_targets(&topic).is_empty());
    }
}
