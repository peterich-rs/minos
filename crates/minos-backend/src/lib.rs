//! Minos backend server library surface.
//!
//! Module layout follows hexagonal "Infrastructure" concerns:
//! - `config` — CLI + env parsing for the binary (step 10).
//! - `error` — backend-local error type (mapped to `MinosError` at API boundary
//!   in step 10; see spec §10.1).
//! - `store` — SQLite pool + embedded migrations.
//! - `pairing` — broker-side pairing service (token issue/consume, forget).
//! - `session` — in-memory registry of live WebSocket sessions with bounded
//!   per-peer outboxes (step 7; consumed by the WS dispatcher in step 8).
//! - `envelope` — WebSocket envelope dispatcher + local-RPC handlers
//!   (step 8; consumed by the axum upgrade handler in step 9).
//! - `http` — axum router + `/health` + `/devices` WS upgrade handshake
//!   (step 9; consumed by `main.rs` in step 10).
//!
//! The binary entry point lives in `src/main.rs` and composes the above
//! modules into a running backend (step 10).

#![forbid(unsafe_code)]

pub mod config;
pub mod envelope;
pub mod error;
pub mod http;
pub mod pairing;
pub mod session;
pub mod store;
