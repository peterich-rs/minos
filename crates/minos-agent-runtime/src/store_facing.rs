use crate::state_machine::ThreadState;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ThreadSnapshot {
    pub thread_id: String,
    pub workspace: PathBuf,
    pub state: ThreadState,
}
