//! Minos JSON-RPC 2.0 contract.
//!
//! - `messages`: typed request / response payloads
//! - `events`:   `AgentEvent` enum reserved for the future `subscribe_events` stream
//! - `rpc`:      jsonrpsee `#[rpc]` trait shared by daemon (server) and mobile (client)

#![forbid(unsafe_code)]

pub mod events;
pub mod messages;
pub mod rpc;

pub use events::*;
pub use messages::*;
pub use rpc::*;

// UniFFI 0.31 per-crate scaffolding: every crate that carries `uniffi::*`
// derives must define `UniFfiTag` locally via `setup_scaffolding!()`; the
// derive expansions reference `crate::UniFfiTag`.
#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
