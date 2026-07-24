//! Pure application business state. Backend handles remain owned by `App`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::teamwork::TeamworkStore;

mod ingest_dedup;
mod selection;
mod workspace_filter;

pub(crate) use ingest_dedup::*;
pub(crate) use selection::*;
pub(crate) use workspace_filter::*;

pub struct AppState {
    pub workspace: PathBuf,
    pub hydrated_threads: HashSet<String>,
    pub session_watermarks: HashMap<String, u64>,
    pub applied_ingest_fingerprints: HashSet<String>,
    pub teamwork_store: TeamworkStore,
    pub recorded_agent_results: HashMap<String, String>,
    pub session_conversations: HashMap<String, String>,
    pub last_daemon_history_sync: Option<Instant>,
}

impl AppState {
    pub fn new(workspace: PathBuf, teamwork_store: TeamworkStore) -> Self {
        Self {
            workspace,
            hydrated_threads: HashSet::new(),
            session_watermarks: HashMap::new(),
            applied_ingest_fingerprints: HashSet::new(),
            teamwork_store,
            recorded_agent_results: HashMap::new(),
            session_conversations: HashMap::new(),
            last_daemon_history_sync: None,
        }
    }
}
