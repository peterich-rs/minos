//! Minos JSON-RPC 2.0 contract.
//!
//! - `auth`:     HTTP DTOs for `/v1/auth/*` (supabase / refresh / logout)
//! - `envelope`: relay WebSocket frame (`Envelope` + sub-enums)
//! - `messages`: typed request / response payloads
//! - `rpc`:      jsonrpsee `#[rpc]` trait shared by daemon (server) and mobile (client)

#![forbid(unsafe_code)]

pub mod auth;
pub mod envelope;
pub mod local_rpc;
pub mod messages;
pub mod realtime;
pub mod rpc;
pub mod ws_ticket;

pub use auth::*;
pub use envelope::*;
pub use local_rpc::*;
pub use messages::*;
pub use realtime::*;
pub use rpc::*;
pub use ws_ticket::*;
