//! SQLite connection pool, schema migrations, and typed CRUD helpers.
//!
//! Submodules:
//! - [`device_installations`] — client/host installation rows (`kind` enum).
//! - [`host_links`] — account ↔ host installation links (ADR-0020 / D02).
//! - [`agent_sessions`] — additive agent session metadata scoped to conversations.
//! - [`agent_turns`] — durable turn metadata for agent sessions.
//! - [`agent_turn_events`] — per-turn cold-replay stream slices.
//! - [`durable_event_log`] — replayable durable event history by topic.
//! - [`host_commands`] — durable host command queue + results.
//! - [`outbox_events`] — durable event dispatcher work queue.

use std::str::FromStr;
use std::time::Duration;

use serde::Serialize;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::PgPool;
use sqlx::SqlitePool;

use crate::error::BackendError;

const DEFAULT_MAX_CONNECTIONS: u32 = 32;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
static SQLITE_MIGRATOR: Migrator = sqlx::migrate!("./migrations/sqlite");
static POSTGRES_MIGRATOR: Migrator = sqlx::migrate!("./migrations/postgres");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StoreBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MigrationVariant {
    Sqlite,
    Postgres,
}

impl MigrationVariant {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone)]
pub enum StoreHandle {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

#[derive(Debug, Clone, Copy)]
pub enum StorePoolRef<'a> {
    Sqlite(&'a SqlitePool),
    Postgres(&'a PgPool),
}

pub trait AsStorePool {
    fn as_store_pool(&self) -> StorePoolRef<'_>;
}

impl StoreHandle {
    #[must_use]
    pub const fn backend(&self) -> StoreBackend {
        match self {
            Self::Sqlite(_) => StoreBackend::Sqlite,
            Self::Postgres(_) => StoreBackend::Postgres,
        }
    }

    #[must_use]
    pub const fn migration_variant(&self) -> MigrationVariant {
        match self {
            Self::Sqlite(_) => MigrationVariant::Sqlite,
            Self::Postgres(_) => MigrationVariant::Postgres,
        }
    }

    #[must_use]
    pub const fn is_sqlite(&self) -> bool {
        matches!(self, Self::Sqlite(_))
    }

    #[must_use]
    pub fn sqlite_pool(&self) -> Option<&SqlitePool> {
        match self {
            Self::Sqlite(pool) => Some(pool),
            Self::Postgres(_) => None,
        }
    }

    #[must_use]
    pub fn sqlite_pool_cloned(&self) -> Option<SqlitePool> {
        self.sqlite_pool().cloned()
    }

    #[must_use]
    pub fn postgres_pool(&self) -> Option<&PgPool> {
        match self {
            Self::Sqlite(_) => None,
            Self::Postgres(pool) => Some(pool),
        }
    }

    pub async fn ping(&self) -> Result<(), BackendError> {
        match self {
            Self::Sqlite(pool) => {
                let connection =
                    pool.acquire()
                        .await
                        .map_err(|error| BackendError::StoreQuery {
                            operation: "store::ping_sqlite".to_string(),
                            message: error.to_string(),
                        })?;
                drop(connection);
            }
            Self::Postgres(pool) => {
                let connection =
                    pool.acquire()
                        .await
                        .map_err(|error| BackendError::StoreQuery {
                            operation: "store::ping_postgres".to_string(),
                            message: error.to_string(),
                        })?;
                drop(connection);
            }
        }
        Ok(())
    }

    pub async fn close(&self) {
        match self {
            Self::Sqlite(pool) => pool.close().await,
            Self::Postgres(pool) => pool.close().await,
        }
    }
}

impl AsStorePool for StoreHandle {
    fn as_store_pool(&self) -> StorePoolRef<'_> {
        match self {
            Self::Sqlite(pool) => StorePoolRef::Sqlite(pool),
            Self::Postgres(pool) => StorePoolRef::Postgres(pool),
        }
    }
}

impl AsStorePool for SqlitePool {
    fn as_store_pool(&self) -> StorePoolRef<'_> {
        StorePoolRef::Sqlite(self)
    }
}

impl AsStorePool for PgPool {
    fn as_store_pool(&self) -> StorePoolRef<'_> {
        StorePoolRef::Postgres(self)
    }
}

impl From<SqlitePool> for StoreHandle {
    fn from(value: SqlitePool) -> Self {
        Self::Sqlite(value)
    }
}

impl From<PgPool> for StoreHandle {
    fn from(value: PgPool) -> Self {
        Self::Postgres(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ExternalSqlDriver {
    Postgres,
}

impl ExternalSqlDriver {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalSqlPreflight {
    pub driver: &'static str,
    pub database: String,
    pub server_version: String,
    pub max_connections: u32,
}

pub mod accounts;
pub mod agent_dispatch_queue;
pub mod agent_sessions;
pub mod agent_turn_events;
pub mod agent_turns;
pub mod approval_requests;
pub mod device_installations;
pub mod durable_event_log;
pub mod host_commands;
pub mod host_installation_tokens;
pub mod host_links;
pub mod media_blobs;
pub mod message_attachments;
pub mod notification_cooldowns;
pub mod notification_preferences;
pub mod outbox_events;
pub mod projects;
pub mod completion_watches;
pub mod push_dispatch_log;
pub mod push_dispatch_queue;
pub mod push_tokens;
pub mod raw_events;
pub mod refresh_tokens;
pub mod sessions;
pub mod social;
pub mod thread_sync_state;

pub use device_installations::{get_device, DeviceRow};

#[must_use]
pub const fn sqlite_backend_enabled() -> bool {
    cfg!(feature = "backend-sqlite")
}

#[must_use]
pub const fn postgres_backend_enabled() -> bool {
    cfg!(feature = "backend-postgres")
}

#[must_use]
pub fn compiled_storage_modes() -> Vec<String> {
    let mut modes = Vec::new();
    if sqlite_backend_enabled() {
        modes.push("sqlite".to_string());
    }
    if postgres_backend_enabled() {
        modes.push("external-sql".to_string());
    }
    modes
}

#[must_use]
pub fn detect_external_sql_driver(db_url: &str) -> Option<ExternalSqlDriver> {
    if !postgres_backend_enabled() {
        return None;
    }
    let lower = db_url.trim().to_ascii_lowercase();
    if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
        Some(ExternalSqlDriver::Postgres)
    } else {
        None
    }
}

#[must_use]
pub fn supported_external_sql_drivers() -> Vec<String> {
    if postgres_backend_enabled() {
        vec![ExternalSqlDriver::Postgres.as_str().to_string()]
    } else {
        Vec::new()
    }
}

/// Open the SQLite pool at `db_url` and run all embedded migrations.
///
/// `db_url` is a sqlx connection string, e.g. `sqlite://./minos-backend.db`
/// or `sqlite::memory:` for tests. Missing files are created on connect
/// via `SqliteConnectOptions::create_if_missing(true)`.
pub async fn connect(db_url: &str) -> Result<SqlitePool, BackendError> {
    connect_sqlite_with_options(db_url, DEFAULT_MAX_CONNECTIONS).await
}

/// Open the SQLite pool with explicit pool sizing and production-tuned pragmas.
pub async fn connect_sqlite_with_options(
    db_url: &str,
    max_connections: u32,
) -> Result<SqlitePool, BackendError> {
    if !sqlite_backend_enabled() {
        return Err(BackendError::StoreConnect {
            url: db_url.to_string(),
            message: "sqlite backend support is not compiled into this minos-backend binary"
                .to_string(),
        });
    }

    let opts = db_url
        .parse::<SqliteConnectOptions>()
        .map_err(|e| BackendError::StoreConnect {
            url: db_url.to_string(),
            message: e.to_string(),
        })?
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .pragma("temp_store", "MEMORY");

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(opts)
        .await
        .map_err(|e| BackendError::StoreConnect {
            url: db_url.to_string(),
            message: e.to_string(),
        })?;

    SQLITE_MIGRATOR
        .run(&pool)
        .await
        .map_err(|e| BackendError::StoreMigrate {
            message: e.to_string(),
        })?;

    Ok(pool)
}

pub async fn connect_external_sql_with_options(
    db_url: &str,
    max_connections: u32,
) -> Result<(StoreHandle, ExternalSqlPreflight), BackendError> {
    if !postgres_backend_enabled() {
        return Err(BackendError::StoreConnect {
            url: db_url.to_string(),
            message: "postgres external-sql support is not compiled into this minos-backend binary"
                .to_string(),
        });
    }

    match detect_external_sql_driver(db_url) {
        Some(ExternalSqlDriver::Postgres) => {
            let pool = connect_postgres_with_options(db_url, max_connections).await?;
            let preflight = describe_postgres_pool(&pool, max_connections).await?;
            Ok((StoreHandle::from(pool), preflight))
        }
        None => Err(BackendError::StoreConnect {
            url: db_url.to_string(),
            message:
                "unsupported external SQL driver; only postgres:// and postgresql:// URLs are supported"
                    .to_string(),
        }),
    }
}

/// Perform a real external-SQL preflight against the supported production
/// adapter set.
///
/// Today that means Postgres-class URLs only. The function intentionally stops
/// at connectivity + boot diagnostics because the request-serving runtime still
/// holds a concrete `SqlitePool` and many store modules use SQLite-specific SQL.
pub async fn preflight_external_sql_with_options(
    db_url: &str,
    max_connections: u32,
) -> Result<ExternalSqlPreflight, BackendError> {
    let (pool, preflight) = connect_external_sql_with_options(db_url, max_connections).await?;
    pool.close().await;
    Ok(preflight)
}

pub async fn connect_postgres_with_options(
    db_url: &str,
    max_connections: u32,
) -> Result<PgPool, BackendError> {
    let opts = PgConnectOptions::from_str(db_url).map_err(|error| BackendError::StoreConnect {
        url: db_url.to_string(),
        message: error.to_string(),
    })?;

    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_with(opts)
        .await
        .map_err(|error| BackendError::StoreConnect {
            url: db_url.to_string(),
            message: error.to_string(),
        })?;

    POSTGRES_MIGRATOR
        .run(&pool)
        .await
        .map_err(|error| BackendError::StoreMigrate {
            message: error.to_string(),
        })?;

    Ok(pool)
}

async fn describe_postgres_pool(
    pool: &PgPool,
    max_connections: u32,
) -> Result<ExternalSqlPreflight, BackendError> {
    let (database, server_version) =
        sqlx::query_as::<_, (String, String)>("SELECT current_database(), version()")
            .fetch_one(pool)
            .await
            .map_err(|error| BackendError::StoreQuery {
                operation: "external_sql_preflight".to_string(),
                message: error.to_string(),
            })?;

    Ok(ExternalSqlPreflight {
        driver: ExternalSqlDriver::Postgres.as_str(),
        database,
        server_version,
        max_connections,
    })
}

/// Shared test helpers used by the store submodule tests AND by
/// `crate::pairing`'s integration tests. Extracted to collapse ~35 lines of
/// duplication that accrued across `devices::tests`, `pairings::tests`, and
/// `tokens::tests` during step 5.
///
/// Exposed publicly when the `test-support` feature is enabled so
/// integration tests in sibling crates (and this crate's own integration
/// test files under `tests/`) can build an in-memory pool without
/// duplicating the boilerplate.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
        SQLITE_BUSY_TIMEOUT, SQLITE_MIGRATOR,
    };
    use minos_domain::{DeviceId, DeviceRole};

    /// Fixed unix-epoch ms used as `now` in tests.
    pub const T0: i64 = 1_700_000_000_000;

    /// Open a fresh in-memory SQLite pool with migrations applied.
    ///
    /// `sqlite::memory:` is per-connection — each connection gets its own DB.
    /// The pool is capped at 1 so all queries see a consistent store.
    pub async fn memory_pool() -> SqlitePool {
        let opts: SqliteConnectOptions = "sqlite::memory:".parse().unwrap();
        let opts = opts
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(SQLITE_BUSY_TIMEOUT)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .pragma("temp_store", "MEMORY");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        SQLITE_MIGRATOR.run(&pool).await.unwrap();
        pool
    }

    /// Insert an account row via `store::accounts::create` and return the
    /// generated `account_id`.
    pub async fn insert_account(pool: &SqlitePool, email: &str) -> String {
        crate::store::accounts::create(pool, email)
            .await
            .unwrap()
            .account_id
    }

    /// Insert a mobile installation linked to `account_id` and return its
    /// `DeviceId`. Client auth is bearer-only (no device secret).
    pub async fn insert_ios_device(pool: &SqlitePool, account_id: &str) -> DeviceId {
        let id = DeviceId::new();
        crate::store::device_installations::insert_client_for_account(
            pool,
            id,
            "iPhone",
            DeviceRole::MobileClient,
            account_id,
            T0,
        )
        .await
        .unwrap();
        id
    }

    /// Fixed host public key for fixtures.
    pub const TEST_HOST_PUBLIC_KEY: &str = "test-host-public-key-v1";

    /// Strict host fixture (public_key required).
    pub async fn insert_test_host(
        pool: &impl crate::store::AsStorePool,
        id: DeviceId,
        name: &str,
        now: i64,
    ) {
        crate::store::device_installations::insert_host_with_public_key(
            pool,
            id,
            name,
            TEST_HOST_PUBLIC_KEY,
            now,
        )
        .await
        .unwrap();
    }

    /// Strict host fixture with a generated id.
    pub async fn insert_test_host_new(
        pool: &impl crate::store::AsStorePool,
        name: &str,
        now: i64,
    ) -> DeviceId {
        let id = DeviceId::new();
        insert_test_host(pool, id, name, now).await;
        id
    }

    /// Strict client fixture (account_id required).
    pub async fn insert_test_client(
        pool: &impl crate::store::AsStorePool,
        id: DeviceId,
        role: DeviceRole,
        account_id: &str,
        name: &str,
        now: i64,
    ) {
        crate::store::device_installations::insert_client_for_account(
            pool, id, name, role, account_id, now,
        )
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        detect_external_sql_driver, test_support::memory_pool, ExternalSqlDriver, StoreBackend,
        StoreHandle,
    };
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::str::FromStr;

    #[test]
    fn detect_external_sql_driver_accepts_postgres_urls() {
        assert_eq!(
            detect_external_sql_driver("postgres://minos:secret@localhost/minos"),
            Some(ExternalSqlDriver::Postgres)
        );
        assert_eq!(
            detect_external_sql_driver("POSTGRESQL://minos:secret@localhost/minos"),
            Some(ExternalSqlDriver::Postgres)
        );
    }

    #[test]
    fn detect_external_sql_driver_rejects_non_postgres_urls() {
        assert_eq!(
            detect_external_sql_driver("sqlite://./minos-backend.db?mode=rwc"),
            None
        );
        assert_eq!(
            detect_external_sql_driver("mysql://minos:secret@localhost/minos"),
            None
        );
    }

    #[tokio::test]
    async fn store_handle_sqlite_ping_succeeds() {
        let pool = memory_pool().await;
        let handle = StoreHandle::from(pool);

        assert_eq!(handle.backend(), StoreBackend::Sqlite);
        assert!(handle.sqlite_pool().is_some());
        assert!(handle.postgres_pool().is_none());
        handle.ping().await.unwrap();
    }

    #[tokio::test]
    async fn store_handle_distinguishes_lazy_postgres_pool() {
        let opts = PgConnectOptions::from_str("postgres://minos:secret@127.0.0.1:1/minos").unwrap();
        let pool = PgPoolOptions::new().connect_lazy_with(opts);
        let handle = StoreHandle::from(pool);

        assert_eq!(handle.backend(), StoreBackend::Postgres);
        assert!(handle.sqlite_pool().is_none());
        assert!(handle.postgres_pool().is_some());
    }
}
