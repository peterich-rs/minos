#![cfg(feature = "test-support")]

use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::ws_client::WsClientBuilder;
use minos_agent_runtime::config::AgentRuntimeConfig;
use minos_agent_runtime::test_support::FakeCodexBackend;
use minos_agent_runtime::{AgentManager, InstanceCaps};
use minos_daemon::agent::AgentGlue;
use minos_daemon::local_rpc::{start_local_rpc_server, LocalRpcConfig};
use minos_daemon::store::event_writer::EventWriter;
use minos_daemon::store::LocalStore;
use minos_domain::AgentName;
use minos_protocol::{AgentLaunchMode, ReadSessionParams, StartAgentRequest, StartAgentResponse};

use async_trait::async_trait;
use minos_cli_detect::CommandOutcome;
use minos_domain::MinosError;

struct NoopRunner;

#[async_trait]
impl minos_cli_detect::CommandRunner for NoopRunner {
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

async fn setup() -> (
    Arc<AgentGlue>,
    jsonrpsee::server::ServerHandle,
    tempfile::TempDir,
    FakeCodexBackend,
) {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();

    let store = Arc::new(
        LocalStore::open(&tmp.path().join("test.sqlite"))
            .await
            .unwrap(),
    );

    let (fake, url) = FakeCodexBackend::install().await;
    let mut cfg = AgentRuntimeConfig::new(workspace.clone());
    cfg.test_ws_url = Some(url);
    let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

    let writer = Arc::new(EventWriter::spawn(store.clone()));
    let glue = Arc::new(AgentGlue::wire_with(manager, writer, store, workspace));

    let discovery_path = tmp.path().join("discovery.json");
    let config = LocalRpcConfig {
        addr: "127.0.0.1:0".parse().unwrap(),
        discovery_path,
    };
    let started = start_local_rpc_server(config, Arc::new(NoopRunner), glue.clone())
        .await
        .unwrap();

    (glue, started.handle, tmp, fake)
}

fn discovery_addr(tmp: &tempfile::TempDir) -> String {
    let content = std::fs::read_to_string(tmp.path().join("discovery.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    json["url"].as_str().unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn detect_clis_returns_empty_with_noop_runner() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let response: minos_protocol::ListClisResponse = client
        .request("minos_local_list_clis", ArrayParams::new())
        .await
        .unwrap();

    // One descriptor per known agent (installed or not); NoopRunner still enumerates.
    assert!(response.len() <= minos_domain::AgentName::all().len());
}

#[tokio::test(flavor = "multi_thread")]
async fn start_agent_and_send_message_round_trip() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let start_resp: StartAgentResponse = client
        .request(
            "minos_local_start_agent",
            [StartAgentRequest {
                agent: AgentName::Codex,
                workspace: String::new(),
                mode: Some(AgentLaunchMode::Server),
                profile_id: None,
                model: None,
                reasoning_effort: None,
                instructions: None,
            }],
        )
        .await
        .unwrap();

    assert!(!start_resp.session_id.is_empty());

    client
        .request::<(), _>(
            "minos_local_send_user_message",
            [minos_protocol::SendUserMessageRequest {
                session_id: start_resp.session_id.clone(),
                text: "integration test message".into(),
                origin_message_id: None,
                attachments: vec![],
                delivery_id: None,
                bot_id: None,
            }],
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn list_sessions_returns_data_after_agent_starts() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let threads_before: Vec<minos_protocol::LocalSessionSnapshot> = client
        .request("minos_local_list_local_sessions", ArrayParams::new())
        .await
        .unwrap();
    assert!(threads_before.is_empty());

    let start_resp: StartAgentResponse = client
        .request(
            "minos_local_start_agent",
            [StartAgentRequest {
                agent: AgentName::Codex,
                workspace: String::new(),
                mode: Some(AgentLaunchMode::Server),
                profile_id: None,
                model: None,
                reasoning_effort: None,
                instructions: None,
            }],
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let threads_after: Vec<minos_protocol::LocalSessionSnapshot> = client
        .request("minos_local_list_local_sessions", ArrayParams::new())
        .await
        .unwrap();

    assert_eq!(threads_after.len(), 1);
    assert_eq!(threads_after[0].session_id, start_resp.session_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_session_and_read_history() {
    let (glue, _handle, tmp, fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let start_resp: StartAgentResponse = client
        .request(
            "minos_local_start_agent",
            [StartAgentRequest {
                agent: AgentName::Codex,
                workspace: String::new(),
                mode: Some(AgentLaunchMode::Server),
                profile_id: None,
                model: None,
                reasoning_effort: None,
                instructions: None,
            }],
        )
        .await
        .unwrap();

    let session_id = start_resp.session_id.clone();

    tokio::time::sleep(Duration::from_millis(300)).await;

    let history: minos_protocol::ReadSessionRawHistoryResponse = client
        .request(
            "minos_local_read_session_raw_history",
            [ReadSessionParams {
                session_id: session_id.clone(),
                from_seq: None,
                limit: 100,
            }],
        )
        .await
        .unwrap();
    assert!(
        history
            .events
            .iter()
            .all(|event| event.session_id == session_id),
        "raw history should only contain events for the requested thread"
    );

    let needs = glue
        .manager
        .suspend_for_daemon_stop(&session_id)
        .await
        .unwrap();
    assert!(!needs);
    glue.store()
        .suspend_thread_for_daemon_restart(&session_id, false, 1)
        .await
        .unwrap();

    let resume_resp: StartAgentResponse = client
        .request(
            "minos_local_resume_session",
            [minos_protocol::ResumeSessionRequest {
                session_id: session_id.clone(),
                auto_continue: false,
            }],
        )
        .await
        .unwrap();

    assert_eq!(resume_resp.session_id, session_id);

    fake.stop().await;
}
