//! Backend-internal error type.
//!
//! Kept crate-local for now; a `From<BackendError> for minos_domain::MinosError`
//! conversion lands when `main.rs` wires the HTTP/WebSocket surface. Store
//! errors still collapse to the existing generic internal-error fallback in
//! `minos_domain::MinosError`, but the concrete mapping table is deferred
//! until the outer boundary actually needs it.
//!
//! The enum mirrors the `#[derive(thiserror::Error, Debug)]`
//! + `#[error("...")]` style used in `minos-domain::MinosError`.

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("store connect failed at {url}: {message}")]
    StoreConnect { url: String, message: String },

    #[error("store migrate failed: {message}")]
    StoreMigrate { message: String },

    /// A store operation targeted a device/installation that does not exist.
    ///
    /// Emitted by touch/update helpers when no row matches the given id.
    #[error("device not found: {device_id}")]
    DeviceNotFound { device_id: String },

    /// A row returned by the store failed to parse back into a domain type.
    ///
    /// The store writes ids/kinds as TEXT and parses on read
    /// (see `store/device_installations.rs`). Corrupt rows — or schema drift
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
    /// `From<BackendError> for MinosError` mapping.
    #[error("pairing hash failed: {message}")]
    PairingHash { message: String },

    /// A pairing token was unknown, expired, or already consumed.
    ///
    /// The three cases are intentionally collapsed: distinguishing them at
    /// the API surface would leak token-existence information to an
    /// attacker who can probe. Mirrors `MinosError::PairingTokenInvalid`.
    #[error("pairing token invalid or expired")]
    PairingTokenInvalid,

    /// An account create attempt collided with an existing email row.
    ///
    /// The `accounts` table has `UNIQUE COLLATE NOCASE` on email so the
    /// check is enforced at insert time. Mirrors
    /// `MinosError::EmailTaken` for the boundary mapping.
    #[error("email already registered")]
    EmailTaken,

    /// Host installation is already linked to a different account.
    ///
    /// Enforced by `UNIQUE (host_installation_id)` on `host_links` and by
    /// in-transaction exclusivity checks (Host Link + QR confirm).
    #[error("host already linked to another account")]
    HostLinkedElsewhere { host_installation_id: String },

    /// HS256 JWT signing failed (e.g. malformed key).
    #[error("jwt sign error: {message}")]
    JwtSign { message: String },

    /// HS256 JWT decode/verify failed (bad signature, expired, malformed).
    #[error("jwt verify error: {message}")]
    JwtVerify { message: String },

    /// Pairing refused because the candidate state is invalid, for example
    /// a device trying to pair with itself. `actual` captures the observed
    /// state (currently `"self"` on the multi-device path).
    /// Mirrors `MinosError::PairingStateMismatch`.
    #[error("pairing state mismatch: {actual}")]
    PairingStateMismatch { actual: String },

    /// The routing target is not currently connected.
    ///
    /// Destination device is not currently connected (or its outbox closed).
    /// Mirrors `MinosError::PeerOffline`; the boundary maps this variant
    /// straight across.
    ///
    /// `peer_device_id` is stringly-typed because the error is also used
    /// in log records and API responses where the `DeviceId` newtype is
    /// inconvenient.
    #[error("peer offline: {peer_device_id}")]
    PeerOffline { peer_device_id: String },

    /// The routing target is connected but cannot currently accept more
    /// forwarded frames.
    ///
    /// Destination is connected but cannot currently accept more frames.
    /// This stays backend-local: callers that can recover in protocol space
    /// should surface a deterministic retryable error rather than hanging.
    #[error("peer backpressure: {peer_device_id}")]
    PeerBackpressure { peer_device_id: String },

    #[error("forwarded rpc `{method}` failed: {message}")]
    ForwardRpc { method: String, message: String },

    #[error("forwarded rpc `{method}` timed out after {timeout_ms}ms")]
    ForwardRpcTimeout { method: String, timeout_ms: u64 },

    #[error("cache `{operation}` failed: {message}")]
    Cache { operation: String, message: String },

    #[error("message bus `{operation}` failed: {message}")]
    MessageBus { operation: String, message: String },
}
