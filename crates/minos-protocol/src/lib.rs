//! Minos JSON-RPC 2.0 contract.
//!
//! - `envelope`: relay WebSocket frame (`Envelope` + sub-enums)
//! - `messages`: typed request / response payloads
//! - `rpc`:      jsonrpsee `#[rpc]` trait shared by daemon (server) and mobile (client)

#![forbid(unsafe_code)]

pub mod envelope;
pub mod messages;
pub mod rpc;

pub use envelope::*;
pub use messages::*;
pub use rpc::*;

// UniFFI 0.31 per-crate scaffolding: every crate that carries `uniffi::*`
// derives must define `UniFfiTag` locally via `setup_scaffolding!()`; the
// derive expansions reference `crate::UniFfiTag`.
#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
