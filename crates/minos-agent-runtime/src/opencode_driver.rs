#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use minos_domain::AgentName;
use reqwest::Client;
use serde_json::Value;
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::RawIngest;
use crate::manager::IngestSink;
use crate::manager_event::ManagerEvent;
use crate::state_machine::SessionState;
use crate::session_handle::SessionHandle;

const KILL_ESCALATION: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
struct PendingTaskTool {
    parent_session_id: String,
    tool_call_id: String,
    prompt: Option<String>,
}

pub struct OpencodeServerConfig {
    pub opencode_bin: PathBuf,
    pub port: u16,
    pub password: String,
    pub subprocess_env: Arc<HashMap<String, String>>,
    pub opencode_config_content: Option<String>,
}

pub struct OpencodeServerInstance {
    pub workspace: PathBuf,
    pub config: OpencodeServerConfig,
    pub child: Option<Child>,
    pub http_client: Client,
    pub base_url: String,
    pub auth_header: String,
}

impl OpencodeServerInstance {
    pub async fn spawn(workspace: &Path, config: OpencodeServerConfig) -> anyhow::Result<Self> {
        let port = config.port;
        let mut cmd = Command::new(&config.opencode_bin);
        cmd.args(["serve", "--port", &port.to_string()])
            .current_dir(workspace)
            .env_clear()
            .envs(config.subprocess_env.iter())
            .env("OPENCODE_SERVER_PASSWORD", &config.password)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(content) = config.opencode_config_content.as_deref().filter(|_| {
            !config
                .subprocess_env
                .contains_key("OPENCODE_CONFIG_CONTENT")
        }) {
            cmd.env("OPENCODE_CONFIG_CONTENT", content);
        }

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

        let stderr = child.stderr.take();
        if let Some(err_stream) = stderr {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let reader = tokio::io::BufReader::new(err_stream);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(
                        target: "minos_agent_runtime::opencode_driver",
                        line = %line,
                        "opencode stderr"
                    );
                }
            });
        }

        let base_url = format!("http://127.0.0.1:{port}");
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD
                .encode(format!("opencode:{}", config.password))
        );

        let http_client = Client::builder().timeout(Duration::from_secs(30)).build()?;

        let instance = Self {
            workspace: workspace.to_path_buf(),
            config,
            child: Some(child),
            http_client,
            base_url: base_url.clone(),
            auth_header: auth_header.clone(),
        };

        let health_url = format!("{base_url}/global/health");
        for _ in 0..30 {
            if instance
                .http_client
                .get(&health_url)
                .header("Authorization", &auth_header)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                info!(
                    target: "minos_agent_runtime::opencode_driver",
                    port,
                    workspace = %workspace.display(),
                    "opencode server ready"
                );
                return Ok(instance);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Err(anyhow::anyhow!(
            "opencode server did not become healthy within timeout"
        ))
    }

    pub async fn create_session(&mut self) -> anyhow::Result<String> {
        self.create_session_with_model(None).await
    }

    pub async fn create_session_with_model(
        &mut self,
        model: Option<&str>,
    ) -> anyhow::Result<String> {
        let body = match model.map(str::trim).filter(|s| !s.is_empty()) {
            Some(m) => serde_json::json!({ "model": m }).to_string(),
            None => "{}".to_string(),
        };
        let resp = self
            .http_client
            .post(format!("{}/session", self.base_url))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?
            .error_for_status()?;
        let body: Value = resp.json().await?;
        body.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("opencode create_session: missing id in response"))
    }

    pub async fn send_prompt(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "parts": [{ "type": "text", "text": text }]
        });
        self.http_client
            .post(format!(
                "{}/session/{session_id}/prompt_async",
                self.base_url
            ))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload)?)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn abort_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.http_client
            .post(format!("{}/session/{session_id}/abort", self.base_url))
            .header("Authorization", &self.auth_header)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn respond_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> anyhow::Result<()> {
        let payload = serde_json::json!({ "response": response });
        self.http_client
            .post(format!(
                "{}/session/{session_id}/permissions/{permission_id}",
                self.base_url
            ))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload)?)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn respond_question(
        &self,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> anyhow::Result<()> {
        let payload = serde_json::json!({ "answers": answers });
        self.http_client
            .post(format!("{}/question/{question_id}/reply", self.base_url))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload)?)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub fn subscribe_sse_url(&self) -> String {
        format!("{}/event", self.base_url)
    }

    pub fn auth_header(&self) -> &str {
        &self.auth_header
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub async fn close(mut self) {
        if let Some(mut child) = self.child.take() {
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
                    target: "minos_agent_runtime::opencode_driver",
                    "opencode did not exit after SIGTERM, sending SIGKILL"
                );
                let _ = child.kill().await;
            }
        }

        info!(
            target: "minos_agent_runtime::opencode_driver",
            workspace = %self.workspace.display(),
            "opencode server instance closed"
        );
    }
}

pub fn spawn_sse_pump(
    sse_url: String,
    auth_header: String,
    session_map: Arc<Mutex<HashMap<String, String>>>,
    sessions: Arc<Mutex<HashMap<String, SessionHandle>>>,
    manager_tx: broadcast::Sender<ManagerEvent>,
    events_tx: IngestSink,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let pending_tasks = Arc::new(Mutex::new(HashMap::<String, PendingTaskTool>::new()));
        loop {
            match try_sse_connect(
                &sse_url,
                &auth_header,
                &session_map,
                &sessions,
                &manager_tx,
                &events_tx,
                &pending_tasks,
            )
            .await
            {
                Ok(()) => {
                    info!(
                        target: "minos_agent_runtime::opencode_driver",
                        "SSE stream ended, reconnecting"
                    );
                }
                Err(e) => {
                    warn!(
                        target: "minos_agent_runtime::opencode_driver",
                        error = %e,
                        "SSE connection error, reconnecting"
                    );
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

async fn try_sse_connect(
    sse_url: &str,
    auth_header: &str,
    session_map: &Arc<Mutex<HashMap<String, String>>>,
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    manager_tx: &broadcast::Sender<ManagerEvent>,
    events_tx: &IngestSink,
    pending_tasks: &Arc<Mutex<HashMap<String, PendingTaskTool>>>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    info!(
        target: "minos_agent_runtime::opencode_driver",
        sse_url,
        "connecting to opencode SSE"
    );
    let response = client
        .get(sse_url)
        .header("Authorization", auth_header)
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("SSE endpoint returned {}", response.status());
    }

    let mut stream = response.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(sse_event) => {
                let payload: Value = match serde_json::from_str(&sse_event.data) {
                    Ok(v) => v,
                    Err(_) => serde_json::json!({
                        "kind": "raw",
                        "raw_kind": "sse",
                        "data": sse_event.data
                    }),
                };
                let (session_id, _provider_session_id) =
                    match resolve_session_id(&payload, session_map).await {
                    Some(resolved) => resolved,
                    None => {
                        let Some(session_id) = register_opencode_subagent_from_pending_task(
                            &payload,
                            session_map,
                            sessions,
                            manager_tx,
                            events_tx,
                            pending_tasks,
                        )
                        .await
                        else {
                            tracing::debug!(
                                target: "minos_agent_runtime::opencode_driver",
                                payload = %payload,
                                "dropping opencode event without a resolved Minos thread"
                            );
                            continue;
                        };
                        let provider_session_id =
                            extract_session_id(&payload).unwrap_or("").to_string();
                        (session_id, provider_session_id)
                    }
                };
                if let Err(error) = events_tx
                    .emit(RawIngest::from_json(
                        AgentName::Opencode,
                        session_id.clone(),
                        payload.clone(),
                        chrono::Utc::now().timestamp_millis(),
                    ))
                    .await
                {
                    warn!(
                        target: "minos_agent_runtime::opencode_driver",
                        error = %error,
                        session_id = %session_id,
                        "durable ingest sink closed while reading opencode SSE",
                    );
                    break;
                }
                sync_session_state(&payload, &session_id, sessions, manager_tx).await;
                update_pending_task_tool(&payload, &session_id, pending_tasks).await;
            }
            Err(e) => {
                warn!(
                    target: "minos_agent_runtime::opencode_driver",
                    error = %e,
                    "SSE parse error"
                );
                break;
            }
        }
    }
    Ok(())
}

async fn resolve_session_id(
    payload: &Value,
    session_map: &Arc<Mutex<HashMap<String, String>>>,
) -> Option<(String, String)> {
    let provider_session_id = extract_session_id(payload)?.to_string();
    let map = session_map.lock().await;
    map.iter().find_map(|(minos_session_id, mapped_provider_id)| {
        (mapped_provider_id == &provider_session_id)
            .then(|| (minos_session_id.clone(), provider_session_id.clone()))
    })
}

async fn register_opencode_subagent_from_pending_task(
    payload: &Value,
    session_map: &Arc<Mutex<HashMap<String, String>>>,
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    manager_tx: &broadcast::Sender<ManagerEvent>,
    events_tx: &IngestSink,
    pending_tasks: &Arc<Mutex<HashMap<String, PendingTaskTool>>>,
) -> Option<String> {
    let session_id = extract_session_id(payload)?.to_string();

    // Already registered (same provider id) — never emit a second SessionAdded.
    {
        let map = session_map.lock().await;
        if let Some((minos_id, _)) = map
            .iter()
            .find(|(_, provider_id)| *provider_id == &session_id)
        {
            return Some(minos_id.clone());
        }
        if map.contains_key(&session_id) {
            return Some(session_id.clone());
        }
    }
    {
        let guard = sessions.lock().await;
        if guard.contains_key(&session_id) {
            session_map
                .lock()
                .await
                .insert(session_id.clone(), session_id.clone());
            return Some(session_id);
        }
    }

    let pending = {
        let guard = pending_tasks.lock().await;
        (guard.len() == 1)
            .then(|| guard.values().next().cloned())
            .flatten()
    }?;

    let workspace = {
        let mut guard = sessions.lock().await;
        if guard.contains_key(&session_id) {
            drop(guard);
            session_map
                .lock()
                .await
                .insert(session_id.clone(), session_id.clone());
            return Some(session_id);
        }
        let workspace = guard.get(&pending.parent_session_id)?.workspace.clone();
        guard.insert(
            session_id.clone(),
            SessionHandle::new_subagent(
                session_id.clone(),
                workspace.clone(),
                AgentName::Opencode,
                pending.parent_session_id.clone(),
                Some(session_id.clone()),
                SessionState::Idle,
                0,
            ),
        );
        workspace
    };
    session_map
        .lock()
        .await
        .insert(session_id.clone(), session_id.clone());
    let parent_session_id = pending.parent_session_id.clone();
    let tool_call_id = pending.tool_call_id.clone();
    let prompt = pending.prompt.clone();
    let _ = manager_tx.send(ManagerEvent::SessionAdded {
        session_id: session_id.clone(),
        workspace: workspace.clone(),
        agent: AgentName::Opencode,
        parent_session_id: Some(parent_session_id.clone()),
    });
    let synthetic = serde_json::json!({
        "type": "minos.subagent.spawned",
        "properties": {
            "parent_session_id": parent_session_id.clone(),
            "sub_session_id": session_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "prompt": prompt.clone(),
        }
    });
    if let Err(error) = events_tx
        .emit(RawIngest::from_json(
            AgentName::Opencode,
            synthetic["properties"]["parent_session_id"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            synthetic,
            chrono::Utc::now().timestamp_millis(),
        ))
        .await
    {
        warn!(
            target: "minos_agent_runtime::opencode_driver",
            error = %error,
            "durable ingest sink closed while emitting opencode subagent spawn",
        );
    }
    info!(
        target: "minos_agent_runtime::opencode_driver",
        parent_session_id = %pending.parent_session_id,
        provider_session_id = %session_id,
        tool_call_id = %pending.tool_call_id,
        workspace = %workspace.display(),
        reason = "single_active_task_tool",
        "registered opencode subagent session",
    );
    Some(session_id)
}

async fn update_pending_task_tool(
    payload: &Value,
    parent_session_id: &str,
    pending_tasks: &Arc<Mutex<HashMap<String, PendingTaskTool>>>,
) {
    let Some(update) = task_tool_update(payload, parent_session_id) else {
        return;
    };
    let mut guard = pending_tasks.lock().await;
    if update.active {
        guard.insert(update.task.tool_call_id.clone(), update.task);
    } else {
        guard.remove(&update.task.tool_call_id);
    }
}

struct TaskToolUpdate {
    task: PendingTaskTool,
    active: bool,
}

fn task_tool_update(payload: &Value, parent_session_id: &str) -> Option<TaskToolUpdate> {
    let part = payload
        .get("properties")
        .and_then(|properties| properties.get("part"))
        .or_else(|| payload.get("part"))?;
    if !matches!(
        part.get("type").and_then(Value::as_str),
        Some("tool" | "tool-call")
    ) {
        return None;
    }
    let name = part
        .get("tool")
        .or_else(|| part.get("name"))
        .and_then(Value::as_str)?;
    if name != "task" {
        return None;
    }
    let tool_call_id = part
        .get("callID")
        .or_else(|| part.get("id"))
        .and_then(Value::as_str)?
        .to_string();
    let state = part.get("state").unwrap_or(&Value::Null);
    let status = state
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| state.as_str())
        .unwrap_or("");
    let active = matches!(status, "pending" | "running" | "calling");
    let prompt = state
        .get("input")
        .and_then(|input| input.get("prompt"))
        .or_else(|| part.get("args").and_then(|args| args.get("prompt")))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(TaskToolUpdate {
        task: PendingTaskTool {
            parent_session_id: parent_session_id.to_string(),
            tool_call_id,
            prompt,
        },
        active,
    })
}

fn extract_session_id(payload: &Value) -> Option<&str> {
    let properties = payload.get("properties").unwrap_or(payload);

    properties
        .get("sessionID")
        .and_then(Value::as_str)
        .or_else(|| {
            properties
                .get("info")
                .and_then(|info| info.get("sessionID").or_else(|| info.get("id")))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            properties
                .get("part")
                .and_then(|part| part.get("sessionID"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            payload
                .get("session")
                .and_then(|session| session.get("id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            payload
                .get("message")
                .and_then(|message| message.get("sessionID"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            payload
                .get("part")
                .and_then(|part| part.get("sessionID"))
                .and_then(Value::as_str)
        })
        .or_else(|| payload.get("sessionID").and_then(Value::as_str))
}

async fn sync_session_state(
    payload: &Value,
    session_id: &str,
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    manager_tx: &broadcast::Sender<ManagerEvent>,
) {
    let event_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    let should_idle = match event_type {
        "session.idle" => true,
        "session.status" => {
            payload
                .get("properties")
                .and_then(|properties| properties.get("status"))
                .and_then(|status| status.get("type"))
                .and_then(Value::as_str)
                == Some("idle")
        }
        _ => false,
    };

    if !should_idle {
        return;
    }

    let maybe_transition = {
        let guard = sessions.lock().await;
        guard.get(session_id).and_then(|handle| {
            handle.set_active_turn_id(None);
            let old = handle.current_state();
            if matches!(old, SessionState::Running { .. } | SessionState::Resuming) {
                handle.transition(SessionState::Idle).ok()?;
                Some((old, SessionState::Idle))
            } else {
                None
            }
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn val(s: &str) -> Value {
        serde_json::from_str(s).expect("json fixture should parse")
    }

    #[test]
    fn extract_session_id_supports_current_schema() {
        let payload = val(r#"{
            "type":"message.part.updated",
            "properties":{"part":{"sessionID":"sess_current","messageID":"msg_1","type":"text"}}
        }"#);
        assert_eq!(extract_session_id(&payload), Some("sess_current"));
    }

    #[test]
    fn extract_session_id_supports_legacy_schema() {
        let payload = val(r#"{
            "type":"session.created",
            "session":{"id":"sess_legacy","title":"Legacy"}
        }"#);
        assert_eq!(extract_session_id(&payload), Some("sess_legacy"));
    }

    #[test]
    fn subscribe_sse_url_points_to_single_event_path() {
        let instance = OpencodeServerInstance {
            workspace: "/tmp".into(),
            config: OpencodeServerConfig {
                opencode_bin: "opencode".into(),
                port: 4311,
                password: "pw".into(),
                subprocess_env: Arc::new(HashMap::new()),
                opencode_config_content: None,
            },
            child: None,
            http_client: Client::builder().build().expect("client should build"),
            base_url: "http://127.0.0.1:4311".into(),
            auth_header: "Basic xxx".into(),
        };

        assert_eq!(instance.subscribe_sse_url(), "http://127.0.0.1:4311/event");
    }

    #[tokio::test]
    async fn send_prompt_rejects_non_success_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let instance = OpencodeServerInstance {
            workspace: "/tmp".into(),
            config: OpencodeServerConfig {
                opencode_bin: "opencode".into(),
                port: addr.port(),
                password: "pw".into(),
                subprocess_env: Arc::new(HashMap::new()),
                opencode_config_content: None,
            },
            child: None,
            http_client: Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            base_url: format!("http://{addr}"),
            auth_header: "Basic test".into(),
        };

        let error = instance
            .send_prompt("missing-session", "hello")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("404"));
    }

    #[tokio::test]
    async fn respond_question_posts_answers_to_question_reply_endpoint() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let request_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntrue")
                .await
                .unwrap();
            request
        });

        let instance = OpencodeServerInstance {
            workspace: "/tmp".into(),
            config: OpencodeServerConfig {
                opencode_bin: "opencode".into(),
                port: addr.port(),
                password: "pw".into(),
                subprocess_env: Arc::new(HashMap::new()),
                opencode_config_content: None,
            },
            child: None,
            http_client: Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            base_url: format!("http://{addr}"),
            auth_header: "Basic test".into(),
        };

        instance
            .respond_question("que-1", vec![vec!["Robust".into()]])
            .await
            .expect("question response should succeed");

        let request = request_task.await.expect("request task should finish");
        assert!(request.starts_with("POST /question/que-1/reply "));
        assert!(request.contains(r#""answers":[["Robust"]]"#));
    }
}
