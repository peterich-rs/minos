use crate::state_machine::SessionState;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub workspace: PathBuf,
    pub state: SessionState,
    pub parent_session_id: Option<String>,
}
