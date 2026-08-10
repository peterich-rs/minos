use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use dashmap::DashMap;
use minos_domain::{DeviceId, DeviceRole};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::wire::ServerFrame;
use super::RealtimeTopic;

pub use minos_protocol::realtime::ConnectionPrincipal;

const SEEN_DURABLE_EVENT_IDS_CAPACITY: usize = 1024;
/// Max live frames held per topic while replaying history.
const BARRIER_BUFFER_CAPACITY: usize = 512;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableSendResult {
    Delivered,
    AlreadySeen,
    /// Live frame held until catch-up replay finishes for this topic.
    Buffered,
}

/// Per-topic catch-up barrier: buffer live durable frames until DB replay drains.
#[derive(Debug, Default)]
struct TopicCatchupBarrier {
    /// Watermark at arm time (informational; drain uses after_seq from replay).
    #[allow(dead_code)]
    high_watermark: i64,
    /// Frames with topic_seq > high_watermark (or concurrent with replay), ordered by seq.
    buffer: BTreeMap<i64, (String, ServerFrame)>,
}

#[derive(Debug)]
pub struct ConnectionState {
    pub conn_id: ConnectionId,
    pub principal: ConnectionPrincipal,
    pub device_id: DeviceId,
    pub role: DeviceRole,
    topics: RwLock<HashSet<RealtimeTopic>>,
    seen_durable_event_ids: RwLock<VecDeque<String>>,
    /// Topics currently replaying history; live durable frames go to barrier buffer.
    catchup_barriers: RwLock<HashMap<RealtimeTopic, TopicCatchupBarrier>>,
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
            catchup_barriers: RwLock::new(HashMap::new()),
            push,
            created_at_ms,
        }
    }

    /// Begin buffering live durable events for `topic` until [`drain_catchup_barrier`].
    pub fn arm_catchup_barrier(&self, topic: &RealtimeTopic, high_watermark: i64) {
        let mut barriers = write_lock(&self.catchup_barriers);
        barriers.insert(
            topic.clone(),
            TopicCatchupBarrier {
                high_watermark,
                buffer: BTreeMap::new(),
            },
        );
    }

    #[must_use]
    pub fn is_catchup_buffering(&self, topic: &RealtimeTopic) -> bool {
        read_lock(&self.catchup_barriers).contains_key(topic)
    }

    /// Drain buffered live frames with `topic_seq > after_seq`, ordered by seq.
    /// Disarms the barrier. Caller writes frames to the socket in order.
    pub fn drain_catchup_barrier(
        &self,
        topic: &RealtimeTopic,
        after_seq: i64,
    ) -> Vec<(String, ServerFrame)> {
        let mut barriers = write_lock(&self.catchup_barriers);
        let Some(barrier) = barriers.remove(topic) else {
            return Vec::new();
        };
        barrier
            .buffer
            .into_iter()
            .filter(|(seq, _)| *seq > after_seq)
            .map(|(_, pair)| pair)
            .collect()
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

    #[must_use]
    pub fn has_seen_durable_event(&self, event_id: &str) -> bool {
        read_lock(&self.seen_durable_event_ids)
            .iter()
            .any(|existing| existing == event_id)
    }

    pub fn send(&self, frame: ServerFrame) -> Result<(), mpsc::error::TrySendError<ServerFrame>> {
        self.push.try_send(frame)
    }

    pub fn send_durable_event(
        &self,
        event_id: &str,
        frame: ServerFrame,
    ) -> Result<DurableSendResult, mpsc::error::TrySendError<ServerFrame>> {
        // Already delivered to the wire (or committed to drain path).
        if self.has_seen_durable_event(event_id) {
            return Ok(DurableSendResult::AlreadySeen);
        }

        // While catching up, divert live frames into an ordered barrier buffer so
        // the client never advances past live seq before replay of earlier seqs.
        // Do NOT mark seen here — DB replay must still be able to send retained
        // seqs that race in live; drain skips event_ids already remembered.
        if let ServerFrame::DurableEvent {
            ref topic,
            topic_seq,
            ..
        } = frame
        {
            if let Ok(parsed) = RealtimeTopic::parse(topic) {
                let mut barriers = write_lock(&self.catchup_barriers);
                if let Some(barrier) = barriers.get_mut(&parsed) {
                    if barrier.buffer.len() >= BARRIER_BUFFER_CAPACITY {
                        if let Some(oldest_seq) = barrier.buffer.keys().next().copied() {
                            barrier.buffer.remove(&oldest_seq);
                            tracing::warn!(
                                target: "minos_backend::realtime::subscription",
                                conn_id = %self.conn_id,
                                topic = %topic,
                                dropped_seq = oldest_seq,
                                "catchup barrier buffer full; dropped oldest live frame"
                            );
                        }
                    }
                    barrier
                        .buffer
                        .insert(topic_seq, (event_id.to_string(), frame));
                    return Ok(DurableSendResult::Buffered);
                }
            }
        }

        let mut seen = write_lock(&self.seen_durable_event_ids);
        if seen.iter().any(|existing| existing == event_id) {
            return Ok(DurableSendResult::AlreadySeen);
        }
        self.push.try_send(frame)?;
        seen.push_back(event_id.to_string());
        if seen.len() > SEEN_DURABLE_EVENT_IDS_CAPACITY {
            let _ = seen.pop_front();
        }

        Ok(DurableSendResult::Delivered)
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

    /// Live connection(s) for a device installation (Host or Account).
    /// Used for ephemeral Hub→Host frames such as `BotInboxDelivery`.
    #[must_use]
    pub fn connections_for_device(&self, device_id: DeviceId) -> Vec<Arc<ConnectionState>> {
        self.by_conn
            .iter()
            .filter_map(|entry| {
                let conn = entry.value();
                (conn.device_id == device_id).then(|| Arc::clone(conn))
            })
            .collect()
    }

    /// Push an ephemeral frame to every live connection for `device_id`.
    /// Returns how many sockets accepted the frame into their push queue.
    pub fn push_to_device(&self, device_id: DeviceId, frame: ServerFrame) -> usize {
        let mut sent = 0usize;
        for conn in self.connections_for_device(device_id) {
            match conn.send(frame.clone()) {
                Ok(()) => sent = sent.saturating_add(1),
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::realtime::subscription",
                        device_id = %device_id,
                        conn_id = %conn.conn_id,
                        error = %error,
                        "failed to push ephemeral frame to device connection"
                    );
                }
            }
        }
        sent
    }

    /// Live host connections for a specific installation id (mailbox delivery).
    #[must_use]
    pub fn host_connections_for_device(&self, host_device_id: DeviceId) -> Vec<Arc<ConnectionState>> {
        self.by_conn
            .iter()
            .filter_map(|entry| {
                let conn = entry.value();
                if conn.role == DeviceRole::AgentHost && conn.device_id == host_device_id {
                    Some(Arc::clone(conn))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Force-unsubscribe `topic` on every connection for `account_id` (membership revoke).
    /// Returns how many connections lost the topic.
    pub fn revoke_topic_for_account(&self, account_id: &str, topic: &RealtimeTopic) -> usize {
        let mut revoked = 0_usize;
        for entry in &self.by_conn {
            let conn = entry.value();
            let matches = match &conn.principal {
                ConnectionPrincipal::Account { account_id: aid } => aid == account_id,
                _ => false,
            };
            if !matches {
                continue;
            }
            if !conn.is_subscribed(topic) {
                continue;
            }
            self.remove_topics(conn.conn_id, std::slice::from_ref(topic));
            // Drop any in-flight catch-up buffer for this topic.
            let _ = conn.drain_catchup_barrier(topic, i64::MAX);
            revoked = revoked.saturating_add(1);
        }
        revoked
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
