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
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::RawIngest;
use crate::manager_event::ManagerEvent;
use crate::state_machine::ThreadState;
use crate::thread_handle::ThreadHandle;

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
        session_id: Option<&str>,
        resume_session_id: Option<&str>,
        threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
        manager_tx: broadcast::Sender<ManagerEvent>,
        events_tx: broadcast::Sender<RawIngest>,
        subprocess_env: &Arc<HashMap<String, String>>,
        mcp_config_json: Option<&str>,
    ) -> anyhow::Result<Self> {
        let args = build_claude_args(user_text, session_id, resume_session_id, mcp_config_json);

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
            let threads = threads.clone();
            let manager_tx = manager_tx.clone();
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
                    sync_thread_from_payload(&payload, &tid, &threads, &manager_tx).await;
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
            claude_session_id: resume_session_id.or(session_id).map(str::to_string),
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

fn build_claude_args(
    user_text: &str,
    session_id: Option<&str>,
    resume_session_id: Option<&str>,
    mcp_config_json: Option<&str>,
) -> Vec<String> {
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
    } else if let Some(sid) = session_id {
        args.push("--session-id".into());
        args.push(sid.into());
    }
    if let Some(config_json) = mcp_config_json {
        args.push("--mcp-config".into());
        args.push(config_json.into());
        args.push("--strict-mcp-config".into());
    }
    args.push("--append-system-prompt".into());
    args.push(crate::manager::MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS.into());
    args
}

async fn sync_thread_from_payload(
    payload: &Value,
    thread_id: &str,
    threads: &Arc<Mutex<HashMap<String, ThreadHandle>>>,
    manager_tx: &broadcast::Sender<ManagerEvent>,
) {
    let session_id = payload.get("session_id").and_then(Value::as_str);
    let should_idle = matches!(
        payload.get("type").and_then(Value::as_str),
        Some("result") | Some("error")
    );

    let maybe_transition = {
        let mut guard = threads.lock().await;
        let Some(handle) = guard.get_mut(thread_id) else {
            return;
        };

        if let Some(session_id) = session_id {
            handle.codex_session_id = Some(session_id.to_string());
        }

        if should_idle {
            handle.set_active_turn_id(None);
            let old = handle.current_state();
            if matches!(old, ThreadState::Running { .. } | ThreadState::Resuming) {
                if handle.transition(ThreadState::Idle).is_ok() {
                    Some((old, ThreadState::Idle))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some((old, new)) = maybe_transition {
        let _ = manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.to_string(),
            old,
            new,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_event::ManagerEvent;
    use crate::state_machine::ThreadState;
    use crate::thread_handle::ThreadHandle;

    fn val(s: &str) -> Value {
        serde_json::from_str(s).expect("json fixture should parse")
    }

    #[test]
    fn claude_args_include_mcp_config_and_minos_system_prompt_append() {
        let args = build_claude_args(
            "hello",
            Some("session-1"),
            None,
            Some(r#"{"mcpServers":{"minos_chat":{"command":"minos-chat-mcp"}}}"#),
        );

        assert!(args.windows(2).any(|pair| pair == ["-p", "hello"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--session-id", "session-1"]));
        assert!(args
            .windows(2)
            .any(|pair| { pair[0] == "--mcp-config" && pair[1].contains(r#""minos_chat""#) }));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--append-system-prompt"
                && pair[1].contains("Minos teamwork mode")
                && pair[1].contains("minos_chat")
        }));
    }

    #[test]
    fn claude_payload_exposes_session_id() {
        let payload = val(
            r#"{"type":"stream_event","session_id":"sess_1","event":{"type":"message_start"}}"#,
        );
        assert_eq!(
            payload.get("session_id").and_then(Value::as_str),
            Some("sess_1")
        );
    }

    #[test]
    fn result_payload_is_terminal() {
        let payload = val(r#"{"type":"result","is_error":false}"#);
        assert!(matches!(
            payload.get("type").and_then(Value::as_str),
            Some("result")
        ));
    }

    #[tokio::test]
    async fn sync_thread_from_result_updates_state_and_session_id() {
        let thread_id = "thr_claude".to_string();
        let threads = Arc::new(Mutex::new(HashMap::new()));
        threads.lock().await.insert(
            thread_id.clone(),
            ThreadHandle::new(
                thread_id.clone(),
                "/tmp".into(),
                AgentName::Claude,
                ThreadState::Running {
                    turn_started_at_ms: 1,
                },
                0,
            ),
        );
        let (manager_tx, mut manager_rx) = broadcast::channel::<ManagerEvent>(8);

        sync_thread_from_payload(
            &val(r#"{"type":"result","session_id":"sess_resume","is_error":false}"#),
            &thread_id,
            &threads,
            &manager_tx,
        )
        .await;

        let guard = threads.lock().await;
        let handle = guard.get(&thread_id).expect("thread should exist");
        assert!(matches!(handle.current_state(), ThreadState::Idle));
        assert_eq!(handle.codex_session_id.as_deref(), Some("sess_resume"));
        drop(guard);

        let event = manager_rx
            .recv()
            .await
            .expect("manager event should be emitted");
        assert!(matches!(
            event,
            ManagerEvent::ThreadStateChanged { thread_id, new: ThreadState::Idle, .. }
                if thread_id == "thr_claude"
        ));
    }
}
