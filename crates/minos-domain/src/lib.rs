//! Minos domain types — pure values, no I/O, no async.
//!
//! Module layout follows hexagonal "Entities" concerns:
//! - `ids`         identifier newtypes (DeviceId, PairingToken)
//! - `agent`       AgentName / AgentStatus / AgentDescriptor
//! - `connection`  ConnectionState
//! - `pairing_state`  PairingState (used inside MinosError)
//! - `error`       Lang, ErrorKind, MinosError + user_message

#![forbid(unsafe_code)]

pub mod agent;
pub mod connection;
pub mod error;
pub mod ids;
pub mod pairing_state;

pub use agent::*;
pub use connection::*;
pub use error::*;
pub use ids::*;
pub use pairing_state::*;
