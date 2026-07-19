//! `MinosRpcServer` impl that routes to inner services, plus an
//! envelope-aware [`invoke_forwarded`] entry point.
//!
//! Pre-relay this struct fronted a jsonrpsee WS server; post-Phase-F it is
//! invoked directly from the relay-client dispatch loop when the relay
//! delivers a peer-originated [`Envelope::Forwarded`] frame. Pairing
//! state moved to the relay, so the corresponding fields and the active
//! token / connection-state plumbing are gone.
//!
//! Holds `Arc`s only — cheap to clone once and pass into the dispatcher.

use std::sync::Arc;
use std::time::Instant;

use jsonrpsee::core::async_trait;
use jsonrpsee::types::ErrorObjectOwned;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::MinosError;
use minos_protocol::{
    AgentDispatchRequest, ApprovalDecisionRequest, CloseThreadRequest, GetThreadParams,
    GetThreadResponse, HealthResponse, InterruptThreadRequest, ListClisResponse,
    ListHostSkillsRequest, ListHostSkillsResponse, ListHostWorkspacesRequest,
    ListHostWorkspacesResponse, ListThreadsParams, ListThreadsResponse, MinosRpcServer,
    PairRequest, PairResponse, RespondOpencodeQuestionRequest, SendUserMessageRequest,
    StartAgentRequest, StartAgentResponse, WriteHostSkillConfigRequest,
    WriteHostSkillConfigResponse,
};
use serde_json::{json, Map, Value};

use crate::agent::AgentGlue;

pub struct RpcServerImpl {
    pub started_at: Instant,
    pub runner: Arc<dyn CommandRunner>,
    pub agent: Arc<AgentGlue>,
}

#[async_trait]
impl MinosRpcServer for RpcServerImpl {
    async fn pair(&self, _req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse> {
        // Pairing is owned end-to-end by the backend broker (plan 05 Phase F.3).
        // The Mac receives a Paired event from the backend's HTTP
        // `POST /v1/pairing/consume` handler — it never sees a peer-originated
        // `pair` JSON-RPC. If a forwarded JSON-RPC frame somehow reaches here,
        // the right answer is that the host explicitly does not trust this
        // surface for pairing.
        Err(rpc_err(MinosError::Unauthorized {
            reason: "pair handled by backend, not host".into(),
        }))
    }

    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse> {
        Ok(HealthResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_secs: self.started_at.elapsed().as_secs(),
        })
    }

    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<ListClisResponse> {
        Ok(detect_all(self.runner.clone()).await)
    }

    async fn list_host_skills(
        &self,
        req: ListHostSkillsRequest,
    ) -> jsonrpsee::core::RpcResult<ListHostSkillsResponse> {
        self.agent.list_host_skills(req).await.map_err(rpc_err)
    }

    async fn list_host_workspaces(
        &self,
        req: ListHostWorkspacesRequest,
    ) -> jsonrpsee::core::RpcResult<ListHostWorkspacesResponse> {
        self.agent.list_host_workspaces(req).map_err(rpc_err)
    }

    async fn write_host_skill_config(
        &self,
        req: WriteHostSkillConfigRequest,
    ) -> jsonrpsee::core::RpcResult<WriteHostSkillConfigResponse> {
        self.agent
            .write_host_skill_config(req)
            .await
            .map_err(rpc_err)
    }

    async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        self.agent.start_agent(req).await.map_err(rpc_err)
    }

    async fn send_user_message(
        &self,
        req: SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.send_user_message(req).await.map_err(rpc_err)
    }

    async fn approval_decision(
        &self,
        req: ApprovalDecisionRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.resolve_approval(req).await.map_err(rpc_err)
    }

    async fn respond_opencode_question(
        &self,
        req: RespondOpencodeQuestionRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent
            .respond_opencode_question(req)
            .await
            .map_err(rpc_err)
    }

    async fn interrupt_thread(
        &self,
        req: InterruptThreadRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.interrupt_thread(req).await.map_err(rpc_err)
    }

    async fn close_thread(&self, req: CloseThreadRequest) -> jsonrpsee::core::RpcResult<()> {
        self.agent.close_thread(req).await.map_err(rpc_err)
    }

    async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> jsonrpsee::core::RpcResult<ListThreadsResponse> {
        self.agent.list_threads(req).await.map_err(rpc_err)
    }

    async fn get_thread(
        &self,
        req: GetThreadParams,
    ) -> jsonrpsee::core::RpcResult<GetThreadResponse> {
        self.agent.get_thread(req).await.map_err(rpc_err)
    }
}

pub fn rpc_err(e: MinosError) -> ErrorObjectOwned {
    let code = match e {
        MinosError::PairingStateMismatch { .. } => -32001,
        MinosError::PairingTokenInvalid => -32002,
        MinosError::DeviceNotTrusted { .. } => -32003,
        _ => -32000,
    };
    ErrorObjectOwned::owned(code, e.to_string(), None::<()>)
}

/// Dispatch a host command (method + params) onto the local `RpcServerImpl`.
///
/// Returns `Ok(result_json)` on success or `Err(error_json)` on failure.
/// The caller wraps the result into `ClientFrame::HostCommandResult`.
///
/// Methods are namespaced `minos_*` per the `#[rpc(namespace = "minos")]`
/// derive on [`MinosRpc`].
#[allow(clippy::too_many_lines)]
pub async fn invoke_host_command(
    method: &str,
    params: Value,
    server: &Arc<RpcServerImpl>,
) -> Result<Value, Value> {
    match method {
        "minos_pair" => {
            let req: PairRequest = parse_params(&params)?;
            into_result(server.pair(req).await)
        }
        "minos_health" => into_result(server.health().await),
        "minos_list_clis" => into_result(server.list_clis().await),
        "minos_list_host_skills" => {
            let req: ListHostSkillsRequest = parse_params(&params)?;
            into_result(server.list_host_skills(req).await)
        }
        "minos_list_host_workspaces" => {
            let req: ListHostWorkspacesRequest = parse_params(&params)?;
            into_result(server.list_host_workspaces(req).await)
        }
        "minos_write_host_skill_config" => {
            let req: WriteHostSkillConfigRequest = parse_params(&params)?;
            into_result(server.write_host_skill_config(req).await)
        }
        "minos_start_agent" => {
            let req: StartAgentRequest = parse_params(&params)?;
            into_result(server.start_agent(req).await)
        }
        "agent_session.start" => {
            #[derive(serde::Deserialize)]
            struct StartAgentSessionParams {
                session_id: String,
                agent_id: String,
                #[serde(default)]
                runtime_agent: Option<String>,
                #[serde(default)]
                workspace: String,
                #[serde(default)]
                initial_user_message: Option<String>,
                #[serde(default)]
                model: Option<String>,
                #[serde(default)]
                reasoning_effort: Option<String>,
            }
            let req: StartAgentSessionParams = parse_params(&params)?;
            let agent_label = req.runtime_agent.as_deref().unwrap_or(&req.agent_id);
            let agent = parse_agent_name(agent_label)?;
            let start_req = StartAgentRequest {
                agent,
                workspace: req.workspace,
                mode: None,
                model: req.model,
                reasoning_effort: req.reasoning_effort,
                instructions: None,
            };
            server
                .agent
                .start_agent_with_session_id(req.session_id, start_req, req.initial_user_message)
                .await
                .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                .map_err(rpc_err_value)
        }
        "minos_send_user_message" => {
            let req: SendUserMessageRequest = parse_params(&params)?;
            into_result(server.send_user_message(req).await)
        }
        "agent_session.send_input" => {
            #[derive(serde::Deserialize)]
            struct SendAgentSessionInputParams {
                session_id: String,
                text: String,
            }
            let req: SendAgentSessionInputParams = parse_params(&params)?;
            into_result(
                server
                    .send_user_message(SendUserMessageRequest {
                        session_id: req.session_id,
                        text: req.text,
                    })
                    .await,
            )
        }
        "minos_approval_decision" => {
            let req: ApprovalDecisionRequest = parse_params(&params)?;
            into_result(server.approval_decision(req).await)
        }
        "minos_respond_opencode_question" => {
            let req: RespondOpencodeQuestionRequest = parse_params(&params)?;
            into_result(server.respond_opencode_question(req).await)
        }
        "minos_agent_dispatch" => {
            let req: AgentDispatchRequest = parse_params(&params)?;
            server
                .agent
                .dispatch_message(req)
                .await
                .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                .map_err(|e| rpc_err_value(e))
        }
        "minos_interrupt_thread" => {
            let req: InterruptThreadRequest = parse_params(&params)?;
            into_result(server.interrupt_thread(req).await)
        }
        "minos_close_thread" => {
            let req: CloseThreadRequest = parse_params(&params)?;
            into_result(server.close_thread(req).await)
        }
        "agent_session.stop" => {
            #[derive(serde::Deserialize)]
            struct StopAgentSessionParams {
                session_id: String,
            }
            let req: StopAgentSessionParams = parse_params(&params)?;
            into_result(
                server
                    .close_thread(CloseThreadRequest {
                        thread_id: req.session_id,
                    })
                    .await,
            )
        }
        "minos_list_threads" => {
            let req: ListThreadsParams = parse_params(&params)?;
            into_result(server.list_threads(req).await)
        }
        "minos_get_thread" => {
            let req: GetThreadParams = parse_params(&params)?;
            into_result(server.get_thread(req).await)
        }
        "minos_create_project" => {
            let req: minos_protocol::CreateProjectRequest = parse_params(&params)?;
            server
                .agent
                .create_project(req)
                .await
                .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                .map_err(|e| rpc_err_value(e))
        }
        "minos_list_projects" => server
            .agent
            .list_projects()
            .await
            .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
            .map_err(|e| rpc_err_value(e)),
        "minos_update_project" => {
            let req: minos_protocol::UpdateProjectRequest = parse_params(&params)?;
            server
                .agent
                .update_project(req)
                .await
                .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                .map_err(|e| rpc_err_value(e))
        }
        "minos_delete_project" => {
            let req: minos_protocol::DeleteProjectRequest = parse_params(&params)?;
            server
                .agent
                .delete_project(req)
                .await
                .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                .map_err(|e| rpc_err_value(e))
        }
        other => Err(json!({
            "code": -32601,
            "message": format!("method '{other}' is not host-command-callable"),
        })),
    }
}

fn into_result<T: serde::Serialize>(result: jsonrpsee::core::RpcResult<T>) -> Result<Value, Value> {
    match result {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or(Value::Null)),
        Err(e) => Err(json!({
            "code": e.code(),
            "message": e.message(),
        })),
    }
}

fn rpc_err_value(e: MinosError) -> Value {
    let e = rpc_err(e);
    json!({
        "code": e.code(),
        "message": e.message(),
    })
}

fn parse_agent_name(raw: &str) -> Result<minos_domain::AgentName, Value> {
    match raw {
        "codex" | "agent_codex" => Ok(minos_domain::AgentName::Codex),
        "claude" | "agent_claude" => Ok(minos_domain::AgentName::Claude),
        "gemini" | "agent_gemini" => Ok(minos_domain::AgentName::Gemini),
        "opencode" | "agent_opencode" => Ok(minos_domain::AgentName::Opencode),
        "grok" | "agent_grok" => Ok(minos_domain::AgentName::Grok),
        other => Err(json!({
            "code": -32602,
            "message": format!("unsupported runtime_agent '{other}'"),
        })),
    }
}

fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, Value> {
    if params.is_null() {
        return serde_json::from_value(Value::Object(Map::new()))
            .map_err(|e| json!({"code": -32602, "message": format!("missing params: {e}")}));
    }
    serde_json::from_value(params.clone())
        .map_err(|e| json!({"code": -32602, "message": format!("invalid params: {e}")}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_agent_runtime::test_support::FakeCodexBackend;
    use minos_agent_runtime::{AgentManager, AgentRuntimeConfig, InstanceCaps};
    use minos_cli_detect::CommandOutcome;
    use std::time::Duration;

    /// In-test runner that satisfies the trait without forking a process.
    /// `list_clis` will receive `None`/empty stdout for every probed binary
    /// — that's fine, the dispatcher tests don't assert on the contents.
    struct NoopRunner;

    #[async_trait]
    impl CommandRunner for NoopRunner {
        async fn which(&self, _bin: &str) -> Option<String> {
            None
        }
        async fn run(
            &self,
            _bin: &str,
            _args: &[&str],
            _timeout: Duration,
        ) -> Result<CommandOutcome, MinosError> {
            Ok(CommandOutcome {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    async fn fake_server() -> Arc<RpcServerImpl> {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            crate::store::LocalStore::open(&tmp.path().join("test.sqlite"))
                .await
                .unwrap(),
        );
        Arc::new(RpcServerImpl {
            started_at: Instant::now(),
            runner: Arc::new(NoopRunner),
            agent: Arc::new(AgentGlue::new(
                tmp.path().to_path_buf(),
                Arc::new(std::collections::HashMap::new()),
                store,
            )),
        })
    }

    #[allow(dead_code)]
    fn fake_thread_start_reply(thread_id: &str) -> serde_json::Value {
        json!({
            "approvalPolicy": "never",
            "approvalsReviewer": "user",
            "cwd": "/tmp",
            "instructionSources": [],
            "model": "fake",
            "modelProvider": "fake",
            "sandbox": { "type": "dangerFullAccess" },
            "thread": {
                "id": thread_id,
                "cliVersion": "0.0.0-fake",
                "createdAt": 0,
                "cwd": "/tmp",
                "ephemeral": true,
                "modelProvider": "fake",
                "preview": "",
                "source": "appServer",
                "status": { "type": "idle" },
                "turns": [],
                "updatedAt": 0
            }
        })
    }

    #[allow(dead_code)]
    fn command_approval_params(thread_id: &str, turn_id: &str) -> serde_json::Value {
        json!({
            "itemId": "item-1",
            "threadId": thread_id,
            "turnId": turn_id,
        })
    }

    #[tokio::test]
    async fn invoke_host_command_health_returns_result() {
        let server = fake_server().await;
        let result = invoke_host_command("minos_health", json!({}), &server).await;
        let value = result.unwrap();
        assert!(value["version"].is_string());
        assert!(value["uptime_secs"].is_number());
    }

    #[tokio::test]
    async fn invoke_host_command_pair_returns_error() {
        let server = fake_server().await;
        let result = invoke_host_command(
            "minos_pair",
            json!({
                "device_id": "00000000-0000-0000-0000-000000000000",
                "name": "x",
                "token": "tok",
            }),
            &server,
        )
        .await;
        let err = result.unwrap_err();
        let msg = err["message"].as_str().unwrap_or_default();
        assert!(
            msg.contains("backend"),
            "expected 'backend'-mentioning message, got {msg}"
        );
    }

    #[tokio::test]
    async fn invoke_host_command_unknown_method_returns_error() {
        let server = fake_server().await;
        let result = invoke_host_command("minos_does_not_exist", json!({}), &server).await;
        let err = result.unwrap_err();
        assert_eq!(err["code"], -32601);
    }

    #[tokio::test]
    async fn invoke_host_command_agent_dispatch_returns_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let store = Arc::new(
            crate::store::LocalStore::open(&tmp.path().join("test.sqlite"))
                .await
                .unwrap(),
        );
        let writer = Arc::new(crate::store::event_writer::EventWriter::spawn(
            store.clone(),
        ));
        let server = Arc::new(RpcServerImpl {
            started_at: Instant::now(),
            runner: Arc::new(NoopRunner),
            agent: Arc::new(AgentGlue::wire_with(
                manager,
                writer,
                store,
                tmp.path().to_path_buf(),
            )),
        });

        let result = invoke_host_command(
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "text": "hello from host command",
                "workspace": "/w-rpc-dispatch"
            }),
            &server,
        )
        .await;

        let value = result.unwrap();
        let session_id = value["session_id"]
            .as_str()
            .expect("session_id should be present");
        assert!(!session_id.is_empty());

        fake.stop().await;
    }

    #[tokio::test]
    async fn invoke_host_command_agent_session_start_uses_formal_session_id_and_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("formal-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let store = Arc::new(
            crate::store::LocalStore::open(&tmp.path().join("test.sqlite"))
                .await
                .unwrap(),
        );
        let writer = Arc::new(crate::store::event_writer::EventWriter::spawn(
            store.clone(),
        ));
        let server = Arc::new(RpcServerImpl {
            started_at: Instant::now(),
            runner: Arc::new(NoopRunner),
            agent: Arc::new(AgentGlue::wire_with(
                manager,
                writer,
                store.clone(),
                tmp.path().to_path_buf(),
            )),
        });

        let result = invoke_host_command(
            "agent_session.start",
            json!({
                "session_id": "sess-formal-1",
                "agent_id": "agent_codex",
                "runtime_agent": "codex",
                "workspace": workspace.display().to_string()
            }),
            &server,
        )
        .await;

        let value = result.unwrap();
        assert_eq!(value["session_id"], "sess-formal-1");
        assert_eq!(
            value["cwd"].as_str().unwrap(),
            std::fs::canonicalize(&workspace)
                .unwrap()
                .display()
                .to_string()
        );
        let row = store.get_thread("sess-formal-1").await.unwrap().unwrap();
        assert_eq!(row.workspace_root, value["cwd"].as_str().unwrap());

        fake.stop().await;
    }
}
