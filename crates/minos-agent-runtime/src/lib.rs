//! `minos-agent-runtime` — owns the codex (and later claude / gemini) child
//! process, speaks its native JSON-RPC, and exposes an `AgentRuntime` handle
//! the daemon wires up. Raw notifications are forwarded verbatim as
//! [`runtime::RawIngest`]; translation to `UiEventMessage` is the backend's
//! responsibility (plan §B6).
//!
//! ## Phase C scope
//!
//! Phase C lands the three I/O-touching modules — `process`, `codex_client`,
//! `runtime` — plus the `tests/runtime_e2e.rs` integration harness. Together
//! they compose the full state machine that spans
//! `Idle → Starting → Running → Stopping → Idle` (plus the crash-detection
//! branch into `Crashed`). See spec §5.1 for the sequencing and §6.1 / §6.3 /
//! §6.4 for the flow diagrams.
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
pub(crate) mod exec_jsonl;
pub mod ingest;
pub mod manager_event;
pub(crate) mod process;
pub mod runtime;
pub mod state;
pub mod state_machine;
pub mod thread_handle;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use ingest::{Ingestor, IngestorHandle};
pub use manager_event::ManagerEvent;
pub use minos_domain::AgentName as AgentKind;
pub use runtime::{
    AgentLaunchMode, AgentRuntime, AgentRuntimeConfig, RawIngest, StartAgentOutcome,
};
pub use state::AgentState;
pub use state_machine::{CloseReason, PauseReason, ThreadState};
pub use thread_handle::ThreadHandle;
