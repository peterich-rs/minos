//! Minos JSON-RPC 2.0 contract.
//!
//! - `auth`:     HTTP DTOs for `/v1/auth/*` (supabase / refresh / logout)
//! - `messages`: typed request / response payloads
//! - `realtime`: formal topic gateway WS frames (`ClientFrame` / `ServerFrame`)
//! - `rpc`:      jsonrpsee `#[rpc]` trait shared by daemon (server) and mobile (client)

#![forbid(unsafe_code)]

pub mod auth;
pub mod local_rpc;
pub mod messages;
pub mod realtime;
pub mod rpc;
pub mod ws_ticket;

pub use auth::*;
pub use local_rpc::*;
pub use messages::*;
pub use realtime::*;
pub use rpc::*;
pub use ws_ticket::*;
