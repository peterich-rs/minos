#![allow(unsafe_code)]

//! Claude Code stream-json control session (plane A).
//!
//! Long-lived process with piped stdin/stdout NDJSON:
//! - outbound: user turns + `control_response` permission decisions
//! - inbound: stream events + permission `control_request` reverse-requests
//!
//! Legacy one-shot process-per-turn is replaced by this session. When the child
//! dies, the manager re-spawns with `--resume <provider_session_id>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use minos_domain::AgentName;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::RawIngest;
use crate::manager::{IngestSink, PendingApprovals};
use crate::manager_event::ManagerEvent;
use crate::session_handle::SessionHandle;
use crate::state_machine::SessionState;

const KILL_ESCALATION: Duration = Duration::from_secs(3);

/// Long-lived Claude control-plane session (stdin + stdout stream-json).
pub struct ClaudeControlSession {
    pub session_id: String,
    pub workspace: PathBuf,
    pub claude_session_id: Option<String>,
    /// Kept for interrupt hard-kill; None after graceful close / reaped wait.
    pub(crate) current_turn_child: Option<Child>,
    stdin_tx: Option<mpsc::UnboundedSender<String>>,
    stdin_task: Option<JoinHandle<()>>,
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
    process_alive: Arc<AtomicBool>,
    capabilities: Arc<Mutex<Vec<String>>>,
}

/// Backward-compatible name used by manager / tests / lib re-exports.
pub type ClaudeNdjsonSession = ClaudeControlSession;

impl ClaudeControlSession {
    /// Spawn a bidirectional Claude control session and send the first user turn.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start_turn(
        cli_path: &Path,
        workspace: &Path,
        session_id: String,
        user_text: &str,
        claude_session_id: Option<&str>,
        resume_session_id: Option<&str>,
        sessions: Arc<Mutex<HashMap<String, SessionHandle>>>,
        manager_tx: broadcast::Sender<ManagerEvent>,
        events_tx: IngestSink,
        subprocess_env: &Arc<HashMap<String, String>>,
        mcp_config_json: Option<&str>,
        model: Option<&str>,
        extra_instructions: Option<&str>,
        pending_approvals: PendingApprovals,
    ) -> anyhow::Result<Self> {
        let args = build_claude_args(
            user_text,
            claude_session_id,
            resume_session_id,
            mcp_config_json,
            model,
            extra_instructions,
            /* bidirectional */ true,
        );

        let mut cmd = Command::new(cli_path);
        cmd.args(&args)
            .current_dir(workspace)
            .env_clear()
            .envs(subprocess_env.iter())
            .stdin(Stdio::piped())
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
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("claude stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("claude stdout not piped"))?;
        let stderr = child.stderr.take();

        let process_alive = Arc::new(AtomicBool::new(true));
        let capabilities = Arc::new(Mutex::new(Vec::new()));

        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        let stdin_task = tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = stdin_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if !line.ends_with('\n') {
                    if stdin.write_all(b"\n").await.is_err() {
                        break;
                    }
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        let stdout_task = {
            let tx = events_tx.clone();
            let tid = session_id.clone();
            let sessions = sessions.clone();
            let manager_tx = manager_tx.clone();
            let pending_approvals = pending_approvals.clone();
            let capabilities = capabilities.clone();
            let alive = process_alive.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
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

                    if let Some(caps) = payload
                        .get("capabilities")
                        .and_then(Value::as_array)
                        .filter(|_| {
                            payload.get("type").and_then(Value::as_str) == Some("system")
                                && payload.get("subtype").and_then(Value::as_str) == Some("init")
                        })
                    {
                        let list = caps
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>();
                        *capabilities.lock().await = list;
                    }

                    if is_control_permission_request(&payload) {
                        if let Err(error) = register_claude_permission_request(
                            &tid,
                            &payload,
                            &tx,
                            &pending_approvals,
                        )
                        .await
                        {
                            warn!(
                                target: "minos_agent_runtime::claude_driver",
                                error = %error,
                                session_id = %tid,
                                "failed to park Claude permission request",
                            );
                        }
                        // Still forward raw frame for debugging / Raw projection.
                    }

                    sync_session_from_payload(&payload, &tid, &sessions, &manager_tx).await;
                    if let Err(error) = tx
                        .emit(RawIngest::from_json(
                            AgentName::Claude,
                            tid.clone(),
                            payload,
                            chrono::Utc::now().timestamp_millis(),
                        ))
                        .await
                    {
                        warn!(
                            target: "minos_agent_runtime::claude_driver",
                            error = %error,
                            session_id = %tid,
                            "durable ingest sink closed while reading claude stdout",
                        );
                        break;
                    }
                }
                // stdout EOF ⇒ process gone (or pipe closed).
                alive.store(false, Ordering::SeqCst);
            })
        };

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

        // Bidirectional mode opens stdin for the stream-json control plane.
        // Do not leave the first turn only on argv `-p` with an open empty
        // stdin: some Claude CLI builds wait on NDJSON input and never emit
        // system/init (session stuck Running, zero assistant events).
        // Always enqueue the first user frame on stdin; follow-up turns use
        // the same path via `send_user_message`.
        let session = Self {
            session_id: session_id.clone(),
            workspace: workspace.to_path_buf(),
            claude_session_id: resume_session_id.or(claude_session_id).map(str::to_string),
            current_turn_child: Some(child),
            stdin_tx: Some(stdin_tx),
            stdin_task: Some(stdin_task),
            stdout_task: Some(stdout_task),
            stderr_task,
            process_alive,
            capabilities,
        };
        if let Err(error) = session.send_user_message(user_text) {
            warn!(
                target: "minos_agent_runtime::claude_driver",
                error = %error,
                session_id = %session_id,
                "failed to enqueue first Claude stdin user frame; relying on -p only"
            );
        }

        info!(
            target: "minos_agent_runtime::claude_driver",
            cli = %cli_path.display(),
            workspace = %workspace.display(),
            session_id = %session_id,
            "claude control session started"
        );

        Ok(session)
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

    pub fn is_alive(&self) -> bool {
        if !self.process_alive.load(Ordering::SeqCst) {
            return false;
        }
        // Best-effort: if child already exited, try_wait and mark dead.
        // Cannot mutably borrow child easily from &self — use atomic set from stdout end.
        // When stdout reader ends, mark process dead via a clone of the flag in stdout task.
        self.process_alive.load(Ordering::SeqCst)
    }

    /// Write a follow-up user turn on the live stdin control plane.
    pub fn send_user_message(&self, text: &str) -> anyhow::Result<()> {
        let tx = self
            .stdin_tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("claude stdin closed"))?;
        let frame = build_claude_user_frame(text);
        let line = serde_json::to_string(&frame)?;
        tx.send(line)
            .map_err(|_| anyhow::anyhow!("claude stdin writer gone"))?;
        Ok(())
    }

    /// Reply to a parked permission control request.
    pub fn reply_control(
        &self,
        control_request_id: &str,
        response_body: Value,
    ) -> anyhow::Result<()> {
        let tx = self
            .stdin_tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("claude stdin closed"))?;
        let frame = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": control_request_id,
                "response": response_body
            }
        });
        let line = serde_json::to_string(&frame)?;
        tx.send(line)
            .map_err(|_| anyhow::anyhow!("claude stdin writer gone"))?;
        Ok(())
    }

    /// Cooperative interrupt when capabilities advertise it; else hard-kill.
    pub async fn interrupt(&mut self) -> anyhow::Result<()> {
        let caps = self.capabilities.lock().await.clone();
        let supports_interrupt = caps.iter().any(|c| c.starts_with("interrupt_"));
        if supports_interrupt {
            if let Some(tx) = self.stdin_tx.as_ref() {
                let req_id = format!("minos-interrupt-{}", uuid::Uuid::new_v4());
                let frame = serde_json::json!({
                    "type": "control_request",
                    "request_id": req_id,
                    "request": { "subtype": "interrupt" }
                });
                if let Ok(line) = serde_json::to_string(&frame) {
                    let _ = tx.send(line);
                    // Give CLI a short window to stop gracefully.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        self.hard_kill().await;
        Ok(())
    }

    async fn hard_kill(&mut self) {
        self.process_alive.store(false, Ordering::SeqCst);
        // Drop stdin first so CLI notices EOF.
        self.stdin_tx.take();
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
                    session_id = %self.session_id,
                    "claude did not exit after SIGTERM, sending SIGKILL"
                );
                let _ = child.kill().await;
            }
        }
    }

    pub async fn close(mut self, events_tx: &IngestSink) {
        self.hard_kill().await;

        if let Some(task) = self.stdin_task.take() {
            task.abort();
        }
        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        if let Err(error) = events_tx
            .emit(RawIngest::from_json(
                AgentName::Claude,
                self.session_id.clone(),
                serde_json::json!({
                    "kind": "thread_closed",
                    "session_id": self.session_id,
                    "reason": { "kind": "user_stopped" },
                    "closed_at_ms": chrono::Utc::now().timestamp_millis()
                }),
                chrono::Utc::now().timestamp_millis(),
            ))
            .await
        {
            warn!(
                target: "minos_agent_runtime::claude_driver",
                error = %error,
                session_id = %self.session_id,
                "failed to emit thread_closed ingest",
            );
        }

        info!(
            target: "minos_agent_runtime::claude_driver",
            session_id = %self.session_id,
            "claude control session closed"
        );
    }
}

fn build_claude_user_frame(text: &str) -> Value {
    serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": text}]
        },
        "parent_tool_use_id": null
    })
}

fn build_claude_args(
    user_text: &str,
    session_id: Option<&str>,
    resume_session_id: Option<&str>,
    mcp_config_json: Option<&str>,
    model: Option<&str>,
    extra_instructions: Option<&str>,
    bidirectional: bool,
) -> Vec<String> {
    // Always pass a non-empty `-p` so Claude print mode starts. In bidirectional
    // mode the same text is also enqueued as a stdin user frame immediately
    // after spawn — leaving stdin open-and-empty with `--input-format
    // stream-json` hangs some CLI builds with zero system/init output.
    let mut args: Vec<String> = vec![
        "-p".into(),
        user_text.into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
    ];
    if bidirectional {
        args.push("--input-format".into());
        args.push("stream-json".into());
        // Interactive permission mode (CLI: manual; SDK alias: default).
        args.push("--permission-mode".into());
        args.push("manual".into());
        // Route canUseTool over stdio control protocol when CLI accepts the flag.
        args.push("--permission-prompt-tool".into());
        args.push("stdio".into());
    }
    if let Some(m) = model.map(str::trim).filter(|s| !s.is_empty()) {
        args.push("--model".into());
        args.push(m.into());
    }
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
    let system = match extra_instructions.map(str::trim).filter(|s| !s.is_empty()) {
        Some(extra) => format!(
            "{}\n\n{}",
            crate::manager::MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS,
            extra
        ),
        None => crate::manager::MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS.to_string(),
    };
    args.push("--append-system-prompt".into());
    args.push(system);
    args
}

fn is_control_permission_request(payload: &Value) -> bool {
    let ty = payload.get("type").and_then(Value::as_str).unwrap_or("");
    if !matches!(ty, "control_request" | "sdk_control_request") {
        return false;
    }
    let request = payload
        .get("request")
        .or_else(|| payload.get("params"))
        .cloned()
        .unwrap_or(Value::Null);
    let subtype = request
        .get("subtype")
        .and_then(Value::as_str)
        .or_else(|| payload.get("subtype").and_then(Value::as_str))
        .unwrap_or("");
    matches!(
        subtype,
        "can_use_tool" | "permission" | "request_permission"
    )
}

fn extract_control_request_id(payload: &Value) -> Option<String> {
    payload
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("request")
                .and_then(|r| r.get("request_id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn extract_permission_tool_meta(payload: &Value) -> (String, Value) {
    let request = payload
        .get("request")
        .or_else(|| payload.get("params"))
        .cloned()
        .unwrap_or(Value::Null);
    let tool_name = request
        .get("tool_name")
        .or_else(|| request.get("toolName"))
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_string();
    let tool_input = request
        .get("input")
        .or_else(|| request.get("tool_input"))
        .or_else(|| request.get("toolInput"))
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::default()));
    (tool_name, tool_input)
}

async fn register_claude_permission_request(
    session_id: &str,
    payload: &Value,
    events_tx: &IngestSink,
    pending_approvals: &PendingApprovals,
) -> anyhow::Result<()> {
    let control_request_id = extract_control_request_id(payload)
        .ok_or_else(|| anyhow::anyhow!("control permission request missing request_id"))?;
    let (tool_name, tool_input) = extract_permission_tool_meta(payload);
    let is_question = tool_name == "AskUserQuestion";
    let method = if is_question {
        "claude/ask_user_question"
    } else {
        "claude/can_use_tool"
    };
    let params = serde_json::json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
        "control_request_id": control_request_id,
        "raw": payload,
    });

    events_tx
        .emit(crate::manager::approval_request_ingest(
            AgentName::Claude,
            session_id.to_string(),
            control_request_id.clone(),
            String::new(),
            method.into(),
            params,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("ingest closed: {e}"))?;

    pending_approvals.insert(
        control_request_id.clone(),
        crate::manager::PendingApproval {
            session_id: session_id.to_string(),
            target: crate::manager::PendingApprovalTarget::ClaudeControl {
                control_request_id,
                tool_input,
            },
        },
    );
    Ok(())
}

async fn sync_session_from_payload(
    payload: &Value,
    session_id: &str,
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    manager_tx: &broadcast::Sender<ManagerEvent>,
) {
    let provider_session_id = payload.get("session_id").and_then(Value::as_str);
    let should_idle = matches!(
        payload.get("type").and_then(Value::as_str),
        Some("result") | Some("error")
    );

    let maybe_transition = {
        let mut guard = sessions.lock().await;
        let Some(handle) = guard.get_mut(session_id) else {
            return;
        };

        if let Some(provider_session_id) = provider_session_id {
            handle.codex_session_id = Some(provider_session_id.to_string());
        }

        if should_idle {
            handle.set_active_turn_id(None);
            let old = handle.current_state();
            if matches!(old, SessionState::Running { .. } | SessionState::Resuming) {
                if handle.transition(SessionState::Idle).is_ok() {
                    Some((old, SessionState::Idle))
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
        let _ = manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old,
            new,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
    }
}

/// Mark process dead when stdout pump exits (called from manager if needed).
impl ClaudeControlSession {
    pub fn mark_dead(&self) {
        self.process_alive.store(false, Ordering::SeqCst);
    }

    /// Best-effort liveness: try_wait on child without consuming it permanently.
    pub fn poll_alive(&mut self) -> bool {
        if !self.process_alive.load(Ordering::SeqCst) {
            return false;
        }
        if let Some(child) = self.current_turn_child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.process_alive.store(false, Ordering::SeqCst);
                    self.stdin_tx.take();
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    self.process_alive.store(false, Ordering::SeqCst);
                    false
                }
            }
        } else {
            self.process_alive.store(false, Ordering::SeqCst);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_event::ManagerEvent;
    use crate::session_handle::SessionHandle;
    use crate::state_machine::SessionState;

    fn val(s: &str) -> Value {
        serde_json::from_str(s).expect("json fixture should parse")
    }

    #[test]
    fn claude_args_include_mcp_config_and_minos_system_prompt_append() {
        let args = build_claude_args(
            "hello",
            Some("session-1"),
            None,
            Some(r#"{"mcpServers":{"minos_teamwork":{"command":"minos-teamwork-mcp"}}}"#),
            Some("sonnet"),
            Some("Be concise."),
            true,
        );
        assert!(args.windows(2).any(|pair| pair == ["--model", "sonnet"]));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--append-system-prompt" && pair[1].contains("Be concise.")
        }));

        assert!(args.windows(2).any(|pair| pair == ["-p", "hello"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--session-id", "session-1"]));
        assert!(args
            .windows(2)
            .any(|pair| { pair[0] == "--mcp-config" && pair[1].contains(r#""minos_teamwork""#) }));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--input-format", "stream-json"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "manual"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--permission-prompt-tool", "stdio"]));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--append-system-prompt"
                && pair[1].contains("Minos teamwork mode")
                && pair[1].contains("minos_teamwork")
        }));
    }

    #[test]
    fn one_shot_args_omit_control_plane_flags() {
        let args = build_claude_args("hello", None, None, None, None, None, false);
        assert!(args.windows(2).any(|pair| pair == ["-p", "hello"]));
        assert!(!args.iter().any(|a| a == "--input-format"));
        assert!(!args.iter().any(|a| a == "--permission-mode"));
    }

    #[test]
    fn user_frame_carries_text_content() {
        let frame = build_claude_user_frame("你好");
        assert_eq!(frame["type"], "user");
        assert_eq!(frame["message"]["content"][0]["text"], "你好");
        assert!(frame["parent_tool_use_id"].is_null());
    }

    #[test]
    fn detects_control_permission_shapes() {
        let a = val(
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}"#,
        );
        assert!(is_control_permission_request(&a));
        assert_eq!(extract_control_request_id(&a).as_deref(), Some("r1"));
        let (name, input) = extract_permission_tool_meta(&a);
        assert_eq!(name, "Bash");
        assert_eq!(input["command"], "ls");

        let b = val(
            r#"{"type":"sdk_control_request","request":{"subtype":"permission","request_id":"p1","tool_name":"Write","tool_input":{"file_path":"/t"}}}"#,
        );
        assert!(is_control_permission_request(&b));
        assert_eq!(extract_control_request_id(&b).as_deref(), Some("p1"));
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
        let session_id = "thr_claude".to_string();
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        sessions.lock().await.insert(
            session_id.clone(),
            SessionHandle::new(
                session_id.clone(),
                "/tmp".into(),
                AgentName::Claude,
                SessionState::Running {
                    turn_started_at_ms: 1,
                },
                0,
            ),
        );
        let (manager_tx, mut manager_rx) = broadcast::channel::<ManagerEvent>(8);

        sync_session_from_payload(
            &val(r#"{"type":"result","session_id":"sess_resume","is_error":false}"#),
            &session_id,
            &sessions,
            &manager_tx,
        )
        .await;

        let guard = sessions.lock().await;
        let handle = guard.get(&session_id).expect("thread should exist");
        assert!(matches!(handle.current_state(), SessionState::Idle));
        assert_eq!(handle.codex_session_id.as_deref(), Some("sess_resume"));
        drop(guard);

        let event = manager_rx
            .recv()
            .await
            .expect("manager event should be emitted");
        assert!(matches!(
            event,
            ManagerEvent::SessionStateChanged { session_id, new: SessionState::Idle, .. }
                if session_id == "thr_claude"
        ));
    }
}
