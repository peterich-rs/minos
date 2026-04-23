//! HTTP surface: axum `Router` + shared state + header extraction helpers.
//!
//! The relay exposes exactly two HTTP-level endpoints:
//!
//! - `GET /health` — plaintext liveness probe ([`health::get`]). Body carries
//!   the crate name and version so a deploy smoke can assert both.
//! - `GET /devices` — WebSocket upgrade for the envelope hub
//!   ([`ws_devices::upgrade`]). Headers authenticate the device; the
//!   post-upgrade loop lives in [`crate::envelope::run_session`].
//!
//! # State plumbing
//!
//! [`RelayState`] bundles the three runtime Arcs (`SessionRegistry`,
//! `PairingService`, `SqlitePool`) plus the relay version string. It is
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

use std::sync::Arc;

use axum::Router;
use sqlx::SqlitePool;

use crate::{pairing::PairingService, session::SessionRegistry};

pub mod health;
pub mod ws_devices;

/// Shared state for every HTTP handler.
///
/// Cheap to clone: the two service types are `Arc`-wrapped, and
/// [`SqlitePool`] is itself an `Arc` internally.
#[derive(Clone)]
pub struct RelayState {
    /// In-memory map of live WS sessions.
    pub registry: Arc<SessionRegistry>,
    /// Pairing business logic (token issue / consume / forget).
    pub pairing: Arc<PairingService>,
    /// SQLite pool with migrations already applied.
    pub store: SqlitePool,
    /// Crate version string; exposed via `/health`.
    ///
    /// Stored here rather than read from `env!("CARGO_PKG_VERSION")` at the
    /// handler so tests can substitute a fixed value without reaching into
    /// proc-macros.
    pub version: &'static str,
}

impl RelayState {
    /// Construct a state bundle with the crate's `CARGO_PKG_VERSION`.
    ///
    /// Intended call site: `main.rs` in step 10. Tests that need a custom
    /// version string can build the struct literally.
    #[must_use]
    pub fn new(
        registry: Arc<SessionRegistry>,
        pairing: Arc<PairingService>,
        store: SqlitePool,
    ) -> Self {
        Self {
            registry,
            pairing,
            store,
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

/// Build the relay's top-level axum `Router`.
///
/// Two routes (see module docs). No middleware is attached here — the
/// edge (Cloudflare Access) handles auth, TLS, and rate limiting; logs
/// are wired via `tracing` rather than `tower-http::trace` in this MVP.
pub fn router(state: RelayState) -> Router {
    Router::new()
        .route("/health", axum::routing::get(health::get))
        .route("/devices", axum::routing::get(ws_devices::upgrade))
        .with_state(state)
}
