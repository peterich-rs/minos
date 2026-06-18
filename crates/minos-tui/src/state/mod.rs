//! Pure application business state. Backend handles remain owned by `App`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::group_chat::GroupChatStore;

mod ingest_dedup;
mod selection;
mod workspace_filter;

pub(crate) use ingest_dedup::*;
pub(crate) use selection::*;
pub(crate) use workspace_filter::*;

pub struct AppState {
    pub workspace: PathBuf,
    pub hydrated_threads: HashSet<String>,
    pub thread_watermarks: HashMap<String, u64>,
    pub applied_ingest_fingerprints: HashSet<String>,
    pub group_chat_store: GroupChatStore,
    pub recorded_agent_results: HashMap<String, String>,
    pub last_daemon_history_sync: Option<Instant>,
    pub last_group_result_retry: Option<Instant>,
}

impl AppState {
    pub fn new(workspace: PathBuf, group_chat_store: GroupChatStore) -> Self {
        Self {
            workspace,
            hydrated_threads: HashSet::new(),
            thread_watermarks: HashMap::new(),
            applied_ingest_fingerprints: HashSet::new(),
            group_chat_store,
            recorded_agent_results: HashMap::new(),
            last_daemon_history_sync: None,
            last_group_result_retry: None,
        }
    }
}
