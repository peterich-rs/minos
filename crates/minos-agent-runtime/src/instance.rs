use crate::codex_client::CodexClient;
use minos_codex_protocol::{ThreadResumeParams, TurnInterruptParams, TurnStartParams, UserInput};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

/// Default RPC timeouts when issuing per-turn calls. Mirrors the figures used
/// by the legacy `runtime.rs` paths so behaviour stays uniform across the
/// runtime rewrite.
const TURN_START_TIMEOUT: Duration = Duration::from_secs(10);
const RESUME_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_FALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AppServerInstance {
    pub workspace: PathBuf,
    #[allow(dead_code)]
    pub(crate) child: Mutex<Option<tokio::process::Child>>,
    pub(crate) client: Arc<CodexClient>,
    pub threads: Mutex<HashSet<String>>,
    pub spawned_at: Instant,
    pub last_activity_at: Mutex<Instant>,
    pub crash_signal: mpsc::Sender<()>,
}

impl AppServerInstance {
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

    /// Issue `thread/start` and return the result describing the new thread
    /// id (which doubles as the codex session id for resume) and cwd metadata.
    pub(crate) async fn start_thread(
        &self,
        cwd: &Path,
    ) -> anyhow::Result<crate::manager::StartThreadResult> {
        crate::manager::rpc_start_thread(&self.client, cwd, HANDSHAKE_FALLBACK_TIMEOUT).await
    }

    /// Resume an existing codex thread under the same `thread_id`.
    #[allow(dead_code)]
    pub(crate) async fn start_thread_resume(
        &self,
        thread_id: &str,
        _codex_session_id: &str,
    ) -> anyhow::Result<()> {
        let params = ThreadResumeParams {
            approval_policy: None,
            approvals_reviewer: None,
            base_instructions: None,
            config: None,
            cwd: None,
            developer_instructions: None,
            exclude_turns: None,
            model: None,
            model_provider: None,
            permission_profile: None,
            personality: None,
            sandbox: None,
            service_tier: None,
            thread_id: thread_id.to_string(),
        };
        tokio::time::timeout(RESUME_TIMEOUT, self.client.call_typed(params))
            .await
            .map_err(|_| anyhow::anyhow!("thread/resume timeout"))?
            .map_err(|e| anyhow::anyhow!("thread/resume failed: {e}"))?;
        Ok(())
    }

    /// Forward a user turn to the codex app-server via `turn/start`.
    pub(crate) async fn send_user_message(
        &self,
        thread_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let params = TurnStartParams {
            approval_policy: None,
            approvals_reviewer: None,
            cwd: None,
            effort: None,
            input: vec![UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
            model: None,
            output_schema: None,
            permission_profile: None,
            personality: None,
            sandbox_policy: None,
            service_tier: None,
            summary: None,
            thread_id: thread_id.to_string(),
        };
        tokio::time::timeout(TURN_START_TIMEOUT, self.client.call_typed(params))
            .await
            .map_err(|_| anyhow::anyhow!("turn/start timeout"))?
            .map_err(|e| anyhow::anyhow!("turn/start failed: {e}"))?;
        Ok(())
    }

    /// Best-effort interrupt of an in-flight turn. Sends `turn/interrupt`; the
    /// codex side responds with an error if there is no active turn — that is
    /// fine, callers always treat interrupt as best-effort.
    #[allow(dead_code)]
    pub(crate) async fn interrupt_turn(&self, thread_id: &str) -> anyhow::Result<()> {
        let params = TurnInterruptParams {
            thread_id: thread_id.to_string(),
            turn_id: String::new(),
        };
        let _ =
            tokio::time::timeout(Duration::from_millis(500), self.client.call_typed(params)).await;
        Ok(())
    }
}
