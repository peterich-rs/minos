//! Agent state machine — pure value type.
//!
//! See spec §5.1 for the state shape; §6.1 / §6.3 for the transitions that
//! drive it. This module only defines the enum and its serde representation;
//! the state machine itself (the `watch::Sender<AgentState>` driven by
//! `start` / `stop` / crash supervisor) lands in Phase C alongside
//! `AgentRuntime`.

use minos_domain::AgentName;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The state of the single currently-tracked agent session.
///
/// MVP is single-session: exactly one variant is "live" at a time. Multi-session
/// concurrency (§2.2 out-of-scope) is a breaking change deferred to a later spec.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AgentState {
    /// No agent session. Boot default and the resting state after a clean stop.
    #[default]
    Idle,
    /// Between `start_agent` RPC and the runtime minting the session/thread id.
    Starting { agent: AgentName },
    /// Agent session is live and has a thread id. For exec/jsonl-backed
    /// sessions the underlying codex subprocess exists only while a turn is
    /// actively running; between turns the session remains resumable.
    Running {
        agent: AgentName,
        thread_id: String,
        started_at: SystemTime,
    },
    /// Between `stop_agent` RPC and the supervisor's Idle transition.
    Stopping,
    /// Agent child exited without a `stop_agent` call.
    Crashed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(state: &AgentState) -> AgentState {
        let s = serde_json::to_string(state).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn default_is_idle() {
        assert_eq!(AgentState::default(), AgentState::Idle);
    }

    #[test]
    fn idle_round_trips() {
        let state = AgentState::Idle;
        assert_eq!(round_trip(&state), state);
        // Pin the exact JSON so a future rename doesn't silently break
        // downstream consumers.
        let s = serde_json::to_string(&state).unwrap();
        assert_eq!(s, r#"{"state":"idle"}"#);
    }

    #[test]
    fn starting_round_trips() {
        let state = AgentState::Starting {
            agent: AgentName::Codex,
        };
        assert_eq!(round_trip(&state), state);
        let s = serde_json::to_string(&state).unwrap();
        assert_eq!(s, r#"{"state":"starting","agent":"codex"}"#);
    }

    #[test]
    fn running_round_trips() {
        // Use the UNIX epoch — serde_json encodes SystemTime as a struct of
        // seconds+nanoseconds, so a fixed instant makes the round-trip
        // equality check hermetic.
        let state = AgentState::Running {
            agent: AgentName::Codex,
            thread_id: "thread-abc12".into(),
            started_at: SystemTime::UNIX_EPOCH,
        };
        assert_eq!(round_trip(&state), state);
    }

    #[test]
    fn stopping_round_trips() {
        let state = AgentState::Stopping;
        assert_eq!(round_trip(&state), state);
        let s = serde_json::to_string(&state).unwrap();
        assert_eq!(s, r#"{"state":"stopping"}"#);
    }

    #[test]
    fn crashed_round_trips() {
        let state = AgentState::Crashed {
            reason: "exit code 137".into(),
        };
        assert_eq!(round_trip(&state), state);
        let s = serde_json::to_string(&state).unwrap();
        assert_eq!(s, r#"{"state":"crashed","reason":"exit code 137"}"#);
    }
}
