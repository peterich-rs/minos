use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use futures::StreamExt;
use minos_domain::DeviceId;
use minos_protocol::Envelope;
use moka::sync::Cache;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::BackendError;
use crate::session::SessionRegistry;

const DEFAULT_PEER_TARGET_CACHE_TTL: Duration = Duration::from_secs(5);
const CLUSTER_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const REALTIME_WORKER_CAPACITY: usize = 1024;

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
    bus: MessageBusBackend,
    instance_id: String,
    jobs: mpsc::Sender<RealtimeJob>,
}

impl RealtimeFanout {
    #[must_use]
    pub fn new(
        registry: Arc<SessionRegistry>,
        bus: MessageBusBackend,
        instance_id: String,
    ) -> Arc<Self> {
        let (jobs, rx) = mpsc::channel(REALTIME_WORKER_CAPACITY);
        let realtime = Arc::new(Self {
            registry,
            bus,
            instance_id,
            jobs,
        });
        Self::spawn_worker(Arc::clone(&realtime), rx);
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

    fn broadcast_social_message_local(&self, target_account_ids: &[String], envelope: Envelope) {
        for account_id in target_account_ids {
            let _ = self
                .registry
                .broadcast_mobile_account(account_id, envelope.clone());
        }
    }
}
