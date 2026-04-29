use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use minos_domain::{AgentName, MinosError};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::runtime::RawIngest;

const STRIP_AUTH_STORE_ENV_KEYS: &[&str] = &[
    "CODEX_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
];
const STDERR_TAIL_LINES: usize = 16;

pub(crate) fn synthetic_thread_id() -> String {
    format!("thr-exec-{}", Uuid::new_v4())
}

pub(crate) fn emit_thread_started(
    ingest_tx: &broadcast::Sender<RawIngest>,
    agent: AgentName,
    thread_id: &str,
    opened_at_ms: i64,
) {
    let _ = ingest_tx.send(RawIngest {
        agent,
        thread_id: thread_id.to_string(),
        payload: json!({
            "method": "thread/started",
            "params": {
                "threadId": thread_id,
                "createdAtMs": opened_at_ms,
            },
        }),
        ts_ms: opened_at_ms,
    });
}

pub(crate) struct ExecTurnRequest<'a> {
    pub(crate) bin: &'a Path,
    pub(crate) workspace_root: &'a Path,
    pub(crate) subprocess_env: Arc<HashMap<String, String>>,
    pub(crate) agent: AgentName,
    pub(crate) thread_id: String,
    pub(crate) prompt: String,
    pub(crate) codex_session_id: Arc<Mutex<Option<String>>>,
    pub(crate) ingest_tx: broadcast::Sender<RawIngest>,
}

pub(crate) async fn spawn_exec_turn(
    req: ExecTurnRequest<'_>,
) -> Result<JoinHandle<()>, MinosError> {
    let ExecTurnRequest {
        bin,
        workspace_root,
        subprocess_env,
        agent,
        thread_id,
        prompt,
        codex_session_id,
        ingest_tx,
    } = req;
    let cwd = workspace_root.canonicalize().map_or_else(
        |_| workspace_root.display().to_string(),
        |path| path.display().to_string(),
    );
    let existing_session_id = codex_session_id.lock().await.clone();
    let args = build_exec_args(existing_session_id.as_deref(), &cwd, &prompt);
    let (child, stdout, stderr) = spawn_exec_child(bin, &args, &subprocess_env)?;

    info!(
        bin = %bin.display(),
        thread_id,
        resumed = existing_session_id.is_some(),
        "spawned codex exec JSONL turn",
    );
    emit_user_prompt(&ingest_tx, agent, &thread_id, &prompt);

    Ok(tokio::spawn(run_exec_turn(ExecTurnTask {
        child,
        stdout,
        stderr,
        agent,
        thread_id,
        codex_session_id,
        ingest_tx,
    })))
}

fn spawn_exec_child(
    bin: &Path,
    args: &[String],
    subprocess_env: &Arc<HashMap<String, String>>,
) -> Result<(Child, ChildStdout, ChildStderr), MinosError> {
    let exec_env = codex_exec_env(subprocess_env);
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env_clear()
        .envs(exec_env.iter())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| MinosError::CodexSpawnFailed {
        message: format!("spawn {}: {e}", bin.display()),
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MinosError::CodexSpawnFailed {
            message: format!("{} stdout pipe unavailable", bin.display()),
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| MinosError::CodexSpawnFailed {
            message: format!("{} stderr pipe unavailable", bin.display()),
        })?;

    Ok((child, stdout, stderr))
}

fn codex_exec_env(subprocess_env: &Arc<HashMap<String, String>>) -> HashMap<String, String> {
    subprocess_env
        .iter()
        .filter(|(key, _)| !STRIP_AUTH_STORE_ENV_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

struct ExecTurnTask {
    child: Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    agent: AgentName,
    thread_id: String,
    codex_session_id: Arc<Mutex<Option<String>>>,
    ingest_tx: broadcast::Sender<RawIngest>,
}

async fn run_exec_turn(task: ExecTurnTask) {
    let ExecTurnTask {
        mut child,
        stdout,
        stderr,
        agent,
        thread_id,
        codex_session_id,
        ingest_tx,
    } = task;
    let stderr_tail = Arc::new(StdMutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
    let stderr_tail_task = Arc::clone(&stderr_tail);
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    push_stderr_line(&stderr_tail_task, line.clone());
                    debug!(target: "minos_agent_runtime::exec_jsonl", line = %line, "codex exec stderr");
                }
                Ok(None) => break,
                Err(e) => {
                    warn!(
                        target: "minos_agent_runtime::exec_jsonl",
                        error = %e,
                        "codex exec stderr read error",
                    );
                    break;
                }
            }
        }
    });

    let mut normalizer = ExecJsonlNormalizer::default();
    let mut stdout_lines = BufReader::new(stdout).lines();
    loop {
        match stdout_lines.next_line().await {
            Ok(Some(line)) => {
                handle_stdout_line(
                    &line,
                    &mut normalizer,
                    &thread_id,
                    agent,
                    &codex_session_id,
                    &ingest_tx,
                )
                .await;
            }
            Ok(None) => break,
            Err(e) => {
                emit_error(
                    &ingest_tx,
                    agent,
                    &thread_id,
                    "exec_stdout_read_failed",
                    &format!("failed reading codex exec JSONL: {e}"),
                );
                break;
            }
        }
    }

    let wait_result = child.wait().await;
    let _ = stderr_task.await;
    let stderr_summary = stderr_tail_summary(&stderr_tail);

    match wait_result {
        Ok(status) if status.success() => {}
        Ok(status) => {
            let exit_message = format_exit_message(status, stderr_summary.as_deref());
            emit_error(
                &ingest_tx,
                agent,
                &thread_id,
                "exec_exit_nonzero",
                &exit_message,
            );
        }
        Err(e) => {
            emit_error(
                &ingest_tx,
                agent,
                &thread_id,
                "exec_wait_failed",
                &format!("failed waiting for codex exec: {e}"),
            );
        }
    }
}

fn push_stderr_line(stderr_tail: &Arc<StdMutex<VecDeque<String>>>, line: String) {
    let mut guard = stderr_tail.lock().unwrap();
    if guard.len() == STDERR_TAIL_LINES {
        guard.pop_front();
    }
    guard.push_back(line);
}

fn stderr_tail_summary(stderr_tail: &Arc<StdMutex<VecDeque<String>>>) -> Option<String> {
    let guard = stderr_tail.lock().unwrap();
    if guard.is_empty() {
        None
    } else {
        Some(guard.iter().cloned().collect::<Vec<_>>().join(" | "))
    }
}

fn format_exit_message(status: std::process::ExitStatus, stderr_tail: Option<&str>) -> String {
    match stderr_tail {
        Some(stderr_tail) => format!("codex exec exited with {status}: {stderr_tail}"),
        None => format!("codex exec exited with {status}"),
    }
}

fn emit_user_prompt(
    ingest_tx: &broadcast::Sender<RawIngest>,
    agent: AgentName,
    thread_id: &str,
    prompt: &str,
) {
    let started_at_ms = current_unix_ms();
    let user_item_id = Uuid::new_v4().to_string();
    let payloads = [
        json!({
            "method": "item/started",
            "params": {
                "itemId": user_item_id,
                "role": "user",
                "startedAtMs": started_at_ms,
                "text": prompt,
            },
        }),
        json!({
            "method": "item/userMessage/delta",
            "params": {
                "itemId": user_item_id,
                "delta": prompt,
            },
        }),
    ];

    for payload in payloads {
        let _ = ingest_tx.send(RawIngest {
            agent,
            thread_id: thread_id.to_string(),
            payload,
            ts_ms: started_at_ms,
        });
    }
}

async fn handle_stdout_line(
    line: &str,
    normalizer: &mut ExecJsonlNormalizer,
    thread_id: &str,
    agent: AgentName,
    codex_session_id: &Arc<Mutex<Option<String>>>,
    ingest_tx: &broadcast::Sender<RawIngest>,
) {
    let Ok(entry) = serde_json::from_str::<Value>(line) else {
        warn!(target: "minos_agent_runtime::exec_jsonl", raw = %line, "ignoring malformed codex exec JSONL line");
        return;
    };

    if let Some(session_id) = session_id_from_entry(&entry) {
        let mut guard = codex_session_id.lock().await;
        if guard.is_none() {
            *guard = Some(session_id);
        }
    }

    for payload in normalizer.normalize(&entry) {
        log_user_visible_payload(thread_id, &payload);
        let _ = ingest_tx.send(RawIngest {
            agent,
            thread_id: thread_id.to_string(),
            payload,
            ts_ms: current_unix_ms(),
        });
    }
}

fn session_id_from_entry(entry: &Value) -> Option<String> {
    match entry.get("type").and_then(Value::as_str) {
        Some("session_meta") => entry
            .get("payload")
            .and_then(|payload| payload.get("id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        Some("session_configured") => entry
            .get("session_id")
            .or_else(|| entry.get("sessionId"))
            .or_else(|| entry.get("thread_id"))
            .or_else(|| entry.get("threadId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        _ => None,
    }
}

fn log_user_visible_payload(thread_id: &str, payload: &Value) {
    let Some(method) = payload.get("method").and_then(Value::as_str) else {
        return;
    };
    match method {
        "item/agentMessage/delta" => {
            let text = payload
                .pointer("/params/delta")
                .and_then(Value::as_str)
                .unwrap_or("");
            if text.is_empty() {
                return;
            }
            info!(
                target: "minos_agent_runtime::exec_jsonl",
                thread_id,
                text = %text,
                "codex agent message",
            );
        }
        "turn/completed" => {
            info!(
                target: "minos_agent_runtime::exec_jsonl",
                thread_id,
                "codex turn completed",
            );
        }
        _ => {}
    }
}

fn emit_error(
    ingest_tx: &broadcast::Sender<RawIngest>,
    agent: AgentName,
    thread_id: &str,
    code: &str,
    message: &str,
) {
    let _ = ingest_tx.send(RawIngest {
        agent,
        thread_id: thread_id.to_string(),
        payload: json!({
            "method": "error",
            "params": {
                "code": code,
                "message": message,
            },
        }),
        ts_ms: current_unix_ms(),
    });
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn build_exec_args(existing_session_id: Option<&str>, cwd: &str, prompt: &str) -> Vec<String> {
    let sandbox_arg =
        format!("sandbox_permissions=['disk-full-read-access','disk-write-folder={cwd}']");
    let mut args = vec!["exec".to_string()];
    if let Some(session_id) = existing_session_id {
        args.push("resume".to_string());
        args.extend([
            "--json".to_string(),
            "-C".to_string(),
            cwd.to_string(),
            "--skip-git-repo-check".to_string(),
            "-c".to_string(),
            "approval_policy=never".to_string(),
            "-c".to_string(),
            sandbox_arg,
            "-c".to_string(),
            "shell_environment_policy.inherit=all".to_string(),
            session_id.to_string(),
            prompt.to_string(),
        ]);
    } else {
        args.extend([
            "--json".to_string(),
            "-C".to_string(),
            cwd.to_string(),
            "--skip-git-repo-check".to_string(),
            "-c".to_string(),
            "approval_policy=never".to_string(),
            "-c".to_string(),
            sandbox_arg,
            "-c".to_string(),
            "shell_environment_policy.inherit=all".to_string(),
            prompt.to_string(),
        ]);
    }
    args
}

#[derive(Default)]
struct ExecJsonlNormalizer {
    assistant_item_id: Option<String>,
    active_turn_id: Option<String>,
    assistant_text_seen: bool,
}

impl ExecJsonlNormalizer {
    fn normalize(&mut self, entry: &Value) -> Vec<Value> {
        let kind = entry.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "event_msg" => self.normalize_event_msg(entry.get("payload").unwrap_or(&Value::Null)),
            "response_item" | "raw_response_item" => {
                self.normalize_response_item(response_item_payload(entry))
            }
            "task_started"
            | "turn_started"
            | "task_complete"
            | "turn_complete"
            | "agent_message"
            | "agent_message_delta"
            | "agent_message_content_delta"
            | "agent_reasoning"
            | "agent_reasoning_delta"
            | "agent_reasoning_raw_content"
            | "agent_reasoning_raw_content_delta"
            | "reasoning_content_delta"
            | "reasoning_raw_content_delta" => self.normalize_event_msg(entry),
            _ => Vec::new(),
        }
    }

    #[allow(clippy::too_many_lines)] // Single-site dispatch over Codex event_msg variants.
    fn normalize_event_msg(&mut self, payload: &Value) -> Vec<Value> {
        let event_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "task_started" | "turn_started" => {
                self.active_turn_id = read_string(
                    payload
                        .get("turn_id")
                        .or_else(|| payload.get("turnId"))
                        .and_then(Value::as_str),
                );
                self.assistant_item_id = None;
                self.assistant_text_seen = false;
                self.ensure_assistant_started()
            }
            "task_complete" | "turn_complete" => {
                let turn_id = read_string(
                    payload
                        .get("turn_id")
                        .or_else(|| payload.get("turnId"))
                        .and_then(Value::as_str),
                )
                .or_else(|| self.active_turn_id.clone())
                .unwrap_or_default();
                self.assistant_item_id = None;
                self.active_turn_id = None;
                self.assistant_text_seen = false;
                vec![json!({
                    "method": "turn/completed",
                    "params": {
                        "turnId": turn_id,
                        "id": turn_id,
                        "finishedAtMs": current_unix_ms(),
                    },
                })]
            }
            "agent_message" => {
                let text = read_string(
                    payload
                        .get("message")
                        .or_else(|| payload.get("text"))
                        .and_then(Value::as_str),
                )
                .unwrap_or_default();
                if text.is_empty() || is_commentary_phase(payload) {
                    return Vec::new();
                }
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/agentMessage/delta",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "delta": text,
                    },
                }));
                self.assistant_text_seen = true;
                out
            }
            "agent_message_delta" | "agent_message_content_delta" => {
                let Some(text) = read_preserved_string(
                    payload
                        .get("delta")
                        .or_else(|| payload.get("text"))
                        .or_else(|| payload.get("message"))
                        .or_else(|| payload.get("content"))
                        .and_then(Value::as_str),
                ) else {
                    return Vec::new();
                };
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/agentMessage/delta",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "delta": text,
                    },
                }));
                self.assistant_text_seen = true;
                out
            }
            "agent_reasoning" => {
                let text = read_string(
                    payload
                        .get("message")
                        .or_else(|| payload.get("text"))
                        .or_else(|| payload.get("summary"))
                        .and_then(Value::as_str),
                )
                .unwrap_or_default();
                if text.is_empty() {
                    return Vec::new();
                }
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/reasoning/delta",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "delta": text,
                    },
                }));
                out
            }
            "agent_reasoning_delta"
            | "agent_reasoning_raw_content_delta"
            | "reasoning_content_delta"
            | "reasoning_raw_content_delta" => {
                let Some(text) = read_preserved_string(
                    payload
                        .get("delta")
                        .or_else(|| payload.get("text"))
                        .or_else(|| payload.get("message"))
                        .or_else(|| payload.get("content"))
                        .and_then(Value::as_str),
                ) else {
                    return Vec::new();
                };
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/reasoning/delta",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "delta": text,
                    },
                }));
                out
            }
            "agent_reasoning_raw_content" => {
                let text = read_string(
                    payload
                        .get("content")
                        .or_else(|| payload.get("text"))
                        .or_else(|| payload.get("message"))
                        .and_then(Value::as_str),
                )
                .unwrap_or_default();
                if text.is_empty() {
                    return Vec::new();
                }
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/reasoning/delta",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "delta": text,
                    },
                }));
                out
            }
            _ => Vec::new(),
        }
    }

    #[allow(clippy::too_many_lines)] // Single-site dispatch over Codex response_item variants.
    fn normalize_response_item(&mut self, payload: &Value) -> Vec<Value> {
        let item_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
        match item_type {
            "message" => {
                let role = payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("assistant");
                if role != "assistant" || self.assistant_text_seen {
                    return Vec::new();
                }
                let text = extract_message_text(payload);
                if text.is_empty() {
                    return Vec::new();
                }
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/agentMessage/delta",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "delta": text,
                    },
                }));
                self.assistant_text_seen = true;
                out
            }
            "reasoning" => {
                let text = extract_reasoning_text(payload);
                if text.is_empty() {
                    return Vec::new();
                }
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/reasoning/delta",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "delta": text,
                    },
                }));
                out
            }
            "function_call" => {
                let call_id = read_string(
                    payload
                        .get("call_id")
                        .or_else(|| payload.get("callId"))
                        .and_then(Value::as_str),
                )
                .unwrap_or_else(|| Uuid::new_v4().to_string());
                let name = read_string(payload.get("name").and_then(Value::as_str))
                    .unwrap_or_else(|| "tool".to_string());
                let args_json = read_string(payload.get("arguments").and_then(Value::as_str))
                    .unwrap_or_else(|| "{}".to_string());
                let mut out = self.ensure_assistant_started();
                out.push(json!({
                    "method": "item/toolCall/started",
                    "params": {
                        "itemId": self.assistant_item_id.clone().unwrap_or_default(),
                        "toolCallId": call_id,
                        "name": name,
                    },
                }));
                out.push(json!({
                    "method": "item/toolCall/arguments",
                    "params": {
                        "toolCallId": call_id,
                        "argumentsDelta": args_json,
                    },
                }));
                out.push(json!({
                    "method": "item/toolCall/argumentsCompleted",
                    "params": {
                        "toolCallId": call_id,
                    },
                }));
                out
            }
            "function_call_output" => {
                let call_id = read_string(
                    payload
                        .get("call_id")
                        .or_else(|| payload.get("callId"))
                        .and_then(Value::as_str),
                )
                .unwrap_or_default();
                if call_id.is_empty() {
                    return Vec::new();
                }
                vec![json!({
                    "method": "item/toolCall/completed",
                    "params": {
                        "toolCallId": call_id,
                        "output": read_string(payload.get("output").and_then(Value::as_str)).unwrap_or_default(),
                        "isError": payload
                            .get("is_error")
                            .or_else(|| payload.get("isError"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                })]
            }
            _ => Vec::new(),
        }
    }

    fn ensure_assistant_started(&mut self) -> Vec<Value> {
        if self.assistant_item_id.is_some() {
            return Vec::new();
        }
        let item_id = Uuid::new_v4().to_string();
        self.assistant_item_id = Some(item_id.clone());
        vec![json!({
            "method": "item/started",
            "params": {
                "itemId": item_id,
                "role": "assistant",
                "startedAtMs": current_unix_ms(),
            },
        })]
    }
}

fn response_item_payload(entry: &Value) -> &Value {
    entry
        .get("payload")
        .and_then(|payload| {
            payload
                .get("item")
                .or_else(|| payload.get("response_item"))
                .or(Some(payload))
        })
        .or_else(|| entry.get("item"))
        .unwrap_or(entry)
}

fn extract_message_text(payload: &Value) -> String {
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(text) = payload.get("message").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(text) = payload.get("content").and_then(Value::as_str) {
        return text.to_string();
    }
    payload
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("output_text" | "text") => part.get("text").and_then(Value::as_str),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn extract_reasoning_text(payload: &Value) -> String {
    let summary = payload
        .get("summary")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    read_string(
                        part.get("text")
                            .or_else(|| part.get("summary"))
                            .and_then(Value::as_str),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    first_non_empty(&[
        summary,
        read_string(payload.get("text").and_then(Value::as_str)).unwrap_or_default(),
        read_string(payload.get("content").and_then(Value::as_str)).unwrap_or_default(),
    ])
}

fn is_commentary_phase(payload: &Value) -> bool {
    payload
        .get("phase")
        .and_then(Value::as_str)
        .is_some_and(|phase| phase.eq_ignore_ascii_case("commentary"))
}

fn read_string(value: Option<&str>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn read_preserved_string(value: Option<&str>) -> Option<String> {
    value.and_then(|text| {
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    })
}

fn first_non_empty(values: &[String]) -> String {
    values
        .iter()
        .find_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_exec_args_for_first_turn_uses_exec_jsonl() {
        let args = build_exec_args(None, "/tmp/work", "hello world");
        assert_eq!(args[0], "exec");
        assert!(args.iter().any(|arg| arg == "--json"));
        assert!(args.iter().any(|arg| arg == "--skip-git-repo-check"));
        assert_eq!(args.last().map(String::as_str), Some("hello world"));
    }

    #[test]
    fn build_exec_args_for_follow_up_uses_resume() {
        let args = build_exec_args(Some("ses_123"), "/tmp/work", "next turn");
        assert!(args.starts_with(&[
            "exec".to_string(),
            "resume".to_string(),
            "--json".to_string(),
        ]));
        assert!(args.iter().any(|arg| arg == "ses_123"));
        assert_eq!(args.last().map(String::as_str), Some("next turn"));
    }

    #[test]
    fn codex_exec_env_strips_auth_store_overrides() {
        let env = Arc::new(HashMap::from([
            ("HOME".to_string(), "/Users/fan".to_string()),
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-test-123".to_string()),
            (
                "CODEX_HOME".to_string(),
                "/Users/fan/custom-codex".to_string(),
            ),
            (
                "XDG_CONFIG_HOME".to_string(),
                "/Users/fan/.config".to_string(),
            ),
            (
                "XDG_DATA_HOME".to_string(),
                "/Users/fan/.local/share".to_string(),
            ),
        ]));

        let filtered = codex_exec_env(&env);

        assert_eq!(filtered.get("HOME").map(String::as_str), Some("/Users/fan"));
        assert_eq!(
            filtered.get("PATH").map(String::as_str),
            Some("/usr/bin:/bin")
        );
        assert_eq!(
            filtered.get("OPENAI_API_KEY").map(String::as_str),
            Some("sk-test-123")
        );
        assert!(!filtered.contains_key("CODEX_HOME"));
        assert!(!filtered.contains_key("XDG_CONFIG_HOME"));
        assert!(!filtered.contains_key("XDG_DATA_HOME"));
    }

    #[test]
    fn format_exit_message_appends_stderr_tail_when_present() {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 7")
            .status()
            .expect("shell exits with status");

        let message = format_exit_message(status, Some("invalid_grant"));

        assert!(message.contains("exit status: 7"), "{message}");
        assert!(message.contains("invalid_grant"), "{message}");
    }

    #[test]
    fn normalizer_maps_exec_turn_entries_to_codex_like_payloads() {
        let mut normalizer = ExecJsonlNormalizer::default();

        let started = normalizer.normalize(&json!({
            "type": "event_msg",
            "payload": {"type": "task_started", "turn_id": "turn-1"}
        }));
        assert_eq!(started.len(), 1);
        assert_eq!(started[0]["method"], "item/started");
        assert_eq!(started[0]["params"]["role"], "assistant");

        let delta = normalizer.normalize(&json!({
            "type": "event_msg",
            "payload": {"type": "agent_message", "text": "hello"}
        }));
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0]["method"], "item/agentMessage/delta");
        assert_eq!(delta[0]["params"]["delta"], "hello");

        let tool = normalizer.normalize(&json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "call_id": "call-1",
                "name": "exec_command",
                "arguments": "{\"cmd\":\"ls\"}"
            }
        }));
        assert_eq!(tool.len(), 3);
        assert_eq!(tool[0]["method"], "item/toolCall/started");
        assert_eq!(tool[1]["method"], "item/toolCall/arguments");
        assert_eq!(tool[2]["method"], "item/toolCall/argumentsCompleted");

        let completed = normalizer.normalize(&json!({
            "type": "event_msg",
            "payload": {"type": "task_complete", "turn_id": "turn-1"}
        }));
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0]["method"], "turn/completed");
        assert_eq!(completed[0]["params"]["turnId"], "turn-1");
    }

    #[test]
    fn normalizer_maps_current_cli_delta_events() {
        let mut normalizer = ExecJsonlNormalizer::default();

        let started = normalizer.normalize(&json!({
            "type": "turn_started",
            "turn_id": "turn-new",
        }));
        assert_eq!(started[0]["method"], "item/started");

        let first = normalizer.normalize(&json!({
            "type": "agent_message_delta",
            "delta": "Hello ",
        }));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["method"], "item/agentMessage/delta");
        assert_eq!(first[0]["params"]["delta"], "Hello ");

        let second = normalizer.normalize(&json!({
            "type": "agent_message_content_delta",
            "delta": "world",
            "content_index": 0,
        }));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0]["params"]["delta"], "world");

        let completed = normalizer.normalize(&json!({
            "type": "turn_complete",
            "turn_id": "turn-new",
        }));
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0]["method"], "turn/completed");
        assert_eq!(completed[0]["params"]["turnId"], "turn-new");
    }

    #[test]
    fn normalizer_uses_raw_response_message_when_no_deltas_arrived() {
        let mut normalizer = ExecJsonlNormalizer::default();
        let _ = normalizer.normalize(&json!({
            "type": "task_started",
            "turn_id": "turn-raw",
        }));

        let out = normalizer.normalize(&json!({
            "type": "raw_response_item",
            "payload": {
                "item": {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "raw "},
                        {"type": "output_text", "text": "text"}
                    ]
                }
            }
        }));

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], "item/agentMessage/delta");
        assert_eq!(out[0]["params"]["delta"], "raw text");
    }

    #[test]
    fn normalizer_does_not_duplicate_raw_response_message_after_deltas() {
        let mut normalizer = ExecJsonlNormalizer::default();
        let _ = normalizer.normalize(&json!({
            "type": "task_started",
            "turn_id": "turn-dup",
        }));
        let _ = normalizer.normalize(&json!({
            "type": "agent_message_delta",
            "delta": "streamed",
        }));

        let out = normalizer.normalize(&json!({
            "type": "raw_response_item",
            "payload": {
                "item": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "streamed"}]
                }
            }
        }));

        assert!(out.is_empty());
    }

    #[test]
    fn session_configured_supplies_resume_id() {
        let entry = json!({
            "type": "session_configured",
            "session_id": "00000000-0000-0000-0000-000000000123",
        });
        assert_eq!(
            session_id_from_entry(&entry),
            Some("00000000-0000-0000-0000-000000000123".to_string())
        );
    }
}
