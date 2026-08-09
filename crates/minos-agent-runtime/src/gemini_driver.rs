#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use minos_acp_protocol::*;
use minos_domain::MinosError;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::info;

use crate::acp_client::AcpClient;
use crate::config::RawIngest;
use crate::manager::IngestSink;
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
        Self::spawn_with_model(cli_path, workspace, subprocess_env, crash_signal, None).await
    }

    pub async fn spawn_with_model(
        cli_path: &Path,
        workspace: &Path,
        subprocess_env: &Arc<HashMap<String, String>>,
        crash_signal: mpsc::Sender<()>,
        model: Option<&str>,
    ) -> Result<Self, MinosError> {
        let mut cmd = Command::new(cli_path);
        let mut args = vec!["--acp".to_string()];
        if let Some(m) = model.map(str::trim).filter(|s| !s.is_empty()) {
            args.push("--model".to_string());
            args.push(m.to_owned());
        }
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

        let mut child = cmd.spawn().map_err(|e| MinosError::GeminiSpawnFailed {
            message: format!("failed to spawn gemini --acp: {e}"),
        })?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(
                        target: "minos_agent_runtime::gemini_driver",
                        stderr = %line,
                        "gemini ACP stderr"
                    );
                }
            });
        }

        // Share the real Child with the instance so shutdown_instances can
        // process-group kill it (AcpClient alone must not own the only handle).
        let child_handle = Arc::new(tokio::sync::Mutex::new(Some(child)));
        let client = AcpClient::from_shared_child(child_handle.clone())?;

        let now = std::time::Instant::now();
        Ok(Self {
            workspace: workspace.to_path_buf(),
            child: child_handle,
            client: Arc::new(client),
            session_id: Mutex::new(None),
            spawned_at: now,
            last_activity_at: Mutex::new(now),
            crash_signal,
        })
    }

    pub async fn initialize(&self) -> Result<InitializeResponse, MinosError> {
        self.client
            .call_typed(InitializeParams {
                protocol_version: 1,
                client_capabilities: Some(ClientCapabilities {
                    fs: FsCapabilities {
                        read_text_file: false,
                        write_text_file: false,
                    },
                    terminal: false,
                }),
                client_info: Some(Implementation {
                    name: "minos".into(),
                    title: Some("Minos Host".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                }),
            })
            .await
    }

    pub async fn authenticate(&self, method_id: &str) -> Result<(), MinosError> {
        self.client
            .call_typed(AuthenticateParams {
                method_id: method_id.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn new_session(
        &self,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
    ) -> Result<NewSessionResponse, MinosError> {
        let resp = self
            .client
            .call_typed(NewSessionParams {
                cwd: cwd.to_string_lossy().to_string(),
                mcp_servers,
                additional_directories: None,
            })
            .await?;
        *self.session_id.lock().await = Some(resp.session_id.clone());
        Ok(resp)
    }

    pub async fn resume_session(
        &self,
        session_id: &str,
        cwd: &Path,
        mcp_servers: Option<Vec<McpServer>>,
    ) -> Result<ResumeSessionResponse, MinosError> {
        let resp = self
            .client
            .call_typed(ResumeSessionParams {
                session_id: session_id.to_string(),
                cwd: cwd.to_string_lossy().to_string(),
                mcp_servers,
                additional_directories: None,
            })
            .await?;
        *self.session_id.lock().await = Some(session_id.to_string());
        Ok(resp)
    }

    pub async fn prompt(&self, text: &str) -> Result<PromptResponse, MinosError> {
        let session_id =
            self.session_id
                .lock()
                .await
                .clone()
                .ok_or_else(|| MinosError::AcpProtocolError {
                    method: "session/prompt".into(),
                    message: "no active session".into(),
                })?;
        self.client
            .call_typed(PromptParams {
                session_id,
                prompt: vec![ContentBlock::Text {
                    text: text.to_string(),
                }],
            })
            .await
    }

    pub async fn cancel(&self) -> Result<(), MinosError> {
        let session_id =
            self.session_id
                .lock()
                .await
                .clone()
                .ok_or_else(|| MinosError::AcpProtocolError {
                    method: "session/cancel".into(),
                    message: "no active session".into(),
                })?;
        self.client
            .notify_typed(CancelNotification { session_id })
            .await
    }

    pub async fn close_session(&self) -> Result<(), MinosError> {
        let session_id =
            self.session_id
                .lock()
                .await
                .clone()
                .ok_or_else(|| MinosError::AcpProtocolError {
                    method: "session/close".into(),
                    message: "no active session".into(),
                })?;
        self.client
            .call_typed(CloseSessionParams { session_id })
            .await?;
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

pub(crate) fn spawn_acp_pump(
    client: Arc<AcpClient>,
    session_id: String,
    events_tx: IngestSink,
    pending_approvals: crate::manager::PendingApprovals,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match client.next_inbound().await {
                Some(crate::acp_client::Inbound::Notification { method, params }) => {
                    if let Err(error) = events_tx
                        .emit(RawIngest::from_json(
                            AgentName::Gemini,
                            session_id.clone(),
                            serde_json::json!({
                                "kind": "acp_notification",
                                "method": method,
                                "params": params,
                            }),
                            chrono::Utc::now().timestamp_millis(),
                        ))
                        .await
                    {
                        tracing::warn!(
                            target: "minos_agent_runtime::gemini_driver",
                            error = %error,
                            session_id = %session_id,
                            "durable ingest sink closed while reading gemini notification",
                        );
                        break;
                    }
                }
                Some(crate::acp_client::Inbound::ServerRequest { id, method, params }) => {
                    if method == "session/request_permission" {
                        if let Err(error) = register_acp_permission_request(
                            AgentName::Gemini,
                            &client,
                            &session_id,
                            id,
                            params,
                            &events_tx,
                            &pending_approvals,
                        )
                        .await
                        {
                            tracing::warn!(
                                target: "minos_agent_runtime::gemini_driver",
                                error = %error,
                                session_id = %session_id,
                                "failed to register gemini ACP permission request",
                            );
                        }
                        continue;
                    }

                    if let Err(error) = events_tx
                        .emit(RawIngest::from_json(
                            AgentName::Gemini,
                            session_id.clone(),
                            serde_json::json!({
                                "kind": "acp_server_request",
                                "id": id,
                                "method": method,
                                "params": params,
                            }),
                            chrono::Utc::now().timestamp_millis(),
                        ))
                        .await
                    {
                        tracing::warn!(
                            target: "minos_agent_runtime::gemini_driver",
                            error = %error,
                            session_id = %session_id,
                            "durable ingest sink closed while reading gemini server request",
                        );
                        break;
                    }
                    if let Err(error) =
                        reply_to_unsupported_acp_server_request(&client, id, &method).await
                    {
                        tracing::warn!(
                            target: "minos_agent_runtime::gemini_driver",
                            error = %error,
                            method = %method,
                            session_id = %session_id,
                            "failed to reply to gemini ACP server request"
                        );
                    }
                }
                Some(crate::acp_client::Inbound::Closed) => {
                    info!(target: "minos_agent_runtime::gemini_driver", session_id = %session_id, "gemini ACP stream closed");
                    if let Err(error) = events_tx
                        .emit(RawIngest::from_json(
                            AgentName::Gemini,
                            session_id.clone(),
                            serde_json::json!({
                                "kind": "acp_closed",
                                "session_id": session_id,
                            }),
                            chrono::Utc::now().timestamp_millis(),
                        ))
                        .await
                    {
                        tracing::warn!(
                            target: "minos_agent_runtime::gemini_driver",
                            error = %error,
                            session_id = %session_id,
                            "failed to emit gemini closed ingest",
                        );
                    }
                    break;
                }
                None => break,
            }
        }
    })
}

async fn register_acp_permission_request(
    agent: AgentName,
    client: &Arc<AcpClient>,
    session_id: &str,
    id: serde_json::Value,
    params: serde_json::Value,
    events_tx: &IngestSink,
    pending_approvals: &crate::manager::PendingApprovals,
) -> Result<(), MinosError> {
    let request_id = match &id {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let (allow_option_id, reject_option_id) = crate::approvals::acp_permission_option_ids(&params);

    // Surface both the approval overlay envelope and a tool-shaped server
    // request for translators that still render tool call chrome.
    events_tx
        .emit(crate::manager::approval_request_ingest(
            agent,
            session_id.to_string(),
            request_id.clone(),
            String::new(),
            "session/request_permission".into(),
            params.clone(),
        ))
        .await
        .map_err(|_| MinosError::AcpProtocolError {
            method: "session/request_permission".into(),
            message: "durable ingest sink closed while emitting approval request".into(),
        })?;
    events_tx
        .emit(RawIngest::from_json(
            agent,
            session_id.to_string(),
            serde_json::json!({
                "kind": "acp_server_request",
                "id": id,
                "method": "session/request_permission",
                "params": params,
            }),
            chrono::Utc::now().timestamp_millis(),
        ))
        .await
        .map_err(|_| MinosError::AcpProtocolError {
            method: "session/request_permission".into(),
            message: "durable ingest sink closed while emitting acp_server_request".into(),
        })?;

    pending_approvals.insert(
        request_id.clone(),
        crate::manager::PendingApproval {
            session_id: session_id.to_string(),
            target: crate::manager::PendingApprovalTarget::Acp {
                request_id: id,
                client: client.clone(),
                allow_option_id,
                reject_option_id,
            },
        },
    );
    Ok(())
}

async fn reply_to_unsupported_acp_server_request(
    client: &AcpClient,
    id: serde_json::Value,
    method: &str,
) -> Result<(), MinosError> {
    match method {
        "fs/read_text_file"
        | "fs/write_text_file"
        | "terminal/create"
        | "terminal/output"
        | "terminal/wait_for_exit"
        | "terminal/kill"
        | "terminal/release" => {
            client
                .reply_error(
                    id,
                    -32000,
                    format!("Minos Gemini ACP client does not support {method}"),
                )
                .await
        }
        _ => {
            client
                .reply_error(
                    id,
                    -32601,
                    format!("unsupported Gemini ACP server request method: {method}"),
                )
                .await
        }
    }
}
