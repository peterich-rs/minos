//! Relay-internal error type.
//!
//! Kept crate-local for now; a `From<RelayError> for minos_domain::MinosError`
//! conversion will land in step 10 when `main.rs` wires the HTTP/WebSocket
//! surface. Per spec §10.1, the fallback mapping is `MinosError::RelayInternal`
//! for store errors, but the concrete mapping table is deferred until the
//! outer boundary actually needs it.
//!
//! Start minimal — steps 5–10 will add variants as the auth, REST, and hub
//! layers grow. The enum mirrors the `#[derive(thiserror::Error, Debug)]`
//! + `#[error("...")]` style used in `minos-domain::MinosError`.

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("store connect failed at {url}: {message}")]
    StoreConnect { url: String, message: String },

    #[error("store migrate failed: {message}")]
    StoreMigrate { message: String },

    /// A store operation targeted a device that does not exist.
    ///
    /// Emitted by `upsert_secret_hash` when no row matches the given
    /// `device_id`. Callers can distinguish this from generic store errors
    /// to render the user-facing "device not found" path.
    #[error("device not found: {device_id}")]
    DeviceNotFound { device_id: String },

    /// A row returned by the store failed to parse back into a domain type.
    ///
    /// The store writes `DeviceId` / `DeviceRole` as TEXT and parses on read
    /// (see `store/devices.rs` strategy note). Corrupt rows — or schema drift
    /// between migrations and domain types — surface here.
    #[error("store decode failed for column `{column}`: {message}")]
    StoreDecode { column: String, message: String },

    /// Fallback for sqlx errors at bind / execute / fetch time.
    ///
    /// `operation` is a short human-readable verb (e.g. `"insert_device"`)
    /// that callers can match on for coarse log grouping; `message` is the
    /// upstream sqlx error stringified.
    #[error("store query `{operation}` failed: {message}")]
    StoreQuery { operation: String, message: String },
}
