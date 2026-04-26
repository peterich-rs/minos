//! Backend-internal error type.
//!
//! Kept crate-local for now; a `From<BackendError> for minos_domain::MinosError`
//! conversion will land in step 10 when `main.rs` wires the HTTP/WebSocket
//! surface. Per spec §10.1, store errors still collapse to the existing
//! generic internal-error fallback in `minos_domain::MinosError`, but the
//! concrete mapping table is deferred until the outer boundary actually
//! needs it.
//!
//! Start minimal — steps 5–10 will add variants as the auth, REST, and hub
//! layers grow. The enum mirrors the `#[derive(thiserror::Error, Debug)]`
//! + `#[error("...")]` style used in `minos-domain::MinosError`.

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
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

    /// An argon2id password hash / verify operation failed.
    ///
    /// Distinct from `PairingHash` so the auth rail and the pairing rail
    /// can surface independent log/metric labels.
    #[error("password hash error: {message}")]
    PasswordHash { message: String },

    /// Pairing refused because one side was already paired.
    ///
    /// Spec §10.2 R4: MVP policy is "refuse and let the UI confirm replace
    /// via explicit `forget_peer` + retry". `actual` captures the observed
    /// state (currently always `"paired"`) so future callers can rely on
    /// the stringly-typed shape without caring about the domain enum.
    /// Mirrors `MinosError::PairingStateMismatch`.
    #[error("pairing state mismatch: {actual}")]
    PairingStateMismatch { actual: String },

    /// The routing target is not currently connected.
    ///
    /// Emitted by `session::SessionRegistry::route` when the destination
    /// `DeviceId` has no live `SessionHandle` in the registry, or when the
    /// destination's outbox receiver has been dropped (session ended mid-
    /// route). Mirrors `MinosError::PeerOffline`; the step-10 boundary maps
    /// this variant straight across.
    ///
    /// `peer_device_id` is stringly-typed because the error is also used
    /// in log records and API responses where the `DeviceId` newtype is
    /// inconvenient.
    #[error("peer offline: {peer_device_id}")]
    PeerOffline { peer_device_id: String },

    /// The routing target is connected but cannot currently accept more
    /// forwarded frames.
    ///
    /// Emitted by `session::SessionRegistry::route` when the destination
    /// outbox is full. This stays backend-local: callers that can recover in
    /// protocol space should surface a deterministic retryable error to the
    /// sender rather than hanging until timeout.
    #[error("peer backpressure: {peer_device_id}")]
    PeerBackpressure { peer_device_id: String },
}
