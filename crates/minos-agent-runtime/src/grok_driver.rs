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

pub struct GrokAcpInstance {
    pub workspace: PathBuf,
    pub child: Arc<tokio::sync::Mutex<Option<Child>>>,
    pub client: Arc<AcpClient>,
    pub session_id: Mutex<Option<String>>,
    pub spawned_at: std::time::Instant,
    pub last_activity_at: Mutex<std::time::Instant>,
    pub crash_signal: mpsc::Sender<()>,
}

/// Build `grok` CLI args for ACP stdio mode.
///
/// `--rules` is a top-level `grok` flag (not under `agent`), so it must come
/// before the `agent` subcommand: `grok --rules "..." agent --no-leader stdio`.
pub(crate) fn build_grok_spawn_args(
    always_approve: bool,
    extra_rules: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(rules) = extra_rules.map(str::trim).filter(|rules| !rules.is_empty()) {
        args.push("--rules".to_string());
        args.push(rules.to_owned());
    }
    // Prefer isolated stdio ACP (same capability surface as `grok agent serve`,
    // without sharing the machine-wide leader socket).
    args.extend([
        "agent".to_string(),
        "--no-leader".to_string(),
        "stdio".to_string(),
    ]);
    if always_approve {
        // Optional auto-approve for unattended local runs via MINOS_GROK_ALWAYS_APPROVE=1.
        args.push("--always-approve".to_string());
    }
    args
}

impl GrokAcpInstance {
    pub async fn spawn(
        cli_path: &Path,
        workspace: &Path,
        subprocess_env: &Arc<HashMap<String, String>>,
        crash_signal: mpsc::Sender<()>,
        extra_rules: Option<&str>,
    ) -> Result<Self, MinosError> {
        let mut cmd = Command::new(cli_path);
        let always_approve = subprocess_env
            .get("MINOS_GROK_ALWAYS_APPROVE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let args = build_grok_spawn_args(always_approve, extra_rules);
        cmd.args(args)
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
            message: format!("failed to spawn grok agent stdio: {e}"),
        })?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(
                        target: "minos_agent_runtime::grok_driver",
                        stderr = %line,
                        "grok ACP stderr"
                    );
                }
            });
        }

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
    thread_id: String,
    events_tx: IngestSink,
    pending_approvals: crate::manager::PendingApprovals,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match client.next_inbound().await {
                Some(crate::acp_client::Inbound::Notification { method, params }) => {
                    if let Err(error) = events_tx
                        .emit(RawIngest::from_json(
                            AgentName::Grok,
                            thread_id.clone(),
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
                            target: "minos_agent_runtime::grok_driver",
                            error = %error,
                            thread_id = %thread_id,
                            "durable ingest sink closed while reading grok notification",
                        );
                        break;
                    }
                }
                Some(crate::acp_client::Inbound::ServerRequest { id, method, params }) => {
                    if method == "session/request_permission" {
                        if let Err(error) = register_acp_permission_request(
                            AgentName::Grok,
                            &client,
                            &thread_id,
                            id,
                            params,
                            &events_tx,
                            &pending_approvals,
                        )
                        .await
                        {
                            tracing::warn!(
                                target: "minos_agent_runtime::grok_driver",
                                error = %error,
                                thread_id = %thread_id,
                                "failed to register grok ACP permission request",
                            );
                        }
                        continue;
                    }

                    // Grok parks plan-approval / ask-user reverse-requests as ACP
                    // extension methods. The ACP library serializes these with a
                    // leading `_`, so the wire JSON-RPC `method` is e.g.
                    // `_x.ai/exit_plan_mode` (NOT `ext_method`, and NOT the bare
                    // `x.ai/...` name). The params are the flat request payload,
                    // not a nested `{method, params}` envelope. See
                    // agent-client-protocol `ext_method` impl and
                    // `ExtRequest { #[serde(skip)] method, params }`.
                    if let Some(nested_method) = method.strip_prefix('_') {
                        if is_known_grok_ext_method(nested_method) {
                            if let Err(error) = handle_grok_ext_method(
                                &client,
                                &thread_id,
                                id,
                                nested_method,
                                params,
                                &events_tx,
                                &pending_approvals,
                            )
                            .await
                            {
                                tracing::warn!(
                                    target: "minos_agent_runtime::grok_driver",
                                    error = %error,
                                    thread_id = %thread_id,
                                    method = %method,
                                    "failed to handle grok ACP ext_method",
                                );
                            }
                            continue;
                        }
                    }

                    if let Err(error) = events_tx
                        .emit(RawIngest::from_json(
                            AgentName::Grok,
                            thread_id.clone(),
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
                            target: "minos_agent_runtime::grok_driver",
                            error = %error,
                            thread_id = %thread_id,
                            "durable ingest sink closed while reading grok server request",
                        );
                        break;
                    }
                    if let Err(error) =
                        reply_to_unsupported_acp_server_request(&client, id, &method).await
                    {
                        tracing::warn!(
                            target: "minos_agent_runtime::grok_driver",
                            error = %error,
                            method = %method,
                            thread_id = %thread_id,
                            "failed to reply to grok ACP server request"
                        );
                    }
                }
                Some(crate::acp_client::Inbound::Closed) => {
                    info!(target: "minos_agent_runtime::grok_driver", thread_id = %thread_id, "grok ACP stream closed");
                    if let Err(error) = events_tx
                        .emit(RawIngest::from_json(
                            AgentName::Grok,
                            thread_id.clone(),
                            serde_json::json!({
                                "kind": "acp_closed",
                                "thread_id": thread_id,
                            }),
                            chrono::Utc::now().timestamp_millis(),
                        ))
                        .await
                    {
                        tracing::warn!(
                            target: "minos_agent_runtime::grok_driver",
                            error = %error,
                            thread_id = %thread_id,
                            "failed to emit grok closed ingest",
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
    thread_id: &str,
    id: serde_json::Value,
    params: serde_json::Value,
    events_tx: &IngestSink,
    pending_approvals: &crate::manager::PendingApprovals,
) -> Result<(), MinosError> {
    let request_id = match &id {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let (allow_option_id, reject_option_id) =
        crate::approvals::acp_permission_option_ids(&params);

    // Surface both the approval overlay envelope and a tool-shaped server
    // request for translators that still render tool call chrome.
    events_tx
        .emit(crate::manager::approval_request_ingest(
            agent,
            thread_id.to_string(),
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
            thread_id.to_string(),
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
            thread_id: thread_id.to_string(),
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

/// Whether a (prefix-stripped) Grok extension method is one Minos handles.
///
/// On the wire these arrive as `_x.ai/exit_plan_mode` / `_x.ai/ask_user_question`;
/// the pump strips the leading `_` before calling this, so the argument here is
/// the bare `x.ai/...` name.
pub(crate) fn is_known_grok_ext_method(nested_method: &str) -> bool {
    matches!(nested_method, "x.ai/exit_plan_mode" | "x.ai/ask_user_question")
}

/// Immediate auto-reply for ext_methods that Minos does not park on UI yet.
pub(crate) fn auto_reply_for_ext_method(nested_method: &str) -> Option<serde_json::Value> {
    match nested_method {
        "x.ai/ask_user_question" => Some(serde_json::json!({ "outcome": "cancelled" })),
        _ => None,
    }
}

/// Whether this ext_method should be parked for user approval.
pub(crate) fn parks_for_user_approval(nested_method: &str) -> bool {
    nested_method == "x.ai/exit_plan_mode"
}

/// Handle a Grok ACP extension reverse-request.
///
/// `nested_method` is the bare `x.ai/...` name (the pump already stripped the
/// leading `_`). `params` is the flat request payload
/// (e.g. `{sessionId, toolCallId, planContent}`), NOT a nested envelope.
async fn handle_grok_ext_method(
    client: &Arc<AcpClient>,
    thread_id: &str,
    id: serde_json::Value,
    nested_method: &str,
    params: serde_json::Value,
    events_tx: &IngestSink,
    pending_approvals: &crate::manager::PendingApprovals,
) -> Result<(), MinosError> {
    if parks_for_user_approval(nested_method) {
        return register_grok_ext_method_approval(
            client,
            thread_id,
            id,
            nested_method,
            params,
            events_tx,
            pending_approvals,
        )
        .await;
    }

    let _ = events_tx
        .emit(RawIngest::from_json(
            AgentName::Grok,
            thread_id.to_owned(),
            serde_json::json!({
                "kind": "acp_server_request",
                "id": id,
                "method": nested_method,
                "params": params,
            }),
            chrono::Utc::now().timestamp_millis(),
        ))
        .await;

    if let Some(result) = auto_reply_for_ext_method(nested_method) {
        info!(
            target: "minos_agent_runtime::grok_driver",
            thread_id = %thread_id,
            method = %nested_method,
            "auto-replying grok ext_method (Minos UI not implemented)"
        );
        return client.reply(id, result).await;
    }

    tracing::warn!(
        target: "minos_agent_runtime::grok_driver",
        thread_id = %thread_id,
        method = %nested_method,
        "unsupported grok ext_method; returning method-not-found"
    );
    client
        .reply_error(
            id,
            -32601,
            format!("unsupported Grok ACP ext_method: {nested_method}"),
        )
        .await
}

async fn register_grok_ext_method_approval(
    client: &Arc<AcpClient>,
    thread_id: &str,
    id: serde_json::Value,
    nested_method: &str,
    nested_params: serde_json::Value,
    events_tx: &IngestSink,
    pending_approvals: &crate::manager::PendingApprovals,
) -> Result<(), MinosError> {
    let request_id = match &id {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let plan_chars = nested_params
        .get("planContent")
        .or_else(|| nested_params.get("plan_content"))
        .and_then(|v| v.as_str())
        .map(|s| s.len())
        .unwrap_or(0);
    info!(
        target: "minos_agent_runtime::grok_driver",
        thread_id = %thread_id,
        request_id = %request_id,
        method = %nested_method,
        plan_chars,
        "parking grok ext_method for user approval"
    );

    events_tx
        .emit(crate::manager::approval_request_ingest(
            AgentName::Grok,
            thread_id.to_owned(),
            request_id.clone(),
            String::new(),
            nested_method.to_owned(),
            nested_params,
        ))
        .await
        .map_err(|_| MinosError::AcpProtocolError {
            method: nested_method.to_owned(),
            message: "durable ingest sink closed while emitting plan approval request".into(),
        })?;

    pending_approvals.insert(
        request_id,
        crate::manager::PendingApproval {
            thread_id: thread_id.to_owned(),
            target: crate::manager::PendingApprovalTarget::GrokExtMethod {
                request_id: id,
                client: client.clone(),
                nested_method: nested_method.to_owned(),
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
                    format!("Minos Grok ACP client does not support {method}"),
                )
                .await
        }
        _ => {
            client
                .reply_error(
                    id,
                    -32601,
                    format!("unsupported Grok ACP server request method: {method}"),
                )
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_known_ext_method_recognizes_plan_and_ask_user() {
        assert!(is_known_grok_ext_method("x.ai/exit_plan_mode"));
        assert!(is_known_grok_ext_method("x.ai/ask_user_question"));
        // The wire prefix is stripped before this check; the bare `x.ai/...`
        // name is what we match, and a leading `_` is NOT present here.
        assert!(!is_known_grok_ext_method("_x.ai/exit_plan_mode"));
        assert!(!is_known_grok_ext_method("x.ai/hooks/run"));
    }

    #[test]
    fn exit_plan_mode_parks_for_user_approval() {
        assert!(parks_for_user_approval("x.ai/exit_plan_mode"));
        assert!(auto_reply_for_ext_method("x.ai/exit_plan_mode").is_none());
    }

    #[test]
    fn auto_reply_ask_user_cancels() {
        let reply = auto_reply_for_ext_method("x.ai/ask_user_question").expect("reply");
        assert_eq!(
            reply.get("outcome").and_then(|v| v.as_str()),
            Some("cancelled")
        );
        assert!(!parks_for_user_approval("x.ai/ask_user_question"));
    }

    #[test]
    fn auto_reply_unknown_is_none() {
        assert!(auto_reply_for_ext_method("x.ai/hooks/run").is_none());
    }

    /// Regression: the wire JSON-RPC method is `_x.ai/exit_plan_mode` and the
    /// params are the FLAT exit_plan_mode payload (not a nested envelope). The
    /// previous implementation matched `method == "ext_method"` and parsed a
    /// nested `{method, params}` shape, so the request fell through to
    /// `-32601` and the agent hung on "Running exit_plan_mode".
    #[test]
    fn wire_method_strips_underscore_prefix() {
        // Simulate the pump's dispatch: strip `_` then check known methods.
        let wire_method = "_x.ai/exit_plan_mode";
        let nested = wire_method.strip_prefix('_').expect("leading underscore");
        assert_eq!(nested, "x.ai/exit_plan_mode");
        assert!(is_known_grok_ext_method(nested));
        assert!(parks_for_user_approval(nested));
    }

    /// The flat plan payload carries camelCase keys per grok-build
    /// `ExitPlanModeExtRequest` (`sessionId`, `toolCallId`, `planContent`).
    #[test]
    fn exit_plan_mode_payload_is_flat_camel_case() {
        let params = serde_json::json!({
            "sessionId": "sess-1",
            "toolCallId": "tc-1",
            "planContent": "# Plan"
        });
        assert_eq!(
            params.get("planContent").and_then(|v| v.as_str()),
            Some("# Plan")
        );
        assert_eq!(
            params.get("toolCallId").and_then(|v| v.as_str()),
            Some("tc-1")
        );
        // No nested envelope wrapper.
        assert!(params.get("method").is_none());
        assert!(params.get("params").is_none());
    }
}
