//! Minos JSON-RPC 2.0 contract.
//!
//! - `envelope`: relay WebSocket frame (`Envelope` + sub-enums)
//! - `events`:   `AgentEvent` enum reserved for the future `subscribe_events` stream
//! - `messages`: typed request / response payloads
//! - `rpc`:      jsonrpsee `#[rpc]` trait shared by daemon (server) and mobile (client)

#![forbid(unsafe_code)]

pub mod envelope;
pub mod events;
pub mod messages;
pub mod rpc;

pub use envelope::*;
pub use events::*;
pub use messages::*;
pub use rpc::*;
