//! HTTP surface: axum `Router` + shared state + header extraction helpers.
//!
//! The backend exposes exactly two HTTP-level endpoints:
//!
//! - `GET /health` — plaintext liveness probe ([`health::get`]). Body carries
//!   the crate name and version so a deploy smoke can assert both.
//! - `GET /devices` — WebSocket upgrade for the envelope hub
//!   ([`ws_devices::upgrade`]). Headers authenticate the device; the
//!   post-upgrade loop lives in [`crate::envelope::run_session`].
//!
//! # State plumbing
//!
//! [`BackendState`] bundles the three runtime Arcs (`SessionRegistry`,
//! `PairingService`, `SqlitePool`) plus the backend version string. It is
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
//! "401 pre-upgrade" contract (see [`ws_devices`]) is easy to read at the
//! call site.

use std::{sync::Arc, time::Duration};

use axum::Router;
use sqlx::SqlitePool;

use crate::{
    auth::rate_limit::RateLimiter, ingest::translate::ThreadTranslators,
    pairing::PairingService, session::SessionRegistry,
};

pub mod auth;
pub mod health;
pub mod v1;
pub mod ws_devices;

/// Backend-public config snapshot shared by every WS session. Bundles the
/// pieces of [`crate::config::Config`] the `RequestPairingQr` handler
/// needs so the envelope dispatcher doesn't have to thread three separate
/// arguments through `run_session` / `dispatch_envelope`.
#[derive(Debug, Clone)]
pub struct BackendPublicConfig {
    pub public_url: String,
    pub cf_access_client_id: Option<String>,
    pub cf_access_client_secret: Option<String>,
}

/// Shared state for every HTTP handler.
///
/// Cheap to clone: the service types are `Arc`-wrapped, and [`SqlitePool`]
/// is itself an `Arc` internally.
#[derive(Clone)]
pub struct BackendState {
    /// In-memory map of live WS sessions.
    pub registry: Arc<SessionRegistry>,
    /// Pairing business logic (token issue / consume / forget).
    pub pairing: Arc<PairingService>,
    /// SQLite pool with migrations already applied.
    pub store: SqlitePool,
    /// Configured pairing-token TTL for live `request_pairing_token` RPCs.
    pub token_ttl: Duration,
    /// Per-thread translator-state cache for the live ingest path.
    pub translators: Arc<ThreadTranslators>,
    /// Public-facing config snapshot (public URL + CF Access tokens) used
    /// by `RequestPairingQr` to assemble the QR payload. `Arc` so
    /// `BackendState::clone` is still cheap.
    pub public_cfg: Arc<BackendPublicConfig>,
    /// HS256 secret used by the bearer-token rail (`crate::auth::jwt`).
    /// `Arc<String>` because every signed/verified bearer borrows the
    /// bytes — sharing one heap copy across the request lifecycle keeps
    /// `BackendState::clone` cheap.
    pub jwt_secret: Arc<String>,
    /// Per-email login bucket (10 / minute, spec §5.6).
    pub auth_login_per_email: Arc<RateLimiter>,
    /// Per-IP login bucket (5 / minute, spec §5.6).
    pub auth_login_per_ip: Arc<RateLimiter>,
    /// Per-IP register bucket (3 / hour, spec §5.6).
    pub auth_register_per_ip: Arc<RateLimiter>,
    /// Per-account refresh bucket (60 / hour, spec §5.6).
    pub auth_refresh_per_acc: Arc<RateLimiter>,
    /// Crate version string; exposed via `/health`.
    ///
    /// Stored here rather than read from `env!("CARGO_PKG_VERSION")` at the
    /// handler so tests can substitute a fixed value without reaching into
    /// proc-macros.
    pub version: &'static str,
}

impl BackendState {
    /// Construct a state bundle with the crate's `CARGO_PKG_VERSION`.
    ///
    /// Intended call site: `main.rs` in step 10. Tests that need a custom
    /// version string can build the struct literally.
    #[must_use]
    pub fn new(
        registry: Arc<SessionRegistry>,
        pairing: Arc<PairingService>,
        store: SqlitePool,
        token_ttl: Duration,
        jwt_secret: String,
    ) -> Self {
        Self {
            registry,
            pairing,
            store,
            token_ttl,
            translators: ThreadTranslators::new(),
            public_cfg: Arc::new(BackendPublicConfig {
                public_url: "ws://127.0.0.1:8787/devices".into(),
                cf_access_client_id: None,
                cf_access_client_secret: None,
            }),
            jwt_secret: Arc::new(jwt_secret),
            auth_login_per_email: default_login_per_email(),
            auth_login_per_ip: default_login_per_ip(),
            auth_register_per_ip: default_register_per_ip(),
            auth_refresh_per_acc: default_refresh_per_acc(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

/// 10 logins per minute, keyed by email. Spec §5.6.
#[must_use]
pub fn default_login_per_email() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(10, Duration::from_mins(1)))
}

/// 5 logins per minute, keyed by IP. Spec §5.6.
#[must_use]
pub fn default_login_per_ip() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(5, Duration::from_mins(1)))
}

/// 3 register calls per hour, keyed by IP. Spec §5.6.
#[must_use]
pub fn default_register_per_ip() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(3, Duration::from_hours(1)))
}

/// 60 refreshes per hour, keyed by account. Spec §5.6.
#[must_use]
pub fn default_refresh_per_acc() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(60, Duration::from_hours(1)))
}

/// Build the backend's top-level axum `Router`.
///
/// Two routes (see module docs). No middleware is attached here — the
/// edge (Cloudflare Access) handles auth, TLS, and rate limiting; logs
/// are wired via `tracing` rather than `tower-http::trace` in this MVP.
pub fn router(state: BackendState) -> Router {
    Router::new()
        .route("/health", axum::routing::get(health::get))
        .route("/devices", axum::routing::get(ws_devices::upgrade))
        .nest("/v1", v1::router())
        .with_state(state)
}

/// Test scaffolding factories shared by the crate's integration tests.
///
/// Exposed publicly when the `test-support` feature is enabled (and
/// always when compiling tests) so test files under `tests/` and
/// downstream crates' dev-deps can build a ready-to-serve
/// [`BackendState`] backed by an in-memory SQLite pool.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::{BackendPublicConfig, BackendState};
    use crate::pairing::PairingService;
    use crate::session::SessionRegistry;
    use crate::store::test_support::memory_pool;
    use std::sync::Arc;
    use std::time::Duration;

    /// Deterministic 32-byte JWT secret used by every test that needs to
    /// sign/verify a bearer token. Long enough to satisfy
    /// `Config::validate`; the literal is fine because tests never hit a
    /// real network.
    pub const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";

    /// Build a `BackendState` against a fresh in-memory pool, with a
    /// 5-minute pairing-token TTL, the deterministic test JWT secret, and
    /// a stub `BackendPublicConfig` whose `public_url` matches the dev
    /// default.
    pub async fn backend_state() -> BackendState {
        let pool = memory_pool().await;
        let registry = Arc::new(SessionRegistry::new());
        let pairing = Arc::new(PairingService::new(pool.clone()));
        BackendState {
            registry,
            pairing,
            store: pool,
            token_ttl: Duration::from_mins(5),
            translators: crate::ingest::translate::ThreadTranslators::new(),
            public_cfg: Arc::new(BackendPublicConfig {
                public_url: "ws://127.0.0.1:8787/devices".into(),
                cf_access_client_id: None,
                cf_access_client_secret: None,
            }),
            jwt_secret: Arc::new(TEST_JWT_SECRET.to_string()),
            auth_login_per_email: super::default_login_per_email(),
            auth_login_per_ip: super::default_login_per_ip(),
            auth_register_per_ip: super::default_register_per_ip(),
            auth_refresh_per_acc: super::default_refresh_per_acc(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}
