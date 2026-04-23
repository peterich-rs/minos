//! `minos-agent-runtime` — owns the codex (and later claude / gemini) child
//! process, speaks its native JSON-RPC, translates notifications into
//! [`minos_domain::AgentEvent`], and exposes an `AgentRuntime` handle the
//! daemon wires up.
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
//! This crate does **not** depend on `minos-protocol`. `AgentEvent` lives in
//! `minos-domain::events`; `minos-protocol` re-exports it for backward
//! compatibility with existing downstream imports. See spec §5.1 / §5.2.

#![forbid(unsafe_code)]

pub mod approvals;
pub(crate) mod codex_client;
pub(crate) mod process;
pub mod runtime;
pub mod state;
pub mod translate;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use approvals::build_auto_reject;
pub use runtime::{AgentRuntime, AgentRuntimeConfig, StartAgentOutcome};
pub use state::AgentState;
pub use translate::translate_notification;
