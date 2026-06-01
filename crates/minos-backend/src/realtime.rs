pub mod auth;
pub mod event;
pub mod gateway;
pub mod subscription;
pub mod topic;
pub mod wire;

use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use futures::StreamExt;
use minos_domain::DeviceId;
use minos_protocol::Envelope;
use moka::sync::Cache;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::BackendError;
use crate::session::SessionRegistry;
use crate::store::{durable_event_log, outbox_events, StoreHandle};

pub use event::{ApprovalResolution, DurableEvent, DurableEventEnvelope, SenderRef};
pub use subscription::{ConnectionId, ConnectionPrincipal, ConnectionState, SubscriptionManager};
pub use topic::{RealtimeTopic, TopicKind};
pub use wire::{ClientFrame as GatewayClientFrame, ServerFrame as GatewayServerFrame};

const DEFAULT_PEER_TARGET_CACHE_TTL: Duration = Duration::from_secs(5);
const CLUSTER_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const REALTIME_WORKER_CAPACITY: usize = 1024;
const OUTBOX_DISPATCH_BATCH_SIZE: u32 = 64;
const OUTBOX_IDLE_DELAY: Duration = Duration::from_millis(100);
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
    UiFanout {
        origin_instance_id: String,
        target_device_ids: Vec<DeviceId>,
        envelope: Envelope,
    },
    SocialFanout {
        origin_instance_id: String,
        target_account_ids: Vec<String>,
        envelope: Envelope,
    },
    DurableFanout {
        origin_instance_id: String,
        topic: String,
        topic_seq: i64,
        event_kind: String,
        payload: Value,
        event_id: String,
    },
}

#[derive(Debug, Clone)]
enum RealtimeJob {
    UiFanout {
        target_device_ids: Vec<DeviceId>,
        envelope: Envelope,
    },
    SocialFanout {
        target_account_ids: Vec<String>,
        envelope: Envelope,
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

#[derive(Clone)]
pub struct RealtimeFanout {
    registry: Arc<SessionRegistry>,
    subscription_mgr: Arc<SubscriptionManager>,
    store: StoreHandle,
    bus: MessageBusBackend,
    instance_id: String,
    outbox_worker_id: String,
    jobs: mpsc::Sender<RealtimeJob>,
}

impl RealtimeFanout {
    #[must_use]
    pub fn new(
        registry: Arc<SessionRegistry>,
        subscription_mgr: Arc<SubscriptionManager>,
        store: StoreHandle,
        bus: MessageBusBackend,
        instance_id: String,
        enable_outbox_worker: bool,
    ) -> Arc<Self> {
        let (jobs, rx) = mpsc::channel(REALTIME_WORKER_CAPACITY);
        let outbox_worker_id = format!("realtime-outbox-{instance_id}");
        let realtime = Arc::new(Self {
            registry,
            subscription_mgr,
            store,
            bus,
            instance_id,
            outbox_worker_id,
            jobs,
        });
        Self::spawn_worker(Arc::clone(&realtime), rx);
        if enable_outbox_worker {
            Self::spawn_outbox_dispatcher(Arc::clone(&realtime));
        }
        realtime
    }

    pub fn spawn_listener(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        self.bus.spawn_listener(Arc::clone(self))
    }

    pub async fn fanout_ui_event(&self, target_device_ids: &[DeviceId], envelope: &Envelope) {
        let job = RealtimeJob::UiFanout {
            target_device_ids: target_device_ids.to_vec(),
            envelope: envelope.clone(),
        };
        if let Err(error) = self.jobs.try_send(job.clone()) {
            tracing::warn!(
                target: "minos_backend::realtime",
                error = ?error,
                target_count = target_device_ids.len(),
                "realtime worker queue full; processing ui fanout inline"
            );
            self.process_job(job).await;
        }
    }

    pub async fn fanout_social_message(&self, target_account_ids: &[String], envelope: &Envelope) {
        let job = RealtimeJob::SocialFanout {
            target_account_ids: target_account_ids.to_vec(),
            envelope: envelope.clone(),
        };
        if let Err(error) = self.jobs.try_send(job.clone()) {
            tracing::warn!(
                target: "minos_backend::realtime",
                error = ?error,
                target_count = target_account_ids.len(),
                "realtime worker queue full; processing social fanout inline"
            );
            self.process_job(job).await;
        }
    }

    fn spawn_worker(realtime: Arc<Self>, mut rx: mpsc::Receiver<RealtimeJob>) {
        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                realtime.process_job(job).await;
            }
        });
    }

    fn spawn_outbox_dispatcher(realtime: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                match realtime.dispatch_outbox_batch().await {
                    Ok(0) => tokio::time::sleep(OUTBOX_IDLE_DELAY).await,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            target: "minos_backend::realtime",
                            error = %error,
                            "realtime outbox dispatcher iteration failed"
                        );
                        tokio::time::sleep(OUTBOX_IDLE_DELAY).await;
                    }
                }
            }
        });
    }

    async fn process_job(&self, job: RealtimeJob) {
        match job {
            RealtimeJob::UiFanout {
                target_device_ids,
                envelope,
            } => {
                self.broadcast_ui_event_local(&target_device_ids, envelope.clone());
                if let Err(error) = self
                    .bus
                    .publish(&ClusterEvent::UiFanout {
                        origin_instance_id: self.instance_id.clone(),
                        target_device_ids,
                        envelope,
                    })
                    .await
                {
                    tracing::warn!(
                        target: "minos_backend::realtime",
                        error = %error,
                        "failed to publish cluster ui fanout event"
                    );
                }
            }
            RealtimeJob::SocialFanout {
                target_account_ids,
                envelope,
            } => {
                self.broadcast_social_message_local(&target_account_ids, envelope.clone());
                if let Err(error) = self
                    .bus
                    .publish(&ClusterEvent::SocialFanout {
                        origin_instance_id: self.instance_id.clone(),
                        target_account_ids,
                        envelope,
                    })
                    .await
                {
                    tracing::warn!(
                        target: "minos_backend::realtime",
                        error = %error,
                        "failed to publish cluster social fanout event"
                    );
                }
            }
        }
    }

    pub async fn dispatch_outbox_batch(&self) -> Result<usize, BackendError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let claimed = outbox_events::claim_available(
            &self.store,
            &self.outbox_worker_id,
            now_ms,
            OUTBOX_DISPATCH_BATCH_SIZE,
        )
        .await?;
        let count = claimed.len();
        for row in claimed {
            self.dispatch_outbox_row(row).await;
        }
        Ok(count)
    }

    async fn dispatch_outbox_row(&self, row: outbox_events::OutboxEventRow) {
        let durable =
            match durable_event_log::get(&self.store, &row.topic_kind, &row.event_id).await {
                Ok(Some(durable)) => durable,
                Ok(None) => {
                    self.dead_letter_outbox_row(&row, "missing durable event")
                        .await;
                    return;
                }
                Err(error) => {
                    self.requeue_outbox_row(&row, &error.to_string()).await;
                    return;
                }
            };

        if let Err(error) = self.publish_durable_row(&durable).await {
            self.requeue_outbox_row(&row, &error.to_string()).await;
            return;
        }

        let ack_at_ms = chrono::Utc::now().timestamp_millis();
        match outbox_events::ack(&self.store, &row.outbox_id, ack_at_ms).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    outbox_id = %row.outbox_id,
                    "realtime outbox dispatcher lost claimed row before ack"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    error = %error,
                    outbox_id = %row.outbox_id,
                    "realtime outbox dispatcher failed to ack claimed row"
                );
            }
        }
    }

    async fn publish_durable_row(
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
        self.broadcast_durable_event_local(
            &topic,
            &row.event_id,
            wire::ServerFrame::DurableEvent {
                topic: row.topic.clone(),
                topic_seq: row.topic_seq,
                kind,
                payload,
                event_id: row.event_id.clone(),
            },
        );
        Ok(())
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
            ClusterEvent::UiFanout {
                origin_instance_id,
                target_device_ids,
                envelope,
            } => {
                if origin_instance_id == self.instance_id {
                    return;
                }
                self.broadcast_ui_event_local(&target_device_ids, envelope);
            }
            ClusterEvent::SocialFanout {
                origin_instance_id,
                target_account_ids,
                envelope,
            } => {
                if origin_instance_id == self.instance_id {
                    return;
                }
                self.broadcast_social_message_local(&target_account_ids, envelope);
            }
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
                self.broadcast_durable_event_local(
                    &parsed_topic,
                    &event_id,
                    wire::ServerFrame::DurableEvent {
                        topic,
                        topic_seq,
                        kind: event_kind,
                        payload,
                        event_id: event_id.clone(),
                    },
                );
            }
        }
    }

    fn broadcast_ui_event_local(&self, target_device_ids: &[DeviceId], envelope: Envelope) {
        for device_id in target_device_ids {
            let Some(handle) = self.registry.get(*device_id) else {
                continue;
            };
            if let Err(error) = self.registry.try_send_current(&handle, envelope.clone()) {
                crate::telemetry::increment_ingest_outbox_dropped();
                tracing::warn!(
                    target: "minos_backend::realtime",
                    peer = %device_id,
                    error = ?error,
                    "peer outbox full or superseded during realtime ui fanout"
                );
            }
        }
    }

    fn broadcast_durable_event_local(
        &self,
        topic: &RealtimeTopic,
        event_id: &str,
        frame: wire::ServerFrame,
    ) {
        for target in self.subscription_mgr.fanout_targets(topic) {
            if !target.remember_durable_event(event_id) {
                continue;
            }
            if let Err(error) = target.send(frame.clone()) {
                tracing::warn!(
                    target: "minos_backend::realtime",
                    conn_id = %target.conn_id,
                    topic = %topic.topic_string(),
                    error = ?error,
                    "formal gateway durable fanout dropped frame"
                );
            }
        }
    }

    fn broadcast_social_message_local(&self, target_account_ids: &[String], envelope: Envelope) {
        for account_id in target_account_ids {
            let _ = self
                .registry
                .broadcast_mobile_account(account_id, envelope.clone());
        }
    }
}

fn durable_event_kind_payload(value: &Value) -> (String, Value) {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let mut payload = value.clone();
    if let Value::Object(map) = &mut payload {
        map.remove("kind");
    }
    (kind, payload)
}
