//! SQLite connection pool, schema migrations, and typed CRUD helpers.
//!
//! Submodules:
//! - [`devices`] — device rows + per-device secret hashes.
//! - [`pairings`] — undirected device ↔ device pairings (canonical `a < b`).
//! - [`tokens`] — one-shot pairing tokens with atomic consume + GC.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::error::RelayError;

pub mod devices;
pub mod pairings;
pub mod tokens;

pub use devices::{get_device, get_secret_hash, insert_device, upsert_secret_hash, DeviceRow};
pub use pairings::{delete_pair, get_pair, insert_pairing};
pub use tokens::{consume_token, gc_expired, issue_token, ConsumedToken};

/// Open the SQLite pool at `db_url` and run all embedded migrations.
///
/// `db_url` is a sqlx connection string, e.g. `sqlite://./minos-relay.db`
/// or `sqlite::memory:` for tests. Missing files are created on connect
/// via `SqliteConnectOptions::create_if_missing(true)`.
pub async fn connect(db_url: &str) -> Result<SqlitePool, RelayError> {
    let opts = db_url
        .parse::<SqliteConnectOptions>()
        .map_err(|e| RelayError::StoreConnect {
            url: db_url.to_string(),
            message: e.to_string(),
        })?
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(|e| RelayError::StoreConnect {
            url: db_url.to_string(),
            message: e.to_string(),
        })?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| RelayError::StoreMigrate {
            message: e.to_string(),
        })?;

    Ok(pool)
}
