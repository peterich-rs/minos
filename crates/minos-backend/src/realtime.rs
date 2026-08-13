pub mod auth;
pub mod connection_registry;
pub mod event;
pub mod gateway;
pub mod liveness;
pub mod presence;
pub mod subscription;
pub mod topic;
pub mod wire;

use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use futures::StreamExt;
use minos_domain::DeviceId;
use moka::sync::Cache;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::error::BackendError;
use crate::notifications::NotificationService;
use crate::store::{durable_event_log, host_commands, outbox_events, StoreHandle};

pub use connection_registry::{ConnectionRevocation, RealtimeConnectionRegistry};
pub use event::{ApprovalResolution, DurableEvent, DurableEventEnvelope, SenderRef};
pub use subscription::{
    ConnectionId, ConnectionPrincipal, ConnectionState, DurableSendResult, SubscriptionManager,
};
pub use topic::{RealtimeTopic, TopicKind};
const DEFAULT_PEER_TARGET_CACHE_TTL: Duration = Duration::from_secs(5);
const CLUSTER_RECONNECT_DELAY: Duration = Duration::from_secs(1);
/// Social durable fanout: large batch, short lease (publish then ack).
const SOCIAL_OUTBOX_BATCH_SIZE: u32 = 64;
const SOCIAL_OUTBOX_CLAIM_LEASE: Duration = Duration::from_secs(30);
/// Host commands: smaller batch, longer lease (async host observation, no serial wait).
const HOST_COMMAND_OUTBOX_BATCH_SIZE: u32 = 16;
const HOST_COMMAND_OUTBOX_CLAIM_LEASE: Duration = Duration::from_secs(120);
const OUTBOX_RETRY_DELAY: Duration = Duration::from_millis(250);
const OUTBOX_MAX_ATTEMPTS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
pub enum CacheBackendKind {
    InMemory,
    Redis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
pub enum MessageBusBackendKind {
    Inline,
    Redis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ClusterEvent {
    DurableFanout {
        origin_instance_id: String,
        topic: String,
        topic_seq: i64,
        event_kind: String,
        payload: Value,
        event_id: String,
    },
}

#[derive(Clone)]
pub enum PeerTargetCacheBackend {
    InMemory(Cache<DeviceId, Vec<DeviceId>>),
    Redis {
        client: redis::Client,
        ttl: Duration,
    },
}

impl PeerTargetCacheBackend {
    #[must_use]
    pub fn in_memory(ttl: Duration) -> Self {
        Self::InMemory(Cache::builder().time_to_live(ttl).build())
    }

    pub fn redis(redis_url: &str, ttl: Duration) -> Result<Self, BackendError> {
        let client = redis::Client::open(redis_url).map_err(|error| BackendError::Cache {
            operation: "peer_target_cache::redis_client".into(),
            message: error.to_string(),
        })?;
        Ok(Self::Redis { client, ttl })
    }

    pub async fn get(
        &self,
        host_device_id: DeviceId,
    ) -> Result<Option<Vec<DeviceId>>, BackendError> {
        match self {
            Self::InMemory(cache) => Ok(cache.get(&host_device_id)),
            Self::Redis { client, .. } => {
                let mut conn =
                    client
                        .get_multiplexed_async_connection()
                        .await
                        .map_err(|error| BackendError::Cache {
                            operation: "peer_target_cache::redis_connect".into(),
                            message: error.to_string(),
                        })?;
                let key = peer_target_cache_key(host_device_id);
                let raw: Option<String> =
                    conn.get(&key).await.map_err(|error| BackendError::Cache {
                        operation: "peer_target_cache::redis_get".into(),
                        message: error.to_string(),
                    })?;
                raw.map(|payload| {
                    serde_json::from_str::<Vec<DeviceId>>(&payload).map_err(|error| {
                        BackendError::Cache {
                            operation: "peer_target_cache::redis_decode".into(),
                            message: error.to_string(),
                        }
                    })
                })
                .transpose()
            }
        }
    }

    pub async fn set(
        &self,
        host_device_id: DeviceId,
        targets: &[DeviceId],
    ) -> Result<(), BackendError> {
        match self {
            Self::InMemory(cache) => {
                cache.insert(host_device_id, targets.to_vec());
                Ok(())
            }
            Self::Redis { client, ttl } => {
                let mut conn =
                    client
                        .get_multiplexed_async_connection()
                        .await
                        .map_err(|error| BackendError::Cache {
                            operation: "peer_target_cache::redis_connect".into(),
                            message: error.to_string(),
                        })?;
                let payload =
                    serde_json::to_string(targets).map_err(|error| BackendError::Cache {
                        operation: "peer_target_cache::redis_encode".into(),
                        message: error.to_string(),
                    })?;
                let ttl_secs = ttl.as_secs().max(1);
                let key = peer_target_cache_key(host_device_id);
                let _: () = conn
                    .set_ex(&key, payload, ttl_secs)
                    .await
                    .map_err(|error| BackendError::Cache {
                        operation: "peer_target_cache::redis_set".into(),
                        message: error.to_string(),
                    })?;
                Ok(())
            }
        }
    }

    pub async fn invalidate(&self, host_device_id: DeviceId) -> Result<(), BackendError> {
        match self {
            Self::InMemory(cache) => {
                cache.invalidate(&host_device_id);
                Ok(())
            }
            Self::Redis { client, .. } => {
                let mut conn =
                    client
                        .get_multiplexed_async_connection()
                        .await
                        .map_err(|error| BackendError::Cache {
                            operation: "peer_target_cache::redis_connect".into(),
                            message: error.to_string(),
                        })?;
                let key = peer_target_cache_key(host_device_id);
                let _: usize = conn.del(&key).await.map_err(|error| BackendError::Cache {
                    operation: "peer_target_cache::redis_del".into(),
                    message: error.to_string(),
                })?;
                Ok(())
            }
        }
    }
}

fn peer_target_cache_key(host_device_id: DeviceId) -> String {
    format!("minos:peer-targets:{host_device_id}")
}

fn peer_target_cache_backend_cell() -> &'static RwLock<PeerTargetCacheBackend> {
    static CELL: OnceLock<RwLock<PeerTargetCacheBackend>> = OnceLock::new();
    CELL.get_or_init(|| {
        RwLock::new(PeerTargetCacheBackend::in_memory(
            DEFAULT_PEER_TARGET_CACHE_TTL,
        ))
    })
}

pub fn configure_peer_target_cache(backend: PeerTargetCacheBackend) {
    let lock = peer_target_cache_backend_cell();
    let mut guard = lock
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = backend;
}

pub fn peer_target_cache_backend() -> PeerTargetCacheBackend {
    peer_target_cache_backend_cell()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

#[derive(Clone)]
pub enum MessageBusBackend {
    Inline,
    Redis {
        client: redis::Client,
        channel: String,
    },
}

impl MessageBusBackend {
    #[must_use]
    pub fn inline() -> Self {
        Self::Inline
    }

    pub fn redis(redis_url: &str, channel: String) -> Result<Self, BackendError> {
        let client = redis::Client::open(redis_url).map_err(|error| BackendError::MessageBus {
            operation: "message_bus::redis_client".into(),
            message: error.to_string(),
        })?;
        Ok(Self::Redis { client, channel })
    }

    async fn publish(&self, event: &ClusterEvent) -> Result<(), BackendError> {
        match self {
            Self::Inline => Ok(()),
            Self::Redis { client, channel } => {
                let payload =
                    serde_json::to_string(event).map_err(|error| BackendError::MessageBus {
                        operation: "message_bus::encode".into(),
                        message: error.to_string(),
                    })?;
                let mut conn =
                    client
                        .get_multiplexed_async_connection()
                        .await
                        .map_err(|error| BackendError::MessageBus {
                            operation: "message_bus::redis_connect".into(),
                            message: error.to_string(),
                        })?;
                let _: i64 = redis::cmd("PUBLISH")
                    .arg(channel)
                    .arg(payload)
                    .query_async(&mut conn)
                    .await
                    .map_err(|error| BackendError::MessageBus {
                        operation: "message_bus::publish".into(),
                        message: error.to_string(),
                    })?;
                Ok(())
            }
        }
    }

    fn spawn_listener(&self, realtime: Arc<RealtimeFanout>) -> Option<JoinHandle<()>> {
        match self {
            Self::Inline => None,
            Self::Redis { client, channel } => {
                let client = client.clone();
                let channel = channel.clone();
                Some(tokio::spawn(async move {
                    loop {
                        match client.get_async_pubsub().await {
                            Ok(mut pubsub) => {
                                if let Err(error) = pubsub.subscribe(&channel).await {
                                    tracing::warn!(
                                        target: "minos_backend::realtime",
                                        channel,
                                        error = %error,
                                        "failed to subscribe to redis cluster channel"
                                    );
                                } else {
                                    let mut stream = pubsub.on_message();
                                    while let Some(message) = stream.next().await {
                                        match message.get_payload::<String>() {
                                            Ok(payload) => {
                                                match serde_json::from_str::<ClusterEvent>(&payload)
                                                {
                                                    Ok(event) => {
                                                        realtime.apply_cluster_event(event)
                                                    }
                                                    Err(error) => tracing::warn!(
                                                        target: "minos_backend::realtime",
                                                        error = %error,
                                                        "failed to decode cluster event"
                                                    ),
                                                }
                                            }
                                            Err(error) => tracing::warn!(
                                                target: "minos_backend::realtime",
                                                error = %error,
                                                "failed to read redis pubsub payload"
                                            ),
                                        }
                                    }
                                }
                            }
                            Err(error) => tracing::warn!(
                                target: "minos_backend::realtime",
                                channel,
                                error = %error,
                                "failed to connect redis pubsub listener"
                            ),
                        }
                        tokio::time::sleep(CLUSTER_RECONNECT_DELAY).await;
                    }
                }))
            }
        }
    }
}

pub struct RealtimeFanout {
    subscription_mgr: Arc<SubscriptionManager>,
    store: StoreHandle,
    bus: MessageBusBackend,
    instance_id: String,
    outbox_worker_id: String,
    notification_service: Option<Arc<dyn NotificationService>>,
}

impl RealtimeFanout {
    #[must_use]
    pub fn new(
        subscription_mgr: Arc<SubscriptionManager>,
        store: StoreHandle,
        bus: MessageBusBackend,
        instance_id: String,
        notification_service: Option<Arc<dyn NotificationService>>,
    ) -> Arc<Self> {
        let outbox_worker_id = format!("realtime-outbox-{instance_id}");
        Arc::new(Self {
            subscription_mgr,
            store,
            bus,
            instance_id,
            outbox_worker_id,
            notification_service,
        })
    }

    pub fn spawn_listener(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        self.bus.spawn_listener(Arc::clone(self))
    }

    /// Dispatch social_durable lane only. Never blocks on host command ack.
    pub async fn dispatch_outbox_batch(&self) -> Result<usize, BackendError> {
        self.dispatch_outbox_lane(
            outbox_events::OutboxLane::SocialDurable,
            SOCIAL_OUTBOX_BATCH_SIZE,
            SOCIAL_OUTBOX_CLAIM_LEASE,
        )
        .await
    }

    /// Dispatch host_command lane: publish without serial wait_ack; expire → dead_letter.
    pub async fn dispatch_host_command_outbox_batch(&self) -> Result<usize, BackendError> {
        self.dispatch_outbox_lane(
            outbox_events::OutboxLane::HostCommand,
            HOST_COMMAND_OUTBOX_BATCH_SIZE,
            HOST_COMMAND_OUTBOX_CLAIM_LEASE,
        )
        .await
    }

    async fn dispatch_outbox_lane(
        &self,
        lane: outbox_events::OutboxLane,
        batch_size: u32,
        claim_lease: Duration,
    ) -> Result<usize, BackendError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.recover_stale_outbox_claims(now_ms, lane, claim_lease)
            .await;
        let claimed = outbox_events::claim_available(
            &self.store,
            &self.outbox_worker_id,
            now_ms,
            batch_size,
            lane,
        )
        .await?;
        let count = claimed.len();
        for row in claimed {
            self.dispatch_outbox_row(row).await;
        }
        Ok(count)
    }

    pub async fn publish_durable_event_by_id(
        &self,
        topic_kind: &str,
        event_id: &str,
    ) -> Result<(), BackendError> {
        let row = durable_event_log::get(&self.store, topic_kind, event_id)
            .await?
            .ok_or_else(|| BackendError::StoreQuery {
                operation: "realtime.publish_durable_event_by_id".into(),
                message: format!("missing durable event {topic_kind}/{event_id}"),
            })?;
        self.publish_durable_row(&row).await
    }

    async fn recover_stale_outbox_claims(
        &self,
        now_ms: i64,
        lane: outbox_events::OutboxLane,
        claim_lease: Duration,
    ) {
        // Lane-scoped requeue every tick: indexed no-op when nothing is stale; avoids a
        // shared recovery timer that starved host_command behind social batches.
        let lease_ms = i64::try_from(claim_lease.as_millis()).unwrap_or(i64::MAX);
        let cutoff_ms = now_ms.saturating_sub(lease_ms);
        match outbox_events::requeue_stale_claims(
            &self.store,
            cutoff_ms,
            now_ms,
            &serde_json::json!({
                "kind": "claim_recovered",
                "worker": self.outbox_worker_id,
                "lane": lane.as_str(),
                "cutoff_ms": cutoff_ms
            }),
            lane,
        )
        .await
        {
            Ok(0) => {}
            Ok(count) => {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    count,
                    cutoff_ms,
                    lane = lane.as_str(),
                    "requeued stale outbox claims"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    error = %error,
                    cutoff_ms,
                    lane = lane.as_str(),
                    "failed to requeue stale outbox claims"
                );
            }
        }
    }

    pub fn fanout_stream_event(
        &self,
        topic: &RealtimeTopic,
        kind: impl Into<String>,
        seq: Option<i64>,
        payload: Value,
    ) {
        let frame = wire::ServerFrame::StreamEvent {
            topic: topic.topic_string(),
            kind: kind.into(),
            seq,
            payload,
        };
        for target in self.subscription_mgr.fanout_targets(topic) {
            if let Err(error) = target.send(frame.clone()) {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    conn_id = %target.conn_id,
                    topic = %topic.topic_string(),
                    error = ?error,
                    "formal gateway stream fanout dropped frame"
                );
            }
        }
    }

    async fn dispatch_outbox_row(&self, row: outbox_events::OutboxEventRow) {
        match row.lane {
            outbox_events::OutboxLane::SocialDurable => {
                self.dispatch_social_outbox_row(row).await;
            }
            outbox_events::OutboxLane::HostCommand => {
                self.dispatch_host_command_outbox_row(row).await;
            }
        }
    }

    async fn dispatch_social_outbox_row(&self, row: outbox_events::OutboxEventRow) {
        let durable =
            match durable_event_log::get(&self.store, &row.topic_kind, &row.event_id).await {
                Ok(Some(durable)) => durable,
                Ok(None) => {
                    self.dead_letter_outbox_row(&row, "missing durable event")
                        .await;
                    crate::telemetry::record_outbox_dispatch("dead_letter");
                    return;
                }
                Err(error) => {
                    self.requeue_outbox_row(&row, &error.to_string()).await;
                    crate::telemetry::record_outbox_dispatch("retry");
                    return;
                }
            };

        if let Err(error) = self.publish_social_durable_row(&durable).await {
            self.requeue_outbox_row(&row, &error.to_string()).await;
            crate::telemetry::record_outbox_dispatch("retry");
            return;
        }

        self.try_ack_outbox_row(&row).await;
        crate::telemetry::record_outbox_dispatch("ok");
    }

    /// Host command path: publish once, leave claimed until host observes (async ack),
    /// or dead-letter on deadline expiry. Never blocks social batches with wait_ack.
    async fn dispatch_host_command_outbox_row(&self, row: outbox_events::OutboxEventRow) {
        let durable =
            match durable_event_log::get(&self.store, &row.topic_kind, &row.event_id).await {
                Ok(Some(durable)) => durable,
                Ok(None) => {
                    self.dead_letter_outbox_row(&row, "missing durable event")
                        .await;
                    crate::telemetry::record_outbox_dispatch("dead_letter");
                    return;
                }
                Err(error) => {
                    self.requeue_outbox_row(&row, &error.to_string()).await;
                    crate::telemetry::record_outbox_dispatch("retry");
                    return;
                }
            };

        let (kind, payload) = durable_event_kind_payload(&durable.payload_json);
        if kind != "host_command_issued" {
            self.dead_letter_outbox_row(&row, "host_command lane row is not host_command_issued")
                .await;
            crate::telemetry::record_outbox_dispatch("dead_letter");
            return;
        }
        let Some(command_id) = payload
            .get("command_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            self.dead_letter_outbox_row(&row, "host_command_issued missing command_id")
                .await;
            crate::telemetry::record_outbox_dispatch("dead_letter");
            return;
        };

        // Host observation wins over deadline: if the host already acked/finished,
        // success-settle the outbox even when the command deadline has elapsed.
        if self.host_command_is_observed(&command_id).await {
            self.try_ack_outbox_row(&row).await;
            crate::telemetry::record_outbox_dispatch("ok");
            return;
        }

        let deadline_at_ms = payload
            .get("deadline_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let now_ms = chrono::Utc::now().timestamp_millis();
        if deadline_at_ms > 0 && deadline_at_ms <= now_ms {
            self.dead_letter_outbox_row(&row, "host_command_expired")
                .await;
            crate::telemetry::record_outbox_dispatch("host_command_expired");
            crate::telemetry::increment_host_command_outbox_expired();
            return;
        }

        if let Err(error) = self
            .publish_host_command_durable_row(&durable, &kind, &payload)
            .await
        {
            self.requeue_outbox_row(&row, &error.to_string()).await;
            crate::telemetry::record_outbox_dispatch("retry");
            return;
        }

        // Async ack: leave claimed until gateway HostCommandAck/Result calls
        // ack_pending_host_command_events, or until reclaimed after lease / deadline.
        if self.host_command_is_observed(&command_id).await {
            self.try_ack_outbox_row(&row).await;
            crate::telemetry::record_outbox_dispatch("ok");
        } else {
            crate::telemetry::record_outbox_dispatch("host_command_published");
        }
    }

    async fn host_command_is_observed(&self, command_id: &str) -> bool {
        match host_commands::get(&self.store, command_id).await {
            Ok(Some(row)) => row.is_host_observed(),
            Ok(None) | Err(_) => false,
        }
    }

    async fn try_ack_outbox_row(&self, row: &outbox_events::OutboxEventRow) {
        let ack_at_ms = chrono::Utc::now().timestamp_millis();
        match outbox_events::ack(&self.store, &row.outbox_id, ack_at_ms).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    outbox_id = %row.outbox_id,
                    lane = row.lane.as_str(),
                    "realtime outbox dispatcher lost claimed row before ack"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    error = %error,
                    outbox_id = %row.outbox_id,
                    lane = row.lane.as_str(),
                    "realtime outbox dispatcher failed to ack claimed row"
                );
            }
        }
    }

    async fn publish_social_durable_row(
        &self,
        row: &durable_event_log::DurableEventRow,
    ) -> Result<(), BackendError> {
        let topic =
            RealtimeTopic::parse(&row.topic).map_err(|error| BackendError::StoreDecode {
                column: "durable_event_log.topic".into(),
                message: error.to_string(),
            })?;
        let (kind, payload) = durable_event_kind_payload(&row.payload_json);
        self.bus
            .publish(&ClusterEvent::DurableFanout {
                origin_instance_id: self.instance_id.clone(),
                topic: row.topic.clone(),
                topic_seq: row.topic_seq,
                event_kind: kind.clone(),
                payload: payload.clone(),
                event_id: row.event_id.clone(),
            })
            .await?;
        let frame = wire::ServerFrame::DurableEvent {
            topic: row.topic.clone(),
            topic_seq: row.topic_seq,
            kind,
            payload,
            event_id: row.event_id.clone(),
        };
        self.broadcast_durable_event_local(&topic, &row.event_id, frame);
        self.enqueue_push_dispatch(row).await;
        Ok(())
    }

    async fn publish_host_command_durable_row(
        &self,
        row: &durable_event_log::DurableEventRow,
        kind: &str,
        payload: &Value,
    ) -> Result<(), BackendError> {
        let topic =
            RealtimeTopic::parse(&row.topic).map_err(|error| BackendError::StoreDecode {
                column: "durable_event_log.topic".into(),
                message: error.to_string(),
            })?;
        self.bus
            .publish(&ClusterEvent::DurableFanout {
                origin_instance_id: self.instance_id.clone(),
                topic: row.topic.clone(),
                topic_seq: row.topic_seq,
                event_kind: kind.to_string(),
                payload: payload.clone(),
                event_id: row.event_id.clone(),
            })
            .await?;
        let frame = wire::ServerFrame::DurableEvent {
            topic: row.topic.clone(),
            topic_seq: row.topic_seq,
            kind: kind.to_string(),
            payload: payload.clone(),
            event_id: row.event_id.clone(),
        };
        self.broadcast_durable_event_local_untracked(&topic, frame);
        Ok(())
    }

    /// Publish a durable event by id (post-commit fast path / tests).
    /// Social events push+return; host commands publish without blocking wait.
    async fn publish_durable_row(
        &self,
        row: &durable_event_log::DurableEventRow,
    ) -> Result<(), BackendError> {
        let (kind, payload) = durable_event_kind_payload(&row.payload_json);
        if kind == "host_command_issued" {
            let deadline_at_ms = payload
                .get("deadline_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let now_ms = chrono::Utc::now().timestamp_millis();
            if deadline_at_ms > 0 && deadline_at_ms <= now_ms {
                return Err(BackendError::MessageBus {
                    operation: "realtime.publish_durable_row".into(),
                    message: format!(
                        "host_command_issued event {} expired at {deadline_at_ms}",
                        row.event_id
                    ),
                });
            }
            return self
                .publish_host_command_durable_row(row, &kind, &payload)
                .await;
        }
        self.publish_social_durable_row(row).await
    }

    /// Enqueue durable push work for target accounts (worker claims + retries).
    /// Failures are logged; social outbox ack is independent of push delivery.
    async fn enqueue_push_dispatch(&self, row: &durable_event_log::DurableEventRow) {
        if self.notification_service.is_none() {
            return;
        }
        let payload = match serde_json::from_value::<DurableEvent>(row.payload_json.clone()) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::notifications",
                    event_id = %row.event_id,
                    error = %error,
                    "skipping push enqueue for undecodable durable event"
                );
                return;
            }
        };
        let targets =
            match crate::notifications::resolve_target_accounts(&self.store, &payload).await {
                Ok(t) => t,
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::notifications",
                        event_id = %row.event_id,
                        error = %error,
                        "failed to resolve push targets; push not enqueued"
                    );
                    return;
                }
            };
        if targets.is_empty() {
            return;
        }
        let payload_json = match serde_json::to_string(&payload) {
            Ok(s) => s,
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::notifications",
                    event_id = %row.event_id,
                    error = %error,
                    "failed to serialize push payload"
                );
                return;
            }
        };
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut enqueued = 0u32;
        for account_id in &targets {
            match crate::store::push_dispatch_queue::enqueue(
                &self.store,
                &row.event_id,
                account_id,
                &row.topic,
                row.topic_seq,
                &payload_json,
                now_ms,
            )
            .await
            {
                Ok(true) => enqueued = enqueued.saturating_add(1),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::notifications",
                        event_id = %row.event_id,
                        account_id = %account_id,
                        error = %error,
                        "failed to enqueue push dispatch row"
                    );
                }
            }
        }
        if enqueued > 0 {
            tracing::debug!(
                target: "minos_backend::notifications",
                event_id = %row.event_id,
                enqueued,
                "enqueued durable push dispatch work"
            );
        }
    }

    async fn requeue_outbox_row(&self, row: &outbox_events::OutboxEventRow, message: &str) {
        if row.attempts >= OUTBOX_MAX_ATTEMPTS {
            self.dead_letter_outbox_row(row, message).await;
            return;
        }

        let retry_at_ms = chrono::Utc::now()
            .timestamp_millis()
            .saturating_add(i64::try_from(OUTBOX_RETRY_DELAY.as_millis()).unwrap_or(i64::MAX));
        let error_json = serde_json::json!({ "message": message });
        if let Err(error) =
            outbox_events::retry(&self.store, &row.outbox_id, retry_at_ms, &error_json).await
        {
            tracing::warn!(
                target: "minos_backend::realtime",
                error = %error,
                outbox_id = %row.outbox_id,
                "realtime outbox dispatcher failed to requeue claimed row"
            );
        }
    }

    async fn dead_letter_outbox_row(&self, row: &outbox_events::OutboxEventRow, message: &str) {
        let dead_at_ms = chrono::Utc::now().timestamp_millis();
        let error_json = serde_json::json!({ "message": message });
        if let Err(error) =
            outbox_events::dead_letter(&self.store, &row.outbox_id, dead_at_ms, &error_json).await
        {
            tracing::warn!(
                target: "minos_backend::realtime",
                error = %error,
                outbox_id = %row.outbox_id,
                "realtime outbox dispatcher failed to dead-letter claimed row"
            );
        }
    }

    fn apply_cluster_event(&self, event: ClusterEvent) {
        match event {
            ClusterEvent::DurableFanout {
                origin_instance_id,
                topic,
                topic_seq,
                event_kind,
                payload,
                event_id,
            } => {
                if origin_instance_id == self.instance_id {
                    return;
                }
                let parsed_topic = match RealtimeTopic::parse(&topic) {
                    Ok(topic) => topic,
                    Err(error) => {
                        tracing::warn!(
                            target: "minos_backend::realtime",
                            error = %error,
                            topic = %topic,
                            "failed to parse clustered durable topic"
                        );
                        return;
                    }
                };
                let frame = wire::ServerFrame::DurableEvent {
                    topic,
                    topic_seq,
                    kind: event_kind.clone(),
                    payload,
                    event_id: event_id.clone(),
                };
                let _ = if event_kind == "host_command_issued" {
                    self.broadcast_durable_event_local_untracked(&parsed_topic, frame)
                } else {
                    self.broadcast_durable_event_local(&parsed_topic, &event_id, frame)
                };
            }
        }
    }

    fn broadcast_durable_event_local(
        &self,
        topic: &RealtimeTopic,
        event_id: &str,
        frame: wire::ServerFrame,
    ) -> DurableFanoutStats {
        let mut stats = DurableFanoutStats::default();
        for target in self.subscription_mgr.fanout_targets(topic) {
            stats.targets += 1;
            match target.send_durable_event(event_id, frame.clone()) {
                Ok(DurableSendResult::Delivered) => stats.delivered += 1,
                Ok(DurableSendResult::AlreadySeen) => stats.already_seen += 1,
                Ok(DurableSendResult::Buffered) => {
                    // Held for ordered drain after Subscribe replay — still "delivered" to conn.
                    stats.delivered += 1;
                }
                Err(error) => {
                    stats.failed += 1;
                    tracing::warn!(
                        target: "minos_backend::realtime",
                        conn_id = %target.conn_id,
                        topic = %topic.topic_string(),
                        error = ?error,
                        "formal gateway durable fanout backpressure; revoking connection"
                    );
                    target.revoke(ConnectionRevocation::Backpressure);
                }
            }
        }
        stats
    }

    fn broadcast_durable_event_local_untracked(
        &self,
        topic: &RealtimeTopic,
        frame: wire::ServerFrame,
    ) -> DurableFanoutStats {
        let mut stats = DurableFanoutStats::default();
        for target in self.subscription_mgr.fanout_targets(topic) {
            stats.targets += 1;
            match target.send(frame.clone()) {
                Ok(()) => stats.delivered += 1,
                Err(error) => {
                    stats.failed += 1;
                    tracing::warn!(
                        target: "minos_backend::realtime",
                        conn_id = %target.conn_id,
                        topic = %topic.topic_string(),
                        error = ?error,
                        "formal gateway durable fanout backpressure; revoking connection"
                    );
                    target.revoke(ConnectionRevocation::Backpressure);
                }
            }
        }
        stats
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct DurableFanoutStats {
    targets: usize,
    delivered: usize,
    already_seen: usize,
    failed: usize,
}

fn durable_event_kind_payload(value: &Value) -> (String, Value) {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| {
            let looks_like_host_command = value.get("command_id").and_then(Value::as_str).is_some()
                && value.get("method").and_then(Value::as_str).is_some()
                && value
                    .get("deadline_at_ms")
                    .and_then(Value::as_i64)
                    .is_some();
            looks_like_host_command.then_some("host_command_issued")
        })
        .unwrap_or("unknown")
        .to_string();
    let mut payload = value.clone();
    if let Value::Object(map) = &mut payload {
        map.remove("kind");
    }
    (kind, payload)
}
