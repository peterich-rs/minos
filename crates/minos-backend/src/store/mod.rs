//! SQLite connection pool, schema migrations, and typed CRUD helpers.
//!
//! Submodules:
//! - [`devices`] — device rows + per-device secret hashes.
//! - [`pairings`] — undirected device ↔ device pairings (canonical `a < b`).
//! - [`tokens`] — one-shot pairing tokens with atomic consume + GC.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::error::BackendError;

pub mod devices;
pub mod pairings;
pub mod raw_events;
pub mod threads;
pub mod tokens;

pub use devices::{get_device, get_secret_hash, insert_device, upsert_secret_hash, DeviceRow};
pub use pairings::{delete_pair, get_pair, insert_pairing};
pub use tokens::{consume_token, gc_expired, issue_token, ConsumedToken};

/// Open the SQLite pool at `db_url` and run all embedded migrations.
///
/// `db_url` is a sqlx connection string, e.g. `sqlite://./minos-backend.db`
/// or `sqlite::memory:` for tests. Missing files are created on connect
/// via `SqliteConnectOptions::create_if_missing(true)`.
pub async fn connect(db_url: &str) -> Result<SqlitePool, BackendError> {
    let opts = db_url
        .parse::<SqliteConnectOptions>()
        .map_err(|e| BackendError::StoreConnect {
            url: db_url.to_string(),
            message: e.to_string(),
        })?
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(|e| BackendError::StoreConnect {
            url: db_url.to_string(),
            message: e.to_string(),
        })?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| BackendError::StoreMigrate {
            message: e.to_string(),
        })?;

    Ok(pool)
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
    use super::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

    /// Fixed unix-epoch ms used as `now` in tests.
    pub const T0: i64 = 1_700_000_000_000;

    /// Open a fresh in-memory SQLite pool with migrations applied.
    ///
    /// `sqlite::memory:` is per-connection — each connection gets its own DB.
    /// The pool is capped at 1 so all queries see a consistent store.
    pub async fn memory_pool() -> SqlitePool {
        let opts: SqliteConnectOptions = "sqlite::memory:".parse().unwrap();
        let opts = opts.create_if_missing(true).foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }
}
