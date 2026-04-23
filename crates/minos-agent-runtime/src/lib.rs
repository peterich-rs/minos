//! `minos-agent-runtime` — owns the codex (and later claude / gemini) child
//! process, speaks its native JSON-RPC, translates notifications into
//! [`minos_domain::AgentEvent`], and exposes an `AgentRuntime` handle the
//! daemon wires up.
//!
//! ## Phase B scope
//!
//! Phase B ships the pure-logic modules only: `state`, `translate`,
//! `approvals`, and the test-support `FakeCodexServer`. The full
//! `AgentRuntime` + `CodexClient` + supervisor lands in Phase C. Until
//! then the facade re-exports the state enum so Phase C tests can depend
//! on this crate without API churn.
//!
//! ## Dependency rule
//!
//! This crate does **not** depend on `minos-protocol`. `AgentEvent` lives in
//! `minos-domain::events`; `minos-protocol` re-exports it for backward
//! compatibility with existing downstream imports. See spec §5.1 / §5.2.

#![forbid(unsafe_code)]

pub mod approvals;
pub mod state;
pub mod translate;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use approvals::build_auto_reject;
pub use state::AgentState;
pub use translate::translate_notification;
