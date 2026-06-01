use std::sync::Arc;

use async_trait::async_trait;

use crate::app::repositories::RepositorySet;
use crate::store::StoreHandle;

// ---------------------------------------------------------------------------
// Clock trait — injectable monotonic-time source for testability
// ---------------------------------------------------------------------------

/// Wall-clock abstraction. Production code uses `SystemClock`; tests inject a
/// deterministic stub.
#[async_trait]
pub trait Clock: Send + Sync {
    /// Current unix-epoch milliseconds.
    fn now_ms(&self) -> i64;
}

/// Production clock backed by `std::time::SystemTime`.
pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(i64::MAX)
    }
}

// ---------------------------------------------------------------------------
// IdGenerator trait — injectable ID source for testability
// ---------------------------------------------------------------------------

/// ID generation abstraction. Production code uses `UuidGenerator`; tests
/// inject deterministic stubs.
#[async_trait]
pub trait IdGenerator: Send + Sync {
    /// Generate a new opaque string ID (e.g. UUID v4).
    fn next_id(&self) -> String;
}

/// Production ID generator using `uuid::Uuid::new_v4()`.
pub struct UuidGenerator;

#[async_trait]
impl IdGenerator for UuidGenerator {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

// ---------------------------------------------------------------------------
// AppRuntimeConfig
// ---------------------------------------------------------------------------

/// Minimal runtime configuration. Expanded as features land.
#[derive(Debug, Clone)]
pub struct AppRuntimeConfig {
    /// JWT signing secret (HS256).
    pub jwt_secret: Vec<u8>,
    /// Access token TTL in milliseconds.
    pub access_token_ttl_ms: i64,
    /// Refresh token TTL in milliseconds.
    pub refresh_token_ttl_ms: i64,
    /// Pairing code TTL in milliseconds.
    pub pairing_code_ttl_ms: i64,
    /// Max connections for the database pool.
    pub db_max_connections: u32,
}

impl Default for AppRuntimeConfig {
    fn default() -> Self {
        Self {
            jwt_secret: Vec::new(),
            access_token_ttl_ms: 15 * 60 * 1000,     // 15 minutes
            refresh_token_ttl_ms: 30 * 24 * 3600 * 1000, // 30 days
            pairing_code_ttl_ms: 10 * 60 * 1000,      // 10 minutes
            db_max_connections: 32,
        }
    }
}

// ---------------------------------------------------------------------------
// AppDataContext
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppDataContext {
    pub storage: StoreHandle,
    pub repos: Arc<RepositorySet>,
    pub config: Arc<AppRuntimeConfig>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGenerator>,
}

impl AppDataContext {
    #[must_use]
    pub fn new(storage: StoreHandle) -> Self {
        Self {
            repos: Arc::new(RepositorySet::from_store(storage.clone())),
            storage,
            config: Arc::new(AppRuntimeConfig::default()),
            clock: Arc::new(SystemClock),
            ids: Arc::new(UuidGenerator),
        }
    }

    /// Construct with explicit config, clock, and ID generator (for tests).
    #[must_use]
    pub fn new_with(
        storage: StoreHandle,
        config: Arc<AppRuntimeConfig>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
    ) -> Self {
        Self {
            repos: Arc::new(RepositorySet::from_store(storage.clone())),
            storage,
            config,
            clock,
            ids,
        }
    }
}
