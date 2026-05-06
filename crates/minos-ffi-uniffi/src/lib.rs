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

/// Emit a Swift-originated log record through the Rust `tracing` pipeline.
#[uniffi::export]
pub fn swift_log_debug(category: String, message: String) {
    tracing::debug!(
        target: "minos_macos_swift",
        source = "swift",
        subsystem = "ai.minos.macos",
        category = %category,
        swift_message = %message,
        "{message}"
    );
}

/// Emit a Swift-originated log record through the Rust `tracing` pipeline.
#[uniffi::export]
pub fn swift_log_info(category: String, message: String) {
    tracing::info!(
        target: "minos_macos_swift",
        source = "swift",
        subsystem = "ai.minos.macos",
        category = %category,
        swift_message = %message,
        "{message}"
    );
}

/// Emit a Swift-originated log record through the Rust `tracing` pipeline.
#[uniffi::export]
pub fn swift_log_warn(category: String, message: String) {
    tracing::warn!(
        target: "minos_macos_swift",
        source = "swift",
        subsystem = "ai.minos.macos",
        category = %category,
        swift_message = %message,
        "{message}"
    );
}

/// Emit a Swift-originated log record through the Rust `tracing` pipeline.
#[uniffi::export]
pub fn swift_log_error(category: String, message: String) {
    tracing::error!(
        target: "minos_macos_swift",
        source = "swift",
        subsystem = "ai.minos.macos",
        category = %category,
        swift_message = %message,
        "{message}"
    );
}

/// Bridge Rust's single-source-of-truth error copy to Swift.
#[uniffi::export]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}

pub use minos_daemon::{
    AgentStateObserver, DaemonHandle, PeerRecord, PeerStateObserver, RelayConfig,
    RelayLinkStateObserver, RelayQrPayload, Subscription,
};
pub use minos_domain::{
    AgentDescriptor, AgentName, AgentStatus, DeviceId, DeviceSecret, PeerState, RelayLinkState,
};
// `ThreadState` / `PauseReason` / `CloseReason` are exposed to Swift as the
// `minos_agent_runtime` enums. `minos_protocol` carries serde-only mirrors for
// JSON-RPC (mobile) traffic — see `crates/minos-protocol/src/messages.rs` for
// rationale on why those mirrors do not derive `uniffi::*`.
pub use minos_agent_runtime::{CloseReason, PauseReason, ThreadState};
pub use minos_protocol::{
    AgentLaunchMode, CloseThreadRequest, HostPeerSummary, InterruptThreadRequest,
    SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
};
pub use minos_ui_protocol::ThreadEndReason;
