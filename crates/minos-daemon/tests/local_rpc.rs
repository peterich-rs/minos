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
use minos_protocol::{
    AgentLaunchMode, ApprovalDecisionRequest, CloseThreadRequest, CreateProjectRequest,
    GetThreadParams, HealthResponse, ListConversationsParams, ListConversationsResponse,
    ListProjectsResponse, LocalGroupChatMessage, LocalGroupChatMessageKind, ReadGroupChatParams,
    ReadGroupChatResponse, ReadThreadParams, StartAgentRequest, StartAgentResponse,
};

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
        group_chat_db_path: tmp.path().join("chat.sqlite"),
    };
    let handle = start_local_rpc_server(config, Arc::new(NoopRunner), glue.clone())
        .await
        .unwrap();

    (glue, handle, tmp, fake)
}

fn discovery_addr(tmp: &tempfile::TempDir) -> String {
    let content = std::fs::read_to_string(tmp.path().join("discovery.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    json["url"].as_str().unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn health_returns_version_and_uptime() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let response: HealthResponse = client
        .request("minos_local_health", ArrayParams::new())
        .await
        .unwrap();

    assert!(!response.version.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn list_local_threads_returns_empty_initially() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let threads: Vec<minos_protocol::LocalThreadSnapshot> = client
        .request("minos_local_list_local_threads", ArrayParams::new())
        .await
        .unwrap();

    assert!(threads.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn project_methods_are_registered_on_local_rpc() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let created: minos_protocol::CreateProjectResponse = client
        .request(
            "minos_local_create_project",
            [CreateProjectRequest {
                name: "Fire".into(),
                workspace_slug: "fire".into(),
                workspace_path: Some(tmp.path().join("fire").display().to_string()),
            }],
        )
        .await
        .unwrap();

    let projects: ListProjectsResponse = client
        .request("minos_local_list_projects", ArrayParams::new())
        .await
        .unwrap();
    assert_eq!(projects.projects.len(), 1);
    assert_eq!(projects.projects[0].project_id, created.project.project_id);

    let conversations: ListConversationsResponse = client
        .request(
            "minos_local_list_conversations",
            [ListConversationsParams {
                project_id: created.project.project_id,
                limit: Some(100),
                before_updated_at_ms: None,
            }],
        )
        .await
        .unwrap();
    assert!(conversations.conversations.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn start_agent_then_list_local_threads_returns_one() {
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
            }],
        )
        .await
        .unwrap();

    assert!(!start_resp.session_id.is_empty());

    tokio::time::sleep(Duration::from_millis(200)).await;

    let threads: Vec<minos_protocol::LocalThreadSnapshot> = client
        .request("minos_local_list_local_threads", ArrayParams::new())
        .await
        .unwrap();

    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].thread_id, start_resp.session_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_thread_removes_local_thread_and_history() {
    let (glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let start_resp: StartAgentResponse = client
        .request(
            "minos_local_start_agent",
            [StartAgentRequest {
                agent: AgentName::Codex,
                workspace: String::new(),
                mode: Some(AgentLaunchMode::Server),
            }],
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    client
        .request::<(), _>(
            "minos_local_delete_thread",
            [CloseThreadRequest {
                thread_id: start_resp.session_id.clone(),
            }],
        )
        .await
        .unwrap();

    let threads: Vec<minos_protocol::LocalThreadSnapshot> = client
        .request("minos_local_list_local_threads", ArrayParams::new())
        .await
        .unwrap();
    assert!(threads.is_empty());

    let event_count: (i64,) = sqlx::query_as("SELECT count(*) FROM events WHERE thread_id = ?")
        .bind(&start_resp.session_id)
        .fetch_one(glue.store().pool())
        .await
        .unwrap();
    assert_eq!(event_count.0, 0);
    assert!(glue
        .store()
        .get_thread(&start_resp.session_id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn send_user_message_round_trips() {
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
            }],
        )
        .await
        .unwrap();

    client
        .request::<(), _>(
            "minos_local_send_user_message",
            [minos_protocol::SendUserMessageRequest {
                session_id: start_resp.session_id.clone(),
                text: "hello test".into(),
            }],
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn approval_decision_rpc_is_registered() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    client
        .request::<(), _>(
            "minos_local_approval_decision",
            [ApprovalDecisionRequest {
                thread_id: "thread-missing".into(),
                request_id: "request-missing".into(),
                decision: serde_json::json!({ "decision": "deny" }),
            }],
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn read_thread_raw_history_returns_events_after_start() {
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
            }],
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    let event_count: (i64,) = sqlx::query_as("SELECT count(*) FROM events WHERE thread_id = ?")
        .bind(&start_resp.session_id)
        .fetch_one(glue.store().pool())
        .await
        .unwrap();

    if event_count.0 > 0 {
        let response: minos_protocol::ReadThreadRawHistoryResponse = client
            .request(
                "minos_local_read_thread_raw_history",
                [ReadThreadParams {
                    thread_id: start_resp.session_id.clone(),
                    from_seq: None,
                    limit: 100,
                }],
            )
            .await
            .unwrap();

        assert!(!response.events.is_empty());
        for event in &response.events {
            assert_eq!(event.thread_id, start_resp.session_id);
        }
    }

    fake.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn read_group_chat_returns_messages_from_local_db_newest_first() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();
    let db_path = tmp.path().join("chat.sqlite");
    let store = minos_chat_store::ChatStore::open(&db_path).await.unwrap();
    store
        .ensure_room("room-main", "main", "/tmp/ws")
        .await
        .unwrap();

    for (kind, text) in [
        (LocalGroupChatMessageKind::User, "@codex inspect auth"),
        (LocalGroupChatMessageKind::AgentResult, "auth summary"),
    ] {
        store
            .append_message(
                "room-main",
                minos_chat_store::NewChatMessage::from(LocalGroupChatMessage {
                    seq: 0,
                    message_id: String::new(),
                    created_at_ms: 10,
                    kind,
                    text: text.into(),
                    agent: Some(AgentName::Codex),
                    thread_id: Some("thread-1".into()),
                    thread_short_id: Some("thread-1".into()),
                    workspace: Some("/tmp/ws".into()),
                }),
            )
            .await
            .unwrap();
    }

    let response: ReadGroupChatResponse = client
        .request(
            "minos_local_read_group_chat",
            [ReadGroupChatParams {
                room_id: Some("room-main".into()),
                after_seq: None,
                before_seq: None,
                limit: Some(1),
            }],
        )
        .await
        .unwrap();

    assert_eq!(response.log_path, db_path.display().to_string());
    assert_eq!(response.messages.len(), 1);
    assert_eq!(response.messages[0].seq, 2);
    assert_eq!(
        response.messages[0].kind,
        LocalGroupChatMessageKind::AgentResult
    );
    assert!(response.has_more);
    assert_eq!(response.next_before_seq, Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_thread_returns_thread_info() {
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
            }],
        )
        .await
        .unwrap();

    let thread_id = start_resp.session_id.clone();

    glue.close_thread(minos_protocol::CloseThreadRequest {
        thread_id: thread_id.clone(),
    })
    .await
    .ok();

    sqlx::query(
        "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart' WHERE thread_id = ?",
    )
    .bind(&thread_id)
    .execute(glue.store().pool())
    .await
    .unwrap();

    let resume_resp: StartAgentResponse = client
        .request(
            "minos_local_resume_thread",
            [GetThreadParams {
                thread_id: thread_id.clone(),
            }],
        )
        .await
        .unwrap();

    assert_eq!(resume_resp.session_id, thread_id);

    fake.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_local_threads_includes_persisted_suspended_threads() {
    let (glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    glue.store()
        .upsert_workspace("/tmp/persisted", 10)
        .await
        .unwrap();
    glue.store()
        .create_project(
            "p-persisted",
            "Persisted",
            "persisted",
            Some("/tmp/persisted"),
            10,
        )
        .await
        .unwrap();
    glue.store()
        .create_conversation("c-persisted", "p-persisted", "Persisted", 10)
        .await
        .unwrap();
    glue.store()
        .insert_thread(
            "thr-persisted",
            "c-persisted",
            "/tmp/persisted",
            "claude",
            None,
            None,
            "idle",
            10,
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart' WHERE thread_id = ?",
    )
    .bind("thr-persisted")
    .execute(glue.store().pool())
    .await
    .unwrap();

    let threads: Vec<minos_protocol::LocalThreadSnapshot> = client
        .request("minos_local_list_local_threads", ArrayParams::new())
        .await
        .unwrap();

    let persisted = threads
        .iter()
        .find(|thread| thread.thread_id == "thr-persisted")
        .expect("persisted thread missing");
    assert_eq!(persisted.agent, AgentName::Claude);
    assert_eq!(persisted.workspace, "/tmp/persisted");
    assert_eq!(
        persisted.state,
        minos_protocol::ThreadState::Suspended {
            reason: minos_protocol::PauseReason::DaemonRestart,
        }
    );
}
