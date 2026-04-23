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
pub mod events;
pub mod ids;
pub mod pairing_state;

pub use agent::*;
pub use connection::*;
pub use error::*;
pub use events::*;
pub use ids::*;
pub use pairing_state::*;

// UniFFI 0.31 per-crate scaffolding: every crate that carries `uniffi::*`
// derives must define `UniFfiTag` locally via `setup_scaffolding!()`; the
// derive expansions reference `crate::UniFfiTag`. Feature-gated so the
// non-UniFFI build path (plan-03 Dart/frb consumers) pays nothing.
#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
