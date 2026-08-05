//! HTTP surface: axum `Router` + shared state + header extraction helpers.
//!
//! The backend exposes the health probe set plus the formal realtime upgrades:
//!
//! - `GET /health/live`, `GET /health/ready`, `GET /health/info`.
//! - `GET /ws/client`, `GET /ws/host` — WebSocket upgrades for the formal
//!   realtime topic gateway.
//!
//! # State plumbing
//!
//! [`BackendState`] bundles the three runtime Arcs (`SessionRegistry`,
//! `HostLinkService`, `SqlitePool`) plus the backend version string. It is
//! [`Clone`] so axum's [`axum::extract::State`] can hand it to every
//! handler without borrowing; inner fields are either `Arc`-wrapped,
//! cheap-to-clone (`SqlitePool`), or `&'static str`.
//!
//! # Header extraction strategy
//!
//! We use [`axum::http::HeaderMap`] with small typed parsing helpers rather
//! than per-header `TypedHeader` extractors. The custom headers
//! (`X-Device-Id`, `X-Device-Role`, `X-Device-Secret`) all parse to
//! domain newtypes that already own their own `FromStr` / kebab-case
//! mapping; threading them through `TypedHeader` would require a
//! per-header adapter struct for minimal payoff.
//!
//! Extraction errors return `(StatusCode, String)` tuples so the plan's
//! "401 pre-upgrade" contract stays easy to read at the call site.

use std::{ops::Deref, sync::Arc, time::Duration};

use axum::extract::{MatchedPath, State};
use axum::http::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    HeaderMap, HeaderName, HeaderValue, Method, Request,
};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::response::Response;
use axum::Router;
use sqlx::SqlitePool;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::trace::TraceLayer;

use crate::{
    host_link::HostLinkService,
    realtime::{MessageBusBackend, PeerTargetCacheBackend},
    runtime::{AppContext, AppRuntimeConfig},
    session::SessionRegistry,
};

pub mod auth;
pub mod error_response;
pub mod health;
pub mod metrics;
pub mod openapi;
pub mod rate_limit;
pub mod v1;

fn x_request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

#[derive(Clone, Default)]
struct MakeRequestUuid;

impl MakeRequestId for MakeRequestUuid {
    fn make_request_id<B>(&mut self, request: &Request<B>) -> Option<RequestId> {
        request
            .headers()
            .get(x_request_id_header())
            .cloned()
            .or_else(|| HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()).ok())
            .map(RequestId::new)
    }
}

/// Shared state for every HTTP handler.
///
/// Cheap to clone: the transport shell only owns an [`Arc<AppContext>`], the
/// parsed CORS policy, and the version string. The runtime services themselves
/// live behind [`AppContext`], keeping handler state aligned with the runtime
/// shell instead of duplicating ad-hoc wiring in each composition root.
#[derive(Clone)]
pub struct BackendState {
    app: Arc<AppContext>,
    /// Parsed CORS origins from config. `None` means allow-all (dev mode).
    pub cors_origins: Option<Vec<HeaderValue>>,
    /// Crate version string; exposed via the health endpoints.
    ///
    /// Stored here rather than read from `env!("CARGO_PKG_VERSION")` at the
    /// handler so tests can substitute a fixed value without reaching into
    /// proc-macros.
    pub version: &'static str,
}

impl Deref for BackendState {
    type Target = AppContext;

    fn deref(&self) -> &Self::Target {
        self.app.as_ref()
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct RouteContract {
    pub method: &'static str,
    pub path: &'static str,
    pub probe_path: &'static str,
    pub surface: &'static str,
    pub auth: &'static str,
}

impl RouteContract {
    const fn new(
        method: &'static str,
        path: &'static str,
        probe_path: &'static str,
        surface: &'static str,
        auth: &'static str,
    ) -> Self {
        Self {
            method,
            path,
            probe_path,
            surface,
            auth,
        }
    }
}

const ROUTE_INVENTORY: &[RouteContract] = &[
    RouteContract::new("GET", "/health/live", "/health/live", "platform", "public"),
    RouteContract::new(
        "GET",
        "/health/ready",
        "/health/ready",
        "platform",
        "public",
    ),
    RouteContract::new("GET", "/health/info", "/health/info", "platform", "public"),
    RouteContract::new("GET", "/health/jobs", "/health/jobs", "platform", "public"),
    RouteContract::new("GET", "/metrics", "/metrics", "platform", "public"),
    RouteContract::new(
        "GET",
        "/openapi.json",
        "/openapi.json",
        "platform",
        "public",
    ),
    RouteContract::new(
        "GET",
        "/ws/client",
        "/ws/client",
        "client_gateway",
        "realtime_ticket",
    ),
    RouteContract::new(
        "GET",
        "/ws/host",
        "/ws/host",
        "host_gateway",
        "realtime_ticket",
    ),
    RouteContract::new(
        "POST",
        "/v1/approvals/respond",
        "/v1/approvals/respond",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agent-sessions/start",
        "/v1/agent-sessions/start",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agent-sessions/send-input",
        "/v1/agent-sessions/send-input",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agent-sessions/stop",
        "/v1/agent-sessions/stop",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agent-sessions/list",
        "/v1/agent-sessions/list",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agent-sessions/read-turns",
        "/v1/agent-sessions/read-turns",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/auth/refresh",
        "/v1/auth/refresh",
        "account_api",
        "public",
    ),
    RouteContract::new(
        "POST",
        "/v1/auth/logout",
        "/v1/auth/logout",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/auth/supabase",
        "/v1/auth/supabase",
        "account_api",
        "public",
    ),
    RouteContract::new(
        "POST",
        "/v1/host/bootstrap/nonce",
        "/v1/host/bootstrap/nonce",
        "host_api",
        "host_bootstrap",
    ),
    RouteContract::new(
        "POST",
        "/v1/hosts/link",
        "/v1/hosts/link",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/hosts/unlink",
        "/v1/hosts/unlink",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "GET",
        "/v1/hosts",
        "/v1/hosts",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/host/installations/self",
        "/v1/host/installations/self",
        "host_api",
        "host_installation",
    ),
    RouteContract::new(
        "POST",
        "/v1/host-commands/list-clis",
        "/v1/host-commands/list-clis",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/host-commands/list-host-skills",
        "/v1/host-commands/list-host-skills",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/host-commands/list-workspaces",
        "/v1/host-commands/list-workspaces",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/host-commands/write-host-skill-config",
        "/v1/host-commands/write-host-skill-config",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/host/realtime/ws-ticket",
        "/v1/host/realtime/ws-ticket",
        "host_api",
        "host_installation",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects",
        "/v1/projects",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/create",
        "/v1/projects/create",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/query",
        "/v1/projects/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/list",
        "/v1/projects/list",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/update",
        "/v1/projects/update",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/rename",
        "/v1/projects/rename",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/delete",
        "/v1/projects/delete",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "DELETE",
        "/v1/projects/:project_id",
        "/v1/projects/proj_probe",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/link-conversation",
        "/v1/projects/link-conversation",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/conversations/link",
        "/v1/projects/conversations/link",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/agent-sessions/link",
        "/v1/projects/agent-sessions/link",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/projects/agent-sessions/query",
        "/v1/projects/agent-sessions/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/realtime/ws-ticket",
        "/v1/realtime/ws-ticket",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/profiles/self",
        "/v1/profiles/self",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/profiles/minos-id",
        "/v1/profiles/minos-id",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/profiles/display-name",
        "/v1/profiles/display-name",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/profiles/search",
        "/v1/profiles/search",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/friends/query",
        "/v1/friends/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/friend-requests",
        "/v1/friend-requests",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/friend-requests/query",
        "/v1/friend-requests/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/friend-requests/:request_id/accept",
        "/v1/friend-requests/request_probe/accept",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/friend-requests/:request_id/reject",
        "/v1/friend-requests/request_probe/reject",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/query",
        "/v1/conversations/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/list",
        "/v1/conversations/list",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/direct",
        "/v1/conversations/direct",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/group",
        "/v1/conversations/group",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "DELETE",
        "/v1/conversations/:conversation_id",
        "/v1/conversations/conv_probe",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/members/query",
        "/v1/conversations/conv_probe/members/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/read",
        "/v1/conversations/conv_probe/read",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/messages",
        "/v1/conversations/conv_probe/messages",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/send-message",
        "/v1/conversations/send-message",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/messages/query",
        "/v1/conversations/conv_probe/messages/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/messages/:message_id/recall",
        "/v1/conversations/conv_probe/messages/msg_probe/recall",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agents",
        "/v1/agents",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agents/query",
        "/v1/agents/query",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/agents/:agent_id/delete",
        "/v1/agents/agent_probe/delete",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/members/add",
        "/v1/conversations/conv_probe/members/add",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/agents",
        "/v1/conversations/conv_probe/agents",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/agents/add",
        "/v1/conversations/conv_probe/agents/add",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/agents/remove",
        "/v1/conversations/conv_probe/agents/remove",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/conversations/:conversation_id/agents/message",
        "/v1/conversations/conv_probe/agents/message",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/notifications/tokens/register",
        "/v1/notifications/tokens/register",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/notifications/tokens/unregister",
        "/v1/notifications/tokens/unregister",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/notifications/tokens/list",
        "/v1/notifications/tokens/list",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/notifications/preferences/get",
        "/v1/notifications/preferences/get",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/notifications/preferences/update",
        "/v1/notifications/preferences/update",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "GET",
        "/v1/media/status",
        "/v1/media/status",
        "account_api",
        "public",
    ),
    RouteContract::new(
        "POST",
        "/v1/media/blobs",
        "/v1/media/blobs",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/media/blobs/get",
        "/v1/media/blobs/get",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "POST",
        "/v1/media/blobs/delete",
        "/v1/media/blobs/delete",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "PUT",
        "/v1/media/blobs/:blob_id/content",
        "/v1/media/blobs/test-blob/content",
        "account_api",
        "account_bearer",
    ),
    RouteContract::new(
        "GET",
        "/v1/media/blobs/:blob_id/content",
        "/v1/media/blobs/test-blob/content",
        "account_api",
        "account_bearer_or_download_token",
    ),
];

impl BackendState {
    /// Construct a state bundle with the crate's `CARGO_PKG_VERSION`.
    ///
    /// Intended call site: `main.rs` in step 10. Tests that need a custom
    /// version string can build the struct literally.
    #[must_use]
    pub fn new(
        registry: Arc<SessionRegistry>,
        host_link: Arc<HostLinkService>,
        store: SqlitePool,
        token_ttl: Duration,
        jwt_secret: String,
        cors_origins: Option<Vec<HeaderValue>>,
        instance_id: String,
    ) -> Self {
        Self::new_with_runtime(
            registry,
            host_link,
            store,
            token_ttl,
            jwt_secret,
            cors_origins,
            instance_id,
            MessageBusBackend::inline(),
            PeerTargetCacheBackend::in_memory(Duration::from_secs(5)),
        )
    }

    #[must_use]
    pub fn from_app_context(
        app: Arc<AppContext>,
        cors_origins: Option<Vec<HeaderValue>>,
        version: &'static str,
    ) -> Self {
        Self {
            app,
            cors_origins,
            version,
        }
    }

    #[must_use]
    pub fn new_with_runtime(
        registry: Arc<SessionRegistry>,
        host_link: Arc<HostLinkService>,
        store: SqlitePool,
        token_ttl: Duration,
        jwt_secret: String,
        cors_origins: Option<Vec<HeaderValue>>,
        instance_id: String,
        message_bus: MessageBusBackend,
        peer_target_cache: PeerTargetCacheBackend,
    ) -> Self {
        let app = AppContext::compose(
            AppRuntimeConfig {
                environment: crate::config::Environment::Dev,
                storage_mode: crate::config::StorageMode::Sqlite,
                runtime_mode: crate::config::RuntimeMode::Monolith,
                cache_backend: crate::realtime::CacheBackendKind::InMemory,
                message_bus_backend: crate::realtime::MessageBusBackendKind::Inline,
                db_max_connections: 1,
                token_ttl_secs: u64::try_from(token_ttl.as_secs()).unwrap_or(u64::MAX),
                cluster_channel: crate::config::DEFAULT_CLUSTER_CHANNEL.to_string(),
            },
            registry,
            host_link,
            store.into(),
            token_ttl,
            jwt_secret,
            instance_id,
            message_bus,
            peer_target_cache,
            Arc::new(crate::auth::realtime_ticket::RealtimeTicketStore::default()),
            None,
            None,
        );
        Self::from_app_context(app, cors_origins, env!("CARGO_PKG_VERSION"))
    }
}

#[must_use]
pub fn formal_route_inventory() -> &'static [RouteContract] {
    ROUTE_INVENTORY
}

/// Parse the `--cors-origins` / `MINOS_CORS_ORIGINS` config string into
/// a list of `HeaderValue`s. Returns `None` for wildcard (allow-all).
#[must_use]
pub fn parse_cors_origins(raw: &str) -> Option<Vec<HeaderValue>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return None;
    }
    let origins: Vec<HeaderValue> = trimmed
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                HeaderValue::from_str(s).ok()
            }
        })
        .collect();
    if origins.is_empty() {
        None
    } else {
        Some(origins)
    }
}

/// Build the backend's top-level axum `Router`.
///
/// Includes request-id propagation, tracing, and Prometheus metrics.
pub fn router(state: BackendState) -> Router {
    crate::telemetry::init();
    crate::telemetry::set_session_registry_size(state.registry.len());

    // Initialize rate limiter for sensitive endpoints.
    let _rate_limiter = rate_limit::RateLimiter::from_env();
    tracing::info!(
        target: "minos_backend::startup",
        "rate limiter initialized for sensitive endpoints"
    );

    let cors = cors_layer(state.cors_origins.clone());
    let is_sqlite = state.store.is_sqlite();
    let router = Router::new()
        .route("/health/live", axum::routing::get(health::live))
        .route("/health/ready", axum::routing::get(health::ready))
        .route("/health/info", axum::routing::get(health::info))
        .route("/health/jobs", axum::routing::get(health::jobs))
        .route("/metrics", axum::routing::get(metrics::get))
        .route(
            "/openapi.json",
            axum::routing::get(openapi::serve_openapi_json),
        )
        .route(
            "/ws/client",
            axum::routing::get(crate::realtime::gateway::upgrade_client),
        )
        .route(
            "/ws/host",
            axum::routing::get(crate::realtime::gateway::upgrade_host),
        );
    let v1_router = if is_sqlite {
        v1::router()
    } else {
        v1::external_sql_router()
    }
    .layer(from_fn_with_state(state.clone(), touch_account_last_seen));
    let router = router.nest("/v1", v1_router);
    router
        .layer(cors)
        .layer(from_fn(record_http_metrics))
        .layer(PropagateRequestIdLayer::new(x_request_id_header()))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                let matched_path = request
                    .extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str)
                    .unwrap_or_else(|| request.uri().path());
                let request_id = request
                    .headers()
                    .get(x_request_id_header())
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path = %matched_path,
                    request_id = %request_id,
                )
            }),
        )
        .layer(SetRequestIdLayer::new(
            x_request_id_header(),
            MakeRequestUuid,
        ))
        .with_state(state)
}

async fn record_http_metrics(request: Request<axum::body::Body>, next: Next) -> Response {
    let started_at = std::time::Instant::now();
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or_else(|| request.uri().path())
        .to_string();
    let response = next.run(request).await;
    crate::telemetry::record_http_request(
        &route,
        method.as_str(),
        response.status().as_u16(),
        started_at.elapsed().as_secs_f64(),
    );

    response
}

async fn touch_account_last_seen(
    State(state): State<BackendState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if let Some(device_id) = account_device_id_from_headers(&state, request.headers()) {
        touch_device_last_seen(&state, device_id, "http.account_request").await;
    }
    next.run(request).await
}

fn account_device_id_from_headers(
    state: &BackendState,
    headers: &HeaderMap,
) -> Option<minos_domain::DeviceId> {
    let token = bearer_token(headers)?;
    let claims = crate::auth::jwt::verify(state.auth.jwt_secret(), token).ok()?;
    uuid::Uuid::parse_str(&claims.did)
        .map(minos_domain::DeviceId)
        .ok()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
}

async fn touch_device_last_seen(
    state: &BackendState,
    device_id: minos_domain::DeviceId,
    operation: &'static str,
) {
    if let Err(error) = crate::store::device_installations::touch_last_seen(
        &state.store,
        &device_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    {
        tracing::debug!(
            target: "minos_backend::http",
            error = %error,
            device_id = %device_id,
            operation,
            "failed to touch device last_seen_at",
        );
    }
}

fn cors_layer(origins: Option<Vec<HeaderValue>>) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            AUTHORIZATION,
            CONTENT_TYPE,
            HeaderName::from_static(auth::HDR_DEVICE_ID),
            HeaderName::from_static(auth::HDR_DEVICE_ROLE),
            HeaderName::from_static(auth::HDR_DEVICE_SECRET),
            HeaderName::from_static(auth::HDR_DEVICE_NAME),
            x_request_id_header(),
        ]);
    match origins {
        None => layer
            .allow_origin(Any)
            .expose_headers([x_request_id_header()]),
        Some(list) => layer
            .allow_origin(AllowOrigin::list(list))
            .expose_headers([x_request_id_header()]),
    }
}

/// Test scaffolding factories shared by the crate's integration tests.
///
/// Exposed publicly when the `test-support` feature is enabled (and
/// always when compiling tests) so test files under `tests/` and
/// downstream crates' dev-deps can build a ready-to-serve
/// [`BackendState`] backed by an in-memory SQLite pool.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::BackendState;
    use crate::auth::realtime_ticket::RealtimeTicketStore;
    use crate::auth::supabase::SupabaseTokenVerifier;
    use crate::host_link::HostLinkService;
    use crate::realtime::{MessageBusBackend, PeerTargetCacheBackend};
    use crate::runtime::{AppContext, AppRuntimeConfig};
    use crate::session::SessionRegistry;
    use crate::store::test_support::memory_pool;
    use std::sync::Arc;
    use std::time::Duration;

    /// Deterministic 32-byte JWT secret used by every test that needs to
    /// sign/verify a bearer token. Long enough to satisfy
    /// `Config::validate`; the literal is fine because tests never hit a
    /// real network.
    pub const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";

    /// HS256 secret for minting synthetic Supabase tokens in tests.
    pub const TEST_SUPABASE_HMAC: &[u8] = b"test-supabase-hmac-secret-32b!";
    pub const TEST_SUPABASE_ISS: &str = "https://example.supabase.co/auth/v1";
    pub const TEST_SUPABASE_AUD: &str = "authenticated";

    /// Build a `BackendState` against a fresh in-memory pool, with a
    /// 5-minute token TTL and the deterministic test JWT secret.
    pub async fn backend_state() -> BackendState {
        let pool = memory_pool().await;
        let registry = Arc::new(SessionRegistry::new());
        let host_link = Arc::new(HostLinkService::new(pool.clone()));
        BackendState::new(
            registry,
            host_link,
            pool,
            Duration::from_mins(5),
            TEST_JWT_SECRET.to_string(),
            None,
            "test-instance".to_string(),
        )
    }

    /// Like [`backend_state`] but with an HS256 Supabase verifier so
    /// `/v1/auth/supabase` can be exercised without network JWKS.
    pub async fn backend_state_with_supabase() -> BackendState {
        let pool = memory_pool().await;
        let registry = Arc::new(SessionRegistry::new());
        let host_link = Arc::new(HostLinkService::new(pool.clone()));
        let verifier = SupabaseTokenVerifier::for_tests(
            TEST_SUPABASE_ISS,
            TEST_SUPABASE_AUD,
            TEST_SUPABASE_HMAC,
        );
        let app = AppContext::compose(
            AppRuntimeConfig {
                environment: crate::config::Environment::Dev,
                storage_mode: crate::config::StorageMode::Sqlite,
                runtime_mode: crate::config::RuntimeMode::Monolith,
                cache_backend: crate::realtime::CacheBackendKind::InMemory,
                message_bus_backend: crate::realtime::MessageBusBackendKind::Inline,
                db_max_connections: 1,
                token_ttl_secs: 300,
                cluster_channel: crate::config::DEFAULT_CLUSTER_CHANNEL.to_string(),
            },
            registry,
            host_link,
            pool.into(),
            Duration::from_mins(5),
            TEST_JWT_SECRET.to_string(),
            "test-instance-supabase".to_string(),
            MessageBusBackend::inline(),
            PeerTargetCacheBackend::in_memory(Duration::from_secs(5)),
            Arc::new(RealtimeTicketStore::default()),
            Some(verifier),
            None,
        );
        BackendState::from_app_context(app, None, "test")
    }
}

#[cfg(test)]
mod tests {
    use super::{formal_route_inventory, router};
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;
    use tower::util::ServiceExt;

    use crate::config::{Environment, RuntimeMode, StorageMode, DEFAULT_CLUSTER_CHANNEL};
    use crate::host_link::HostLinkService;
    use crate::realtime::{
        CacheBackendKind, MessageBusBackend, MessageBusBackendKind, PeerTargetCacheBackend,
    };
    use crate::runtime::{AppContext, AppRuntimeConfig};
    use crate::session::SessionRegistry;

    #[tokio::test]
    async fn route_inventory_matches_router() {
        let state = super::test_support::backend_state().await;
        let app = router(state);

        for route in formal_route_inventory() {
            let method = Method::from_bytes(route.method.as_bytes())
                .unwrap_or_else(|_| panic!("invalid method in route inventory: {}", route.method));
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(route.probe_path)
                        .body(Body::empty())
                        .expect("request builder"),
                )
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "router call failed for {} {}: {error}",
                        route.method, route.path
                    )
                });

            assert_ne!(
                response.status(),
                StatusCode::NOT_FOUND,
                "route inventory drifted: {} {} is not mounted",
                route.method,
                route.path,
            );
            assert_ne!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "route inventory drifted: {} {} resolved to a different method",
                route.method,
                route.path,
            );
        }
    }

    #[tokio::test]
    async fn external_sql_router_mounts_v1_surface_without_top_level_fallback() {
        let opts = PgConnectOptions::from_str("postgres://minos:secret@127.0.0.1:1/minos").unwrap();
        let pool = PgPoolOptions::new().connect_lazy_with(opts);
        let app = AppContext::compose(
            AppRuntimeConfig {
                environment: Environment::Dev,
                storage_mode: StorageMode::ExternalSql,
                runtime_mode: RuntimeMode::Monolith,
                cache_backend: CacheBackendKind::InMemory,
                message_bus_backend: MessageBusBackendKind::Inline,
                db_max_connections: 1,
                token_ttl_secs: 300,
                cluster_channel: DEFAULT_CLUSTER_CHANNEL.to_string(),
            },
            Arc::new(SessionRegistry::new()),
            Arc::new(HostLinkService::new(pool.clone())),
            pool.into(),
            Duration::from_mins(5),
            "test-jwt-secret-32-bytes-padding".to_string(),
            "external-sql-test-instance".to_string(),
            MessageBusBackend::inline(),
            PeerTargetCacheBackend::in_memory(Duration::from_secs(5)),
            Arc::new(crate::auth::realtime_ticket::RealtimeTicketStore::default()),
            None,
            None,
        );
        let app = router(super::BackendState::from_app_context(app, None, "test"));

        let live = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health/live")
                    .body(Body::empty())
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_eq!(live.status(), StatusCode::OK);

        let auth_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/auth/supabase")
                    .header("content-type", "application/json")
                    .header("x-device-id", "00000000-0000-0000-0000-0000000000aa")
                    .body(Body::from(r#"{"access_token":"not-a-jwt"}"#))
                    .expect("request builder"),
            )
            .await
            .unwrap();
        // No Supabase verifier in this fixture → not configured.
        assert_eq!(auth_route.status(), StatusCode::SERVICE_UNAVAILABLE);

        let host_nonce_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/host/bootstrap/nonce")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"installation_id":"00000000-0000-0000-0000-000000000001"}"#,
                    ))
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_eq!(host_nonce_route.status(), StatusCode::OK);

        let hosts_link_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hosts/link")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"installation_id":"00000000-0000-0000-0000-000000000001","nonce":"nonce_x","signature":"ed25519-sig:x"}"#,
                    ))
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_eq!(hosts_link_route.status(), StatusCode::UNAUTHORIZED);

        let realtime_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/realtime/ws-ticket")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"installation_id":"00000000-0000-0000-0000-000000000001"}"#,
                    ))
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_eq!(realtime_route.status(), StatusCode::UNAUTHORIZED);

        let host_realtime_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/host/realtime/ws-ticket")
                    .body(Body::empty())
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_eq!(host_realtime_route.status(), StatusCode::UNAUTHORIZED);

        let client_ws_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/ws/client")
                    .header("connection", "upgrade")
                    .header("upgrade", "websocket")
                    .header("sec-websocket-version", "13")
                    .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                    .body(Body::empty())
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_ne!(client_ws_route.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(client_ws_route.status(), StatusCode::NOT_FOUND);
        assert_ne!(client_ws_route.status(), StatusCode::METHOD_NOT_ALLOWED);

        let host_ws_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/ws/host")
                    .header("connection", "upgrade")
                    .header("upgrade", "websocket")
                    .header("sec-websocket-version", "13")
                    .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                    .body(Body::empty())
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_ne!(host_ws_route.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(host_ws_route.status(), StatusCode::NOT_FOUND);
        assert_ne!(host_ws_route.status(), StatusCode::METHOD_NOT_ALLOWED);

        let approvals_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/approvals/respond")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_ne!(approvals_route.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(approvals_route.status(), StatusCode::NOT_FOUND);
        assert_ne!(approvals_route.status(), StatusCode::METHOD_NOT_ALLOWED);

        let social_route = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/conversations/query")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_eq!(social_route.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn external_sql_ready_returns_503_when_postgres_is_unreachable() {
        let opts = PgConnectOptions::from_str("postgres://minos:secret@127.0.0.1:1/minos").unwrap();
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy_with(opts);
        let app = AppContext::compose(
            AppRuntimeConfig {
                environment: Environment::Dev,
                storage_mode: StorageMode::ExternalSql,
                runtime_mode: RuntimeMode::Monolith,
                cache_backend: CacheBackendKind::InMemory,
                message_bus_backend: MessageBusBackendKind::Inline,
                db_max_connections: 1,
                token_ttl_secs: 300,
                cluster_channel: DEFAULT_CLUSTER_CHANNEL.to_string(),
            },
            Arc::new(SessionRegistry::new()),
            Arc::new(HostLinkService::new(pool.clone())),
            pool.into(),
            Duration::from_mins(5),
            "test-jwt-secret-32-bytes-padding".to_string(),
            "external-sql-test-instance".to_string(),
            MessageBusBackend::inline(),
            PeerTargetCacheBackend::in_memory(Duration::from_secs(5)),
            Arc::new(crate::auth::realtime_ticket::RealtimeTicketStore::default()),
            None,
            None,
        );
        let app = router(super::BackendState::from_app_context(app, None, "test"));

        let ready = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health/ready")
                    .body(Body::empty())
                    .expect("request builder"),
            )
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
