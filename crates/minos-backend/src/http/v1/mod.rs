//! Versioned `/v1` HTTP routes.
//!
//! Resource layout:
//! - `POST   /v1/pairing/tokens`     — agent-host mints a pairing token (replaces WS RequestPairingQr)
//! - `POST   /v1/pairing/consume`    — ios-client redeems a pairing token (replaces WS Pair)
//! - `DELETE /v1/pairing`            — paired device tears down the pairing (replaces WS ForgetPeer)
//! - `GET    /v1/me/peer`            — authenticated host looks up its current mobile peer (used by macOS post-connect)
//! - `GET    /v1/threads`            — paired device lists threads (replaces WS ListThreads)
//! - `GET    /v1/threads/{thread_id}/events`   — read window of UI events (replaces WS ReadThread)
//! - `GET    /v1/threads/{thread_id}/last_seq` — host helper (replaces WS GetThreadLastSeq)
//!
//! All routes share the auth model defined in [`crate::http::auth`].

use axum::Router;

use super::BackendState;

pub mod auth;
pub mod me;
pub mod pairing;
pub mod social;
pub mod threads;

pub fn router() -> Router<BackendState> {
    Router::new()
        .merge(auth::router())
        .merge(me::router())
        .merge(pairing::router())
        .merge(social::router())
        .merge(threads::router())
}
