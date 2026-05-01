use crate::codex_client::CodexClient;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, mpsc};

pub struct AppServerInstance {
    pub workspace: PathBuf,
    #[allow(dead_code)]
    pub(crate) child: Mutex<Option<tokio::process::Child>>,
    #[allow(dead_code)]
    pub(crate) client: Arc<CodexClient>,
    pub threads: Mutex<HashSet<String>>,
    pub spawned_at: Instant,
    pub last_activity_at: Mutex<Instant>,
    pub crash_signal: mpsc::Sender<()>,
}

impl AppServerInstance {
    #[allow(dead_code)]
    pub(crate) fn new(
        workspace: PathBuf,
        child: tokio::process::Child,
        client: Arc<CodexClient>,
        crash_signal: mpsc::Sender<()>,
    ) -> Self {
        let now = Instant::now();
        Self {
            workspace,
            child: Mutex::new(Some(child)),
            client,
            threads: Mutex::new(HashSet::new()),
            spawned_at: now,
            last_activity_at: Mutex::new(now),
            crash_signal,
        }
    }

    pub async fn touch(&self) {
        *self.last_activity_at.lock().await = Instant::now();
    }

    pub async fn add_thread(&self, thread_id: String) {
        self.threads.lock().await.insert(thread_id);
    }

    pub async fn remove_thread(&self, thread_id: &str) {
        self.threads.lock().await.remove(thread_id);
    }

    pub async fn thread_ids(&self) -> Vec<String> {
        self.threads.lock().await.iter().cloned().collect()
    }
}
