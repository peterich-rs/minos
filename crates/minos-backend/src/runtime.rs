use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderValue;
use serde::Serialize;
use tokio::task::JoinHandle;

use crate::agent_sessions::{AgentSessionService, DefaultAgentSessionService};
use crate::app::context::AppDataContext;
use crate::approvals::{ApprovalService, DefaultApprovalService};
use crate::auth::host_bootstrap::BootstrapNonceStore;
use crate::auth::realtime_ticket::RealtimeTicketStore;
use crate::auth::supabase::SupabaseTokenVerifier;
use crate::auth::use_case::AuthUseCase;
use crate::config::{
    Config, Environment, RuntimeMode, StorageMode, DEFAULT_CLUSTER_CHANNEL,
    DEFAULT_DB_MAX_CONNECTIONS, DEFAULT_TOKEN_TTL_SECS,
};
use crate::host_commands::{HostCommandService, RuntimeHostCommandService};
use crate::host_link::HostLinkService;
use crate::http::{self, BackendState, RouteContract};
use crate::ingest::{translate::SessionTranslators, use_case::IngestUseCase};
use crate::notifications::channels::composite::CompositeChannel;
use crate::notifications::use_case::DefaultNotificationService;
use crate::notifications::NotificationService;
use crate::project::ProjectService;
use crate::realtime::{
    configure_peer_target_cache, CacheBackendKind, MessageBusBackend, MessageBusBackendKind,
    PeerTargetCacheBackend, RealtimeFanout, SubscriptionManager,
};
use crate::session::SessionRegistry;
use crate::store::{self, StoreHandle};

#[derive(Debug, Clone, Serialize)]
pub struct AppRuntimeConfig {
    pub environment: Environment,
    pub storage_mode: StorageMode,
    pub runtime_mode: RuntimeMode,
    pub cache_backend: CacheBackendKind,
    pub message_bus_backend: MessageBusBackendKind,
    pub db_max_connections: u32,
    pub token_ttl_secs: u64,
    pub cluster_channel: String,
}

impl From<&Config> for AppRuntimeConfig {
    fn from(cfg: &Config) -> Self {
        Self {
            environment: cfg.environment,
            storage_mode: cfg.storage_mode,
            runtime_mode: cfg.runtime_mode,
            cache_backend: cfg.cache_backend,
            message_bus_backend: cfg.message_bus_backend,
            db_max_connections: cfg.db_max_connections,
            token_ttl_secs: cfg.token_ttl_secs,
            cluster_channel: cfg.cluster_channel.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BackendPlatformContract {
    pub service: &'static str,
    pub version: &'static str,
    pub runtime_modes: Vec<String>,
    pub storage_modes: Vec<String>,
    pub external_sql: ExternalSqlContract,
    pub cache_backends: Vec<String>,
    pub message_bus_backends: Vec<String>,
    pub defaults: BackendPlatformDefaults,
    pub prod_guards: Vec<&'static str>,
    pub routes: Vec<RouteContract>,
}

#[derive(Debug, Serialize)]
pub struct ExternalSqlContract {
    pub supported_drivers: Vec<String>,
    pub boot_capabilities: Vec<&'static str>,
    pub runtime_blockers: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct BackendPlatformDefaults {
    pub environment: &'static str,
    pub storage_mode: &'static str,
    pub runtime_mode: &'static str,
    pub db_max_connections: u32,
    pub token_ttl_secs: u64,
    pub cache_backend: &'static str,
    pub message_bus_backend: &'static str,
    pub cluster_channel: &'static str,
}

pub struct AppContext {
    pub config: Arc<AppRuntimeConfig>,
    pub data: AppDataContext,
    pub registry: Arc<SessionRegistry>,
    pub subscription_mgr: Arc<SubscriptionManager>,
    pub host_link: Arc<HostLinkService>,
    pub agent_sessions: Arc<dyn AgentSessionService>,
    pub approvals: Arc<dyn ApprovalService>,
    pub auth: Arc<AuthUseCase>,
    pub bootstrap_nonces: Arc<BootstrapNonceStore>,
    pub host_commands: Arc<dyn HostCommandService>,
    pub projects: Arc<ProjectService>,
    pub store: StoreHandle,
    pub token_ttl: Duration,
    pub ingest: Arc<IngestUseCase>,
    /// Per-session agent translators for host live ingest (raw → UiEventMessage).
    /// Shared with [`IngestUseCase`] so legacy Envelope::Ingest and HostIngest
    /// share stateful projection.
    pub translators: Arc<SessionTranslators>,
    pub realtime: Arc<RealtimeFanout>,
    pub notifications: Arc<dyn NotificationService>,
    /// Agent turn completion watches (ingest-driven TurnCompletionProjector).
    pub completion_watches: Arc<crate::completion_watch::CompletionWatchRegistry>,
    pub instance_id: String,
    /// In-process wake for outbox_dispatcher (post-commit notify_one).
    pub outbox_wake: Arc<tokio::sync::Notify>,
    /// In-process wake for agent_dispatch_worker (enqueue + host online).
    pub agent_dispatch_wake: Arc<tokio::sync::Notify>,
}

impl AppContext {
    /// Wake outbox_dispatcher after a successful commit that enqueued outbox rows.
    pub fn wake_outbox(&self) {
        self.outbox_wake.notify_one();
    }

    /// Wake agent_dispatch_worker after enqueue or host online.
    pub fn wake_agent_dispatch(&self) {
        self.agent_dispatch_wake.notify_one();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        runtime_config: AppRuntimeConfig,
        registry: Arc<SessionRegistry>,
        host_link: Arc<HostLinkService>,
        store: StoreHandle,
        token_ttl: Duration,
        jwt_secret: String,
        instance_id: String,
        message_bus: MessageBusBackend,
        peer_target_cache: PeerTargetCacheBackend,
        realtime_tickets: Arc<RealtimeTicketStore>,
        supabase: Option<Arc<SupabaseTokenVerifier>>,
        bootstrap_nonces: Option<Arc<BootstrapNonceStore>>,
    ) -> Arc<Self> {
        let translators = SessionTranslators::new();
        let data = AppDataContext::new(store.clone());
        configure_peer_target_cache(peer_target_cache);
        let subscription_mgr = Arc::new(SubscriptionManager::default());
        let run_workers = runtime_config.runtime_mode.runs_supervised_workers();
        // Build notification service before realtime so durable fanout can
        // trigger push as a secondary side effect without owning the outbox.
        let composite_channel = CompositeChannel::from_env();
        let notifications: Arc<dyn NotificationService> = Arc::new(
            DefaultNotificationService::new(
                store.clone(),
                vec![Arc::new(composite_channel)],
                Arc::clone(&registry) as Arc<dyn crate::notifications::PresencePort>,
            ),
        );
        let realtime = RealtimeFanout::new(
            Arc::clone(&registry),
            Arc::clone(&subscription_mgr),
            store.clone(),
            message_bus,
            instance_id.clone(),
            Some(Arc::clone(&notifications)),
        );
        let host_commands: Arc<dyn HostCommandService> =
            RuntimeHostCommandService::new_with_timeout_worker_and_realtime(
                store.clone(),
                Some(Arc::clone(&registry)),
                run_workers,
                Some(Arc::clone(&realtime)),
            );
        let approvals: Arc<dyn ApprovalService> = DefaultApprovalService::new(
            Arc::clone(&data.repos),
            store.clone(),
            Arc::clone(&registry),
            Arc::clone(&host_commands),
            run_workers,
        );
        let ingest = IngestUseCase::new(
            store.clone(),
            Arc::clone(&registry),
            Arc::clone(&translators),
            Arc::clone(&approvals),
            Arc::clone(&realtime),
        );
        let auth = AuthUseCase::new_with_realtime_tickets_and_supabase(
            store.clone(),
            jwt_secret,
            realtime_tickets,
            supabase,
        );
        let bootstrap_nonces =
            bootstrap_nonces.unwrap_or_else(|| Arc::new(BootstrapNonceStore::in_memory()));
        let projects = ProjectService::new(store.clone());
        let agent_sessions: Arc<dyn AgentSessionService> = DefaultAgentSessionService::new(
            Arc::clone(&data.repos),
            store.clone(),
            Arc::clone(&host_commands),
        );
        Arc::new(Self {
            config: Arc::new(runtime_config),
            data,
            registry,
            subscription_mgr,
            host_link,
            agent_sessions,
            approvals,
            auth,
            bootstrap_nonces,
            host_commands,
            projects,
            store,
            token_ttl,
            ingest,
            translators,
            realtime,
            notifications,
            completion_watches: Arc::new(crate::completion_watch::CompletionWatchRegistry::new()),
            instance_id,
            outbox_wake: Arc::new(tokio::sync::Notify::new()),
            agent_dispatch_wake: Arc::new(tokio::sync::Notify::new()),
        })
    }
}

pub struct RuntimeShell {
    pub app: Arc<AppContext>,
    cors_origins: Option<Vec<HeaderValue>>,
    cluster_listener: Option<JoinHandle<()>>,
    job_supervisor: Option<crate::jobs::JobSupervisor>,
}

impl RuntimeShell {
    pub fn from_config(
        cfg: &Config,
        store: StoreHandle,
        jwt_secret: String,
        cors_origins: Option<Vec<HeaderValue>>,
    ) -> Result<Self, crate::error::BackendError> {
        let registry = Arc::new(SessionRegistry::new());
        let host_link = Arc::new(HostLinkService::new(store.clone()));
        let instance_id = uuid::Uuid::new_v4().to_string();
        let redis_url = cfg.redis_url.as_deref().unwrap_or_default();
        let message_bus = match cfg.message_bus_backend {
            MessageBusBackendKind::Inline => MessageBusBackend::inline(),
            MessageBusBackendKind::Redis => {
                MessageBusBackend::redis(redis_url, cfg.cluster_channel.clone())?
            }
        };
        let peer_target_cache = match cfg.cache_backend {
            CacheBackendKind::InMemory => PeerTargetCacheBackend::in_memory(Duration::from_secs(5)),
            CacheBackendKind::Redis => {
                PeerTargetCacheBackend::redis(redis_url, Duration::from_secs(5))?
            }
        };
        let run_workers = cfg.runtime_mode.runs_supervised_workers();
        let realtime_tickets = match cfg.redis_url.as_deref() {
            Some(redis_url) if !redis_url.is_empty() => {
                Arc::new(RealtimeTicketStore::redis(redis_url)?)
            }
            _ => Arc::new(RealtimeTicketStore::default()),
        };
        let bootstrap_nonces = match cfg.redis_url.as_deref() {
            Some(redis_url) if !redis_url.is_empty() => {
                Arc::new(BootstrapNonceStore::redis(redis_url)?)
            }
            _ => Arc::new(BootstrapNonceStore::in_memory()),
        };
        let app = AppContext::compose(
            AppRuntimeConfig::from(cfg),
            registry,
            host_link,
            store,
            cfg.token_ttl(),
            jwt_secret,
            instance_id,
            message_bus,
            peer_target_cache,
            realtime_tickets,
            cfg.supabase_verifier(),
            Some(bootstrap_nonces),
        );
        let cluster_listener = if cfg.runtime_mode.serves_http() {
            app.realtime.spawn_listener()
        } else {
            None
        };
        let job_supervisor = if run_workers {
            let ctx = Arc::new(crate::jobs::JobContext {
                store: app.store.clone(),
                instance_id: app.instance_id.clone(),
                outbox_wake: Arc::clone(&app.outbox_wake),
                agent_dispatch_wake: Arc::clone(&app.agent_dispatch_wake),
            });
            let jobs = crate::jobs::default_jobs(
                Some(Arc::clone(&app.realtime)),
                Some(Arc::clone(&app)),
            );
            Some(crate::jobs::JobSupervisor::spawn_all(
                jobs,
                ctx,
                cfg.runtime_mode,
            ))
        } else {
            None
        };
        Ok(Self {
            app,
            cors_origins,
            cluster_listener,
            job_supervisor,
        })
    }

    #[must_use]
    pub fn backend_state(&self) -> BackendState {
        BackendState::from_app_context(
            Arc::clone(&self.app),
            self.cors_origins.clone(),
            env!("CARGO_PKG_VERSION"),
        )
    }

    pub async fn shutdown(mut self) {
        if let Some(supervisor) = self.job_supervisor.take() {
            supervisor.abort_all();
        }
        if let Some(task) = self.cluster_listener.take() {
            task.abort();
        }
        self.app.store.close().await;
    }
}

#[must_use]
pub fn platform_contract_snapshot() -> BackendPlatformContract {
    BackendPlatformContract {
        service: "minos-backend",
        version: env!("CARGO_PKG_VERSION"),
        runtime_modes: enum_names::<RuntimeMode>(),
        storage_modes: store::compiled_storage_modes(),
        external_sql: ExternalSqlContract {
            supported_drivers: store::supported_external_sql_drivers(),
            boot_capabilities: vec![
                "Validates postgres:// and postgresql:// URLs during config parsing",
                "Opens a real sqlx Postgres pool for the runtime store handle",
                "Queries current_database() and version() for boot diagnostics",
                "Serves /health/* and /metrics against the live external SQL pool",
                "Mounts the full /v1 route tree instead of a reduced external-sql-only router",
                "Serves /ws/client and /ws/host for realtime ticket auth, session activation, forwarding, regular ingest persistence, and UI fanout",
            ],
            runtime_blockers: vec![
                "Most remaining /v1 handlers plus approval-driven realtime flows still rely on SQLite-only services and store queries",
                "Many store modules under crates/minos-backend/src/store still use SQLite-specific SQL placeholders and transactions",
                "The embedded migration set under crates/minos-backend/migrations is SQLite-only",
            ],
        },
        cache_backends: enum_names::<CacheBackendKind>(),
        message_bus_backends: enum_names::<MessageBusBackendKind>(),
        defaults: BackendPlatformDefaults {
            environment: Environment::Dev.as_str(),
            storage_mode: StorageMode::Sqlite.as_str(),
            runtime_mode: RuntimeMode::Monolith.as_str(),
            db_max_connections: DEFAULT_DB_MAX_CONNECTIONS,
            token_ttl_secs: DEFAULT_TOKEN_TTL_SECS,
            cache_backend: "in-memory",
            message_bus_backend: "inline",
            cluster_channel: DEFAULT_CLUSTER_CHANNEL,
        },
        prod_guards: vec![
            "MINOS_STORAGE_MODE must not remain sqlite in prod",
            "MINOS_CORS_ORIGINS must not be wildcard for HTTP-serving prod nodes",
            "MINOS_CACHE_BACKEND must be redis for HTTP-serving prod nodes",
            "MINOS_MESSAGE_BUS_BACKEND must be redis for HTTP-serving prod nodes",
            "MINOS_REDIS_URL is required whenever redis runtime adapters are selected",
            "MINOS_STORAGE_MODE=external-sql now mounts the full /v1 route tree and admits regular /ws traffic; unported handlers must fail at their own surface boundary instead of the top-level router",
        ],
        routes: http::formal_route_inventory().to_vec(),
    }
}

fn enum_names<T: clap::ValueEnum>() -> Vec<String> {
    T::value_variants()
        .iter()
        .filter_map(|value| {
            value
                .to_possible_value()
                .map(|name| name.get_name().to_string())
        })
        .collect()
}
