//! UniFFI surface for Swift.
//!
//! The source crates carry the actual UniFFI derives and remote custom-type
//! registrations behind their `uniffi` features. This shim just aggregates the
//! exported surface that Swift binds against.

#![allow(clippy::unused_async)]

use minos_domain::{ErrorKind, Lang, MinosError};

uniffi::setup_scaffolding!();

/// Initialize the Rust-side mars-xlog writer once at app startup.
#[uniffi::export]
pub fn init_logging() -> Result<(), MinosError> {
    minos_daemon::logging::init()
}

/// Toggle the daemon log level at runtime.
#[uniffi::export]
pub fn set_debug(enabled: bool) {
    minos_daemon::logging::set_debug(enabled);
}

/// Return the flushed current-day log file path as a UTF-8 string.
#[uniffi::export]
pub fn today_log_path() -> Result<String, MinosError> {
    minos_daemon::logging::today().map(|path| path.to_string_lossy().into_owned())
}

/// Bridge Rust's single-source-of-truth error copy to Swift.
#[uniffi::export]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}

pub use minos_agent_runtime::AgentState;
pub use minos_daemon::{
    AgentStateObserver, DaemonHandle, PeerRecord, PeerStateObserver, RelayConfig,
    RelayLinkStateObserver, RelayQrPayload, Subscription,
};
pub use minos_domain::{
    AgentDescriptor, AgentName, AgentStatus, DeviceId, DeviceSecret, PeerState, RelayLinkState,
};
pub use minos_protocol::{SendUserMessageRequest, StartAgentRequest, StartAgentResponse};
