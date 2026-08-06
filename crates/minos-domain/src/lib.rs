//! Minos domain types — pure values, no I/O, no async.
//!
//! Module layout follows hexagonal "Entities" concerns:
//! - `ids`         identifier newtypes (DeviceId, DeviceSecret, PairingToken)
//! - `agent`       AgentName / AgentStatus / AgentDescriptor / ModelDiscovery
//! - `connection`  ConnectionState
//! - `pairing_state`  PairingState (used inside MinosError)
//! - `role`        DeviceRole (AgentHost / MobileClient / BrowserAdmin)
//! - `error`       Lang, ErrorKind, MinosError + user_message

#![forbid(unsafe_code)]

pub mod agent;
pub mod connection;
pub mod defaults;
pub mod error;
pub mod ids;
pub mod pairing_state;
pub mod relay_state;
pub mod role;

pub use agent::*;
pub use connection::*;
pub use error::*;
pub use ids::*;
pub use pairing_state::*;
pub use relay_state::{PeerState, RelayLinkState};
pub use role::*;
