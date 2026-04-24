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

use std::{sync::Arc, time::Duration};

use axum::Router;
use sqlx::SqlitePool;

use crate::{
    ingest::translate::ThreadTranslators, pairing::PairingService, session::SessionRegistry,
};

pub mod health;
pub mod ws_devices;

/// Backend-public config snapshot shared by every WS session. Bundles the
/// pieces of [`crate::config::Config`] the `RequestPairingQr` handler
/// needs so the envelope dispatcher doesn't have to thread three separate
/// arguments through `run_session` / `dispatch_envelope`.
#[derive(Debug, Clone)]
pub struct BackendPublicConfig {
    pub public_url: String,
    pub cf_access_client_id: Option<String>,
    pub cf_access_client_secret: Option<String>,
}

/// Shared state for every HTTP handler.
///
/// Cheap to clone: the service types are `Arc`-wrapped, and [`SqlitePool`]
/// is itself an `Arc` internally.
#[derive(Clone)]
pub struct RelayState {
    /// In-memory map of live WS sessions.
    pub registry: Arc<SessionRegistry>,
    /// Pairing business logic (token issue / consume / forget).
    pub pairing: Arc<PairingService>,
    /// SQLite pool with migrations already applied.
    pub store: SqlitePool,
    /// Configured pairing-token TTL for live `request_pairing_token` RPCs.
    pub token_ttl: Duration,
    /// Per-thread translator-state cache for the live ingest path.
    pub translators: Arc<ThreadTranslators>,
    /// Public-facing config snapshot (public URL + CF Access tokens) used
    /// by `RequestPairingQr` to assemble the QR payload. `Arc` so
    /// `RelayState::clone` is still cheap.
    pub public_cfg: Arc<BackendPublicConfig>,
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
        token_ttl: Duration,
    ) -> Self {
        Self {
            registry,
            pairing,
            store,
            token_ttl,
            translators: ThreadTranslators::new(),
            public_cfg: Arc::new(BackendPublicConfig {
                public_url: "ws://127.0.0.1:8787/devices".into(),
                cf_access_client_id: None,
                cf_access_client_secret: None,
            }),
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
