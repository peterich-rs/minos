//! `minos-agent-runtime` — owns the codex (and later claude / gemini) child
//! process(es), speaks their native JSON-RPC, and exposes an `AgentManager`
//! handle the daemon wires up. Raw notifications are forwarded verbatim as
//! [`RawIngest`]; translation to `UiEventMessage` is the backend's
//! responsibility (plan §B6).
//!
//! ## Phase C scope
//!
//! Phase C retired the single-session `AgentRuntime` (lived in `runtime.rs`)
//! and the legacy `AgentState` value object (lived in `state.rs`). The
//! replacement is a multi-workspace `AgentManager` that owns one
//! `AppServerInstance` per workspace and N `SessionHandle`s per instance.
//!
//! ## Dependency rule
//!
//! Runtime emits raw bytes plus lightweight transport metadata. Projection to
//! renderable UI events happens after durable commit; the shared display/artifact
//! value types live in `minos-ui-protocol` so local, backend, and mobile paths
//! use the same wire shape.

// `deny` (not `forbid`) so the per-module allows in `process.rs` and
// `manager.rs` can carve narrow holes for the Unix-only `setpgid(2)` /
// `kill(2)` calls that put each codex child in its own process group and
// signal that group on shutdown. Every `unsafe` block in this crate is
// gated `#[cfg(unix)]` and limited to async-signal-safe libc entry points.
#![deny(unsafe_code)]


pub mod acp_client;
pub(crate) mod approvals;
pub mod claude_driver;
pub(crate) mod codex_client;
pub mod config;
pub mod gemini_driver;
pub mod grok_driver;
pub mod ingest;
pub mod instance;
pub mod manager;
pub mod manager_event;
pub mod opencode_driver;
pub(crate) mod process;
pub mod pty_agent;
pub mod session_handle;
pub mod state_machine;
pub mod store_facing;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use claude_driver::ClaudeNdjsonSession;
pub use config::{
    AgentEventProjection, AgentLaunchMode, AgentRuntimeConfig, RawBody, RawIngest, TextLane,
    ToolStatus, ToolStream, INLINE_RAW_BODY_THRESHOLD,
};
pub use gemini_driver::GeminiAcpInstance;
pub use grok_driver::GrokAcpInstance;
pub use ingest::{Ingestor, IngestorHandle};
pub use instance::AppServerInstance;
pub use manager::{
    AgentLaunchOptions, AgentManager, DispatchOutcome, IngestSink, InstanceCaps, SessionPolicies,
    StartAgentOutcome, CONTINUE_PROMPT,
};
pub use manager_event::ManagerEvent;
pub use minos_domain::AgentName as AgentKind;
pub use opencode_driver::OpencodeServerInstance;
pub use pty_agent::PtyAgent;
pub use session_handle::SessionHandle;
pub use state_machine::{CloseReason, PauseReason, SessionState};
