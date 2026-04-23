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

    /// An argon2 hash / verify operation failed.
    ///
    /// Raised by `pairing::secret::{hash_secret, verify_secret}` for malformed
    /// PHC strings or internal argon2 errors. Named for easy future
    /// `From<RelayError> for MinosError` mapping (mirrors
    /// `MinosError::RelayInternal`).
    #[error("pairing hash failed: {message}")]
    PairingHash { message: String },

    /// A pairing token was unknown, expired, or already consumed.
    ///
    /// The three cases are intentionally collapsed: distinguishing them at
    /// the API surface would leak token-existence information to an
    /// attacker who can probe. Mirrors `MinosError::PairingTokenInvalid`.
    #[error("pairing token invalid or expired")]
    PairingTokenInvalid,

    /// Pairing refused because one side was already paired.
    ///
    /// Spec §10.2 R4: MVP policy is "refuse and let the UI confirm replace
    /// via explicit `forget_peer` + retry". `actual` captures the observed
    /// state (currently always `"paired"`) so future callers can rely on
    /// the stringly-typed shape without caring about the domain enum.
    /// Mirrors `MinosError::PairingStateMismatch`.
    #[error("pairing state mismatch: {actual}")]
    PairingStateMismatch { actual: String },
}
