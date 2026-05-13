//! Versioned `/v1` HTTP routes.
//!
//! Resource layout:
//! - `POST   /v1/pairing/tokens`     — agent-host mints a pairing token (replaces WS RequestPairingQr)
//! - `POST   /v1/pairing/consume`    — ios-client redeems a pairing token (replaces WS Pair)
//! - `DELETE /v1/pairing`            — paired device tears down the pairing (replaces WS ForgetPeer)
//! - `POST   /v1/me/peer/query`      — authenticated host looks up its current mobile peer
//! - `POST   /v1/threads/query`      — paired device lists threads (replaces WS ListThreads)
//! - `POST   /v1/threads/read`       — read window of UI events (replaces WS ReadThread)
//! - `POST   /v1/threads/last-seq`   — host helper (replaces WS GetThreadLastSeq)
//!
//! Read routes are POST-first; query-string GET endpoints are no longer
//! exposed on `/v1`. All routes share the auth model defined in
//! [`crate::http::auth`].

use axum::Router;

use super::BackendState;

pub mod auth;
pub mod me;
pub mod pairing;
pub mod projects;
pub mod social;
pub mod threads;

pub fn router() -> Router<BackendState> {
    Router::new()
        .merge(auth::router())
        .merge(me::router())
        .merge(pairing::router())
        .merge(projects::router())
        .merge(social::router())
        .merge(threads::router())
}
