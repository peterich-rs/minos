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

    /// Start a fresh thread on this instance via the codex `thread/start`
    /// JSON-RPC. The real codex spawn lands in C12; until then this returns
    /// an error so callers can drive coverage of the surrounding orchestration.
    pub(crate) async fn start_thread(&self) -> anyhow::Result<String> {
        anyhow::bail!("start_thread unimplemented (C12)")
    }

    /// Resume an existing codex thread under the same `thread_id`. Real
    /// implementation lands in C13.
    #[allow(dead_code)]
    pub(crate) async fn start_thread_resume(
        &self,
        _thread_id: &str,
        _codex_session_id: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("start_thread_resume unimplemented (C13)")
    }

    /// Forward a user turn to the codex app-server. Real implementation lands
    /// in C12.
    pub(crate) async fn send_user_message(
        &self,
        _thread_id: &str,
        _text: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("send_user_message unimplemented (C12)")
    }

    /// Best-effort interrupt of an in-flight turn. Real implementation lands
    /// in C12.
    #[allow(dead_code)]
    pub(crate) async fn interrupt_turn(&self, _thread_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("interrupt_turn unimplemented (C12)")
    }
}
