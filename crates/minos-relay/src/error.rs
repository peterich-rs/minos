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
}
