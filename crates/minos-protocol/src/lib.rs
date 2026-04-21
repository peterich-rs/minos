//! Minos JSON-RPC 2.0 contract.
//!
//! - `messages`: typed request / response payloads
//! - `events`:   AgentEvent enum reserved for the future `subscribe_events` stream
//! - `rpc`:      jsonrpsee `#[rpc]` trait shared by daemon (server) and mobile (client)
//!               (added in Task 12)

#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod events;
pub mod messages;

pub use events::*;
pub use messages::*;
