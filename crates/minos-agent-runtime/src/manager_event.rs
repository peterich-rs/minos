use crate::AgentKind;
use crate::state_machine::{CloseReason, ThreadState};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum ManagerEvent {
    ThreadAdded {
        thread_id: String,
        workspace: PathBuf,
        agent: AgentKind,
    },
    ThreadStateChanged {
        thread_id: String,
        old: ThreadState,
        new: ThreadState,
        at_ms: i64,
    },
    ThreadClosed {
        thread_id: String,
        reason: CloseReason,
    },
    InstanceCrashed {
        workspace: PathBuf,
        affected_threads: Vec<String>,
    },
}
