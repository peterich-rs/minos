#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use minos_acp_protocol::*;
use minos_domain::MinosError;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::info;

use crate::acp_client::AcpClient;
use crate::config::RawIngest;
use minos_domain::AgentName;

#[allow(dead_code)]
const KILL_ESCALATION: Duration = Duration::from_secs(3);

pub struct GeminiAcpInstance {
    pub workspace: PathBuf,
    pub child: Arc<tokio::sync::Mutex<Option<Child>>>,
    pub client: Arc<AcpClient>,
    pub session_id: Mutex<Option<String>>,
    pub spawned_at: std::time::Instant,
    pub last_activity_at: Mutex<std::time::Instant>,
    pub crash_signal: mpsc::Sender<()>,
}

impl GeminiAcpInstance {
    pub async fn spawn(
        cli_path: &Path,
        workspace: &Path,
        subprocess_env: &Arc<HashMap<String, String>>,
        crash_signal: mpsc::Sender<()>,
    ) -> Result<Self, MinosError> {
        let mut cmd = Command::new(cli_path);
        cmd.args(["--acp"])
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

        let child = cmd.spawn().map_err(|e| MinosError::GeminiSpawnFailed {
            message: format!("failed to spawn gemini --acp: {e}"),
        })?;

        let client = AcpClient::new(child)?;

        let now = std::time::Instant::now();
        Ok(Self {
            workspace: workspace.to_path_buf(),
            child: Arc::new(tokio::sync::Mutex::new(None)),
            client: Arc::new(client),
            session_id: Mutex::new(None),
            spawned_at: now,
            last_activity_at: Mutex::new(now),
            crash_signal,
        })
    }

    pub async fn initialize(&self) -> Result<InitializeResponse, MinosError> {
        self.client.call_typed(InitializeParams {
            protocol_version: 1,
            client_capabilities: Some(ClientCapabilities {
                fs: FsCapabilities { read_text_file: false, write_text_file: false },
                terminal: false,
            }),
            client_info: Some(Implementation {
                name: "minos".into(),
                title: Some("Minos Host".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        }).await
    }

    pub async fn authenticate(&self, method_id: &str) -> Result<(), MinosError> {
        self.client.call_typed(AuthenticateParams { method_id: method_id.to_string() }).await?;
        Ok(())
    }

    pub async fn new_session(&self, cwd: &Path) -> Result<NewSessionResponse, MinosError> {
        let resp = self.client.call_typed(NewSessionParams {
            cwd: cwd.to_string_lossy().to_string(),
            mcp_servers: vec![],
            additional_directories: None,
        }).await?;
        *self.session_id.lock().await = Some(resp.session_id.clone());
        Ok(resp)
    }

    pub async fn prompt(&self, text: &str) -> Result<PromptResponse, MinosError> {
        let session_id = self.session_id.lock().await.clone().ok_or_else(|| MinosError::AcpProtocolError {
            method: "session/prompt".into(),
            message: "no active session".into(),
        })?;
        self.client.call_typed(PromptParams {
            session_id,
            prompt: vec![ContentBlock::Text { text: text.to_string() }],
        }).await
    }

    pub async fn cancel(&self) -> Result<(), MinosError> {
        let session_id = self.session_id.lock().await.clone().ok_or_else(|| MinosError::AcpProtocolError {
            method: "session/cancel".into(),
            message: "no active session".into(),
        })?;
        self.client.notify_typed(CancelNotification { session_id }).await
    }

    pub async fn close_session(&self) -> Result<(), MinosError> {
        let session_id = self.session_id.lock().await.clone().ok_or_else(|| MinosError::AcpProtocolError {
            method: "session/close".into(),
            message: "no active session".into(),
        })?;
        self.client.call_typed(CloseSessionParams { session_id }).await?;
        *self.session_id.lock().await = None;
        Ok(())
    }

    pub async fn touch(&self) {
        *self.last_activity_at.lock().await = std::time::Instant::now();
    }

    pub async fn get_session_id(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }
}

pub fn spawn_acp_pump(
    client: Arc<AcpClient>,
    thread_id: String,
    events_tx: tokio::sync::broadcast::Sender<RawIngest>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match client.next_inbound().await {
                Some(crate::acp_client::Inbound::Notification { method, params }) => {
                    let _ = events_tx.send(RawIngest {
                        agent: AgentName::Gemini,
                        thread_id: thread_id.clone(),
                        payload: serde_json::json!({
                            "kind": "acp_notification",
                            "method": method,
                            "params": params,
                        }),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
                Some(crate::acp_client::Inbound::ServerRequest { id, method, params }) => {
                    let _ = events_tx.send(RawIngest {
                        agent: AgentName::Gemini,
                        thread_id: thread_id.clone(),
                        payload: serde_json::json!({
                            "kind": "acp_server_request",
                            "id": id,
                            "method": method,
                            "params": params,
                        }),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
                Some(crate::acp_client::Inbound::Closed) => {
                    info!(target: "minos_agent_runtime::gemini_driver", thread_id = %thread_id, "gemini ACP stream closed");
                    let _ = events_tx.send(RawIngest {
                        agent: AgentName::Gemini,
                        thread_id: thread_id.clone(),
                        payload: serde_json::json!({
                            "kind": "acp_closed",
                            "thread_id": thread_id,
                        }),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                    break;
                }
                None => break,
            }
        }
    })
}
