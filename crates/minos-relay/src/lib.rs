//! Minos relay server library surface.
//!
//! Module layout follows hexagonal "Infrastructure" concerns:
//! - `error`  relay-local error type (mapped to `MinosError` at API boundary
//!   in step 10; see spec §10.1)
//! - `store`  SQLite pool + embedded migrations
//!
//! The binary entry point lives in `src/main.rs`; steps 5–10 will flesh it out
//! as the relay gains auth, REST endpoints, and the WebSocket hub.

#![forbid(unsafe_code)]

pub mod error;
pub mod store;
