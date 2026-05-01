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
//! `AppServerInstance` per workspace and N `ThreadHandle`s per instance. See
//! `docs/superpowers/specs/2026-05-01-agent-session-manager-and-minos-home-design.md`
//! for the design intent.
//!
//! ## Dependency rule
//!
//! This crate does **not** depend on `minos-protocol`. It also deliberately
//! does NOT depend on `minos-ui-protocol` — the `UiEventMessage` translator
//! lives in the backend so the host daemon stays a thin ingest pipe.

#![forbid(unsafe_code)]

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub(crate) mod approvals;
pub(crate) mod codex_client;
pub mod config;
pub mod ingest;
pub mod instance;
pub mod manager;
pub mod manager_event;
pub(crate) mod process;
pub mod state_machine;
pub mod store_facing;
pub mod thread_handle;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use config::{AgentLaunchMode, AgentRuntimeConfig, RawIngest};
pub use ingest::{Ingestor, IngestorHandle};
pub use instance::AppServerInstance;
pub use manager::{AgentManager, InstanceCaps, StartAgentOutcome};
pub use manager_event::ManagerEvent;
pub use minos_domain::AgentName as AgentKind;
pub use state_machine::{CloseReason, PauseReason, ThreadState};
pub use thread_handle::ThreadHandle;
