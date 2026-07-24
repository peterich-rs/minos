use crate::state_machine::{CloseReason, PauseReason, SessionState};
use crate::AgentKind;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum ManagerEvent {
    SessionAdded {
        session_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        parent_session_id: Option<String>,
    },
    SessionStateChanged {
        session_id: String,
        old: SessionState,
        new: SessionState,
        at_ms: i64,
    },
    SessionClosed {
        session_id: String,
        reason: CloseReason,
    },
    InstanceCrashed {
        workspace: PathBuf,
        affected_threads: Vec<String>,
        reason: PauseReason,
    },
}
