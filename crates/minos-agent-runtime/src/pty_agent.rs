//! `PtyAgent` — minimal PTY/process adapter for legacy line-oriented CLIs.
//!
//! Spawns the CLI as a child process, pipes stdin/stdout/stderr, and emits
//! `RawIngest` events with `raw_kind: "stdout" | "stderr"` for each line of
//! output. The backend's ingest pipeline will wrap these as
//! `UiEventMessage::Raw` for fan-out to clients.
//!
//! Historical R3.4 behavior spawned a CLI, piped stdout/stderr lines as
//! `UiEventMessage::Raw`, and piped composer text into stdin. Current Gemini
//! support uses ACP instead of this adapter.
//!
//! ## Design decisions
//!
//! - Uses `tokio::process::Command` (not `portable-pty`) per plan §open
//!   decision #1. Revisit only if a supported line-oriented CLI refuses output
//!   without a TTY.
//! - Line-buffers stdout+stderr to avoid partial-line fan-out.
//! - SIGTERM → 3s → SIGKILL escalation on close.

// Module-local allow for the single `setpgid(2)` call in `pre_exec`.
#![allow(unsafe_code)]

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use minos_domain::AgentName;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::RawIngest;
use crate::manager::IngestSink;

/// Timeout before escalating from SIGTERM to SIGKILL.
const KILL_ESCALATION: Duration = Duration::from_secs(3);

/// A running PTY-style agent process.
pub struct PtyAgent {
    agent: AgentName,
    thread_id: String,
    child: Option<Child>,
    stdin_handle: Option<tokio::process::ChildStdin>,
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
}

impl PtyAgent {
    /// Spawn the CLI at `cli_path` in the given `workspace` directory.
    ///
    /// Returns a `PtyAgent` that immediately starts reading stdout/stderr
    /// and broadcasting lines as `RawIngest` events.
    pub fn spawn(
        cli_path: &Path,
        workspace: &Path,
        agent: AgentName,
        thread_id: String,
        events_tx: IngestSink,
    ) -> Result<Self, anyhow::Error> {
        let mut cmd = Command::new(cli_path);
        cmd.current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        {
            // SAFETY: setpgid(0,0) is async-signal-safe.
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_task = stdout.map(|out| {
            let tx = events_tx.clone();
            let tid = thread_id.clone();
            let ag = agent;
            tokio::spawn(async move {
                let reader = BufReader::new(out);
                let mut lines = reader.lines();
                let mut seq: u64 = 0;
                while let Ok(Some(line)) = lines.next_line().await {
                    seq += 1;
                    tx.emit(RawIngest::from_json(
                        ag,
                        tid.clone(),
                        json!({
                            "kind": "raw",
                            "raw_kind": "stdout",
                            "payload_json": serde_json::to_string(&line).unwrap_or_default()
                        }),
                        chrono::Utc::now().timestamp_millis(),
                    ));
                    let _ = seq; // suppress unused warning in non-debug
                }
            })
        });

        let stderr_task = stderr.map(|err_stream| {
            let tx = events_tx.clone();
            let tid = thread_id.clone();
            let ag = agent;
            tokio::spawn(async move {
                let reader = BufReader::new(err_stream);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tx.emit(RawIngest::from_json(
                        ag,
                        tid.clone(),
                        json!({
                            "kind": "raw",
                            "raw_kind": "stderr",
                            "payload_json": serde_json::to_string(&line).unwrap_or_default()
                        }),
                        chrono::Utc::now().timestamp_millis(),
                    ));
                }
            })
        });

        info!(
            agent = ?agent,
            thread_id = %thread_id,
            cli = %cli_path.display(),
            workspace = %workspace.display(),
            "pty agent spawned"
        );

        Ok(Self {
            agent,
            thread_id,
            child: Some(child),
            stdin_handle: stdin,
            stdout_task,
            stderr_task,
        })
    }

    /// Send a user message to the agent's stdin (appends newline).
    pub async fn send_user_message(&mut self, text: &str) -> anyhow::Result<()> {
        let stdin = self
            .stdin_handle
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("stdin not available"))?;
        stdin.write_all(text.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Gracefully close the agent: SIGTERM, wait up to 3s, then SIGKILL.
    /// Emits a `thread_closed` RawIngest event.
    pub async fn close(mut self, events_tx: &IngestSink) {
        if let Some(mut child) = self.child.take() {
            // Try graceful termination first
            #[cfg(unix)]
            {
                let pid = child.id();
                if let Some(pid) = pid {
                    // SAFETY: kill(2) is async-signal-safe.
                    #[allow(clippy::cast_possible_wrap)]
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }

            let exited = tokio::time::timeout(KILL_ESCALATION, child.wait()).await;
            if exited.is_err() {
                warn!(thread_id = %self.thread_id, "pty agent did not exit after SIGTERM, sending SIGKILL");
                let _ = child.kill().await;
            }
        }

        // Abort reader tasks
        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        // Emit thread_closed event
        events_tx.emit(RawIngest::from_json(
            self.agent,
            self.thread_id.clone(),
            json!({
                "kind": "thread_closed",
                "thread_id": self.thread_id,
                "reason": { "kind": "user_stopped" },
                "closed_at_ms": chrono::Utc::now().timestamp_millis()
            }),
            chrono::Utc::now().timestamp_millis(),
        ));

        info!(thread_id = %self.thread_id, "pty agent closed");
    }

    /// Returns the thread ID this agent is bound to.
    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Returns the agent kind.
    #[must_use]
    pub fn agent(&self) -> AgentName {
        self.agent
    }
}
