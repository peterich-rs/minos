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
