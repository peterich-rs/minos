#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use minos_domain::AgentName;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::RawIngest;

const KILL_ESCALATION: Duration = Duration::from_secs(3);

pub struct ClaudeNdjsonSession {
    pub thread_id: String,
    pub workspace: PathBuf,
    pub claude_session_id: Option<String>,
    pub(crate) current_turn_child: Option<Child>,
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
}

impl ClaudeNdjsonSession {
    pub async fn start_turn(
        cli_path: &Path,
        workspace: &Path,
        thread_id: String,
        user_text: &str,
        resume_session_id: Option<&str>,
        events_tx: broadcast::Sender<RawIngest>,
        subprocess_env: &Arc<HashMap<String, String>>,
    ) -> anyhow::Result<Self> {
        let mut args: Vec<String> = vec![
            "-p".into(),
            user_text.into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
        ];
        if let Some(sid) = resume_session_id {
            args.push("--resume".into());
            args.push(sid.into());
        }

        let mut cmd = Command::new(cli_path);
        cmd.args(&args)
            .current_dir(workspace)
            .env_clear()
            .envs(subprocess_env.iter())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        {
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
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_task = stdout.map(|out| {
            let tx = events_tx.clone();
            let tid = thread_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(out);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let payload: Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => serde_json::json!({
                            "kind": "raw",
                            "raw_kind": "stdout",
                            "payload_json": serde_json::to_string(&line).unwrap_or_default()
                        }),
                    };
                    let _ = tx.send(RawIngest {
                        agent: AgentName::Claude,
                        thread_id: tid.clone(),
                        payload,
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
            })
        });

        let stderr_task = stderr.map(|err_stream| {
            tokio::spawn(async move {
                let reader = BufReader::new(err_stream);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(
                        target: "minos_agent_runtime::claude_driver",
                        line = %line,
                        "claude stderr"
                    );
                }
            })
        });

        info!(
            target: "minos_agent_runtime::claude_driver",
            cli = %cli_path.display(),
            workspace = %workspace.display(),
            thread_id = %thread_id,
            "claude ndjson session started"
        );

        Ok(Self {
            thread_id,
            workspace: workspace.to_path_buf(),
            claude_session_id: None,
            current_turn_child: Some(child),
            stdout_task,
            stderr_task,
        })
    }

    pub fn set_claude_session_id(&mut self, id: String) {
        self.claude_session_id = Some(id);
    }

    pub fn claude_session_id(&self) -> Option<&str> {
        self.claude_session_id.as_deref()
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub async fn close(mut self, events_tx: &broadcast::Sender<RawIngest>) {
        if let Some(mut child) = self.current_turn_child.take() {
            #[cfg(unix)]
            {
                let pid = child.id();
                if let Some(pid) = pid {
                    #[allow(clippy::cast_possible_wrap)]
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }

            let exited = tokio::time::timeout(KILL_ESCALATION, child.wait()).await;
            if exited.is_err() {
                warn!(
                    target: "minos_agent_runtime::claude_driver",
                    thread_id = %self.thread_id,
                    "claude did not exit after SIGTERM, sending SIGKILL"
                );
                let _ = child.kill().await;
            }
        }

        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        let _ = events_tx.send(RawIngest {
            agent: AgentName::Claude,
            thread_id: self.thread_id.clone(),
            payload: serde_json::json!({
                "kind": "thread_closed",
                "thread_id": self.thread_id,
                "reason": { "kind": "user_stopped" },
                "closed_at_ms": chrono::Utc::now().timestamp_millis()
            }),
            ts_ms: chrono::Utc::now().timestamp_millis(),
        });

        info!(
            target: "minos_agent_runtime::claude_driver",
            thread_id = %self.thread_id,
            "claude ndjson session closed"
        );
    }
}
