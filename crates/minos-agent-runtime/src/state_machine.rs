use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ThreadState {
    Starting,
    Idle,
    Running { turn_started_at_ms: i64 },
    Suspended { reason: PauseReason },
    Resuming,
    Closed { reason: CloseReason },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    UserInterrupt,
    CodexCrashed,
    DaemonRestart,
    InstanceReaped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    UserClose,
    TerminalError,
}

#[derive(Debug, thiserror::Error)]
#[error("illegal thread state transition: {from:?} -> {to:?}")]
pub struct IllegalTransition {
    pub from: ThreadState,
    pub to: ThreadState,
}

pub fn validate_transition(
    from: &ThreadState,
    to: &ThreadState,
) -> Result<(), IllegalTransition> {
    use ThreadState::*;
    let ok = matches!(
        (from, to),
        (Starting, Idle)
            | (Idle, Running { .. })
            | (Running { .. }, Idle)
            | (Running { .. }, Suspended { .. })
            | (Idle, Suspended { .. })
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

pub fn status_str(state: &ThreadState) -> &'static str {
    match state {
        ThreadState::Starting => "starting",
        ThreadState::Idle => "idle",
        ThreadState::Running { .. } => "running",
        ThreadState::Suspended { .. } => "suspended",
        ThreadState::Resuming => "resuming",
        ThreadState::Closed { .. } => "closed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_transition_idle_to_running() {
        validate_transition(
            &ThreadState::Idle,
            &ThreadState::Running {
                turn_started_at_ms: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn illegal_transition_running_to_starting() {
        let err = validate_transition(
            &ThreadState::Running {
                turn_started_at_ms: 1,
            },
            &ThreadState::Starting,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("illegal"));
    }

    #[test]
    fn suspended_can_resume_or_close() {
        validate_transition(
            &ThreadState::Suspended {
                reason: PauseReason::UserInterrupt,
            },
            &ThreadState::Resuming,
        )
        .unwrap();
        validate_transition(
            &ThreadState::Suspended {
                reason: PauseReason::UserInterrupt,
            },
            &ThreadState::Closed {
                reason: CloseReason::UserClose,
            },
        )
        .unwrap();
    }
}
