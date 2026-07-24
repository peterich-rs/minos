use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SessionState {
    Starting,
    Idle,
    Running { turn_started_at_ms: i64 },
    Suspended { reason: PauseReason },
    Resuming,
    Closed { reason: CloseReason },
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    UserInterrupt,
    CodexCrashed,
    DaemonRestart,
    InstanceReaped,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    UserClose,
    TerminalError,
}

#[derive(Debug, thiserror::Error)]
#[error("illegal session state transition: {from:?} -> {to:?}")]
pub struct IllegalTransition {
    pub from: SessionState,
    pub to: SessionState,
}

#[allow(clippy::unnested_or_patterns, clippy::enum_glob_use)]
pub fn validate_transition(
    from: &SessionState,
    to: &SessionState,
) -> Result<(), IllegalTransition> {
    use SessionState::*;
    let ok = matches!(
        (from, to),
        (Starting, Idle)
            | (Idle, Running { .. })
            | (Running { .. }, Idle)
            | (Running { .. }, Suspended { .. })
            | (Idle, Suspended { .. })
            // Daemon stop / process-death recovery may suspend mid-flight.
            | (Starting, Suspended { .. })
            | (Resuming, Suspended { .. })
            // Allow pause-reason rewrite (e.g. UserInterrupt → DaemonRestart on stop).
            | (Suspended { .. }, Suspended { .. })
            | (Suspended { .. }, Resuming)
            | (Resuming, Idle)
            | (
                Resuming,
                Closed {
                    reason: CloseReason::TerminalError
                }
            )
            | (Starting, Closed { .. })
            | (Idle, Closed { .. })
            | (Running { .. }, Closed { .. })
            | (Suspended { .. }, Closed { .. })
            | (Resuming, Closed { .. })
    );
    if ok {
        Ok(())
    } else {
        Err(IllegalTransition {
            from: from.clone(),
            to: to.clone(),
        })
    }
}

pub fn status_str(state: &SessionState) -> &'static str {
    match state {
        SessionState::Starting => "starting",
        SessionState::Idle => "idle",
        SessionState::Running { .. } => "running",
        SessionState::Suspended { .. } => "suspended",
        SessionState::Resuming => "resuming",
        SessionState::Closed { .. } => "closed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_transition_idle_to_running() {
        validate_transition(
            &SessionState::Idle,
            &SessionState::Running {
                turn_started_at_ms: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn illegal_transition_running_to_starting() {
        let err = validate_transition(
            &SessionState::Running {
                turn_started_at_ms: 1,
            },
            &SessionState::Starting,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("illegal"));
    }

    #[test]
    fn suspended_can_resume_or_close() {
        validate_transition(
            &SessionState::Suspended {
                reason: PauseReason::UserInterrupt,
            },
            &SessionState::Resuming,
        )
        .unwrap();
        validate_transition(
            &SessionState::Suspended {
                reason: PauseReason::UserInterrupt,
            },
            &SessionState::Closed {
                reason: CloseReason::UserClose,
            },
        )
        .unwrap();
    }

    #[test]
    fn starting_and_resuming_can_suspend_for_daemon_stop() {
        validate_transition(
            &SessionState::Starting,
            &SessionState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
        )
        .unwrap();
        validate_transition(
            &SessionState::Resuming,
            &SessionState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
        )
        .unwrap();
        validate_transition(
            &SessionState::Suspended {
                reason: PauseReason::UserInterrupt,
            },
            &SessionState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
        )
        .unwrap();
    }
}
