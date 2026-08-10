#![cfg(feature = "test-support")]

use std::sync::Arc;
use std::time::Duration;

use futures::{StreamExt, TryStreamExt};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
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
    AgentLaunchMode, AppendConversationMessageParams, AppendConversationMessageResponse,
    ApprovalDecisionRequest, CloseSessionRequest, CreateProjectRequest, HealthResponse,
    ListConversationMessagesParams, ListConversationMessagesResponse, ListConversationsParams,
    ListConversationsResponse, ListProjectsResponse, LocalConversationEvent, ReadSessionParams,
    StartAgentRequest, StartAgentResponse,
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
async fn list_local_sessions_returns_empty_initially() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    let sessions: Vec<minos_protocol::LocalSessionSnapshot> = client
        .request("minos_local_list_local_sessions", ArrayParams::new())
        .await
        .unwrap();

    assert!(sessions.is_empty());
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
async fn start_agent_then_list_local_sessions_returns_one() {
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

    tokio::time::sleep(Duration::from_millis(200)).await;

    let sessions: Vec<minos_protocol::LocalSessionSnapshot> = client
        .request("minos_local_list_local_sessions", ArrayParams::new())
        .await
        .unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, start_resp.session_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_session_removes_local_thread_and_history() {
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
                profile_id: None,
                model: None,
                reasoning_effort: None,
                instructions: None,
            }],
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    client
        .request::<(), _>(
            "minos_local_delete_session",
            [CloseSessionRequest {
                session_id: start_resp.session_id.clone(),
            }],
        )
        .await
        .unwrap();

    let sessions: Vec<minos_protocol::LocalSessionSnapshot> = client
        .request("minos_local_list_local_sessions", ArrayParams::new())
        .await
        .unwrap();
    assert!(sessions.is_empty());

    let event_count: (i64,) = sqlx::query_as("SELECT count(*) FROM events WHERE session_id = ?")
        .bind(&start_resp.session_id)
        .fetch_one(glue.store().pool())
        .await
        .unwrap();
    assert_eq!(event_count.0, 0);
    assert!(glue
        .store()
        .get_session(&start_resp.session_id)
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
                profile_id: None,
                model: None,
                reasoning_effort: None,
                instructions: None,
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
async fn approval_decision_rpc_is_registered() {
    let (_glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();

    // Missing approval must fail-visible (runtime no longer silently Ok).
    let err = client
        .request::<(), _>(
            "minos_local_approval_decision",
            [ApprovalDecisionRequest {
                session_id: "thread-missing".into(),
                request_id: "request-missing".into(),
                decision: serde_json::json!({ "decision": "deny" }),
            }],
        )
        .await
        .expect_err("missing approval must not silently succeed");
    let msg = err.to_string();
    assert!(
        msg.contains("approval request not found") || msg.contains("not found"),
        "expected missing-approval error, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_session_raw_history_returns_events_after_start() {
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

    tokio::time::sleep(Duration::from_millis(500)).await;

    let event_count: (i64,) = sqlx::query_as("SELECT count(*) FROM events WHERE session_id = ?")
        .bind(&start_resp.session_id)
        .fetch_one(glue.store().pool())
        .await
        .unwrap();

    if event_count.0 > 0 {
        let response: minos_protocol::ReadSessionRawHistoryResponse = client
            .request(
                "minos_local_read_session_raw_history",
                [ReadSessionParams {
                    session_id: start_resp.session_id.clone(),
                    from_seq: None,
                    limit: 100,
                }],
            )
            .await
            .unwrap();

        assert!(!response.events.is_empty());
        for event in &response.events {
            assert_eq!(event.session_id, start_resp.session_id);
        }
    }

    fake.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_conversation_messages_returns_messages_from_local_db_newest_first() {
    let (glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();
    let store = glue.store();
    store
        .create_project("project-main", "main", "main", Some("/tmp/ws"), 1)
        .await
        .unwrap();
    store
        .create_conversation("conversation-main", "project-main", "main", 2)
        .await
        .unwrap();
    store.upsert_workspace("/tmp/ws", 3).await.unwrap();
    store
        .insert_session_in_conversation(
            "thread-1",
            "conversation-main",
            "/tmp/ws",
            "codex",
            Some("local-rt-codex"),
            Some("thread-1"),
            None,
            "idle",
            3,
            true,
        )
        .await
        .unwrap();
    store
        .upsert_conversation_message(
            "conversation-main",
            "msg-1",
            None,
            "user",
            None,
            "@codex inspect auth",
            10,
            None,
            None,
            "[]",
        )
        .await
        .unwrap();
    store
        .upsert_conversation_message(
            "conversation-main",
            "msg-2",
            Some("thread-1"),
            "agent",
            Some("local-rt-codex"),
            "auth summary",
            11,
            None,
            None,
            "[]",
        )
        .await
        .unwrap();

    let response: ListConversationMessagesResponse = client
        .request(
            "minos_local_list_conversation_messages",
            [ListConversationMessagesParams {
                conversation_id: "conversation-main".into(),
                before_seq: None,
                limit: Some(1),
            }],
        )
        .await
        .unwrap();

    assert_eq!(response.messages.len(), 1);
    assert_eq!(response.messages[0].message_seq, 2);
    assert_eq!(response.messages[0].sender_role, "agent");
    assert_eq!(response.messages[0].body, "auth summary");
    assert!(response.messages[0].reactions.is_empty());
    assert!(response.has_more);
}

#[tokio::test(flavor = "multi_thread")]
async fn toggle_conversation_message_reaction_add_remove_and_list_embed() {
    use minos_protocol::{
        ToggleConversationMessageReactionParams, ToggleConversationMessageReactionResponse,
        LOCAL_REACTION_ACTOR_ID,
    };

    let (glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();
    let store = glue.store();
    store
        .create_project("project-main", "main", "main", Some("/tmp/ws"), 1)
        .await
        .unwrap();
    store
        .create_conversation("conversation-main", "project-main", "main", 2)
        .await
        .unwrap();
    store
        .upsert_conversation_message(
            "conversation-main",
            "msg-react",
            None,
            "user",
            None,
            "react me",
            10,
            None,
            None,
            "[]",
        )
        .await
        .unwrap();

    let mut events = client
        .subscribe::<LocalConversationEvent, ArrayParams>(
            "minos_local_subscribe_conversation_events",
            ArrayParams::new(),
            "minos_local_unsubscribe_conversation_events",
        )
        .await
        .unwrap()
        .into_stream();

    let added: ToggleConversationMessageReactionResponse = client
        .request(
            "minos_local_toggle_conversation_message_reaction",
            [ToggleConversationMessageReactionParams {
                message_id: "msg-react".into(),
                emoji: "👍".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(added.conversation_id, "conversation-main");
    assert_eq!(added.reactions.len(), 1);
    assert_eq!(added.reactions[0].emoji, "👍");
    assert!(added.reactions[0].reacted_by_me);
    assert_eq!(
        added.reactions[0].actors[0].actor_id,
        LOCAL_REACTION_ACTOR_ID
    );

    let event = tokio::time::timeout(Duration::from_secs(1), events.next())
        .await
        .expect("reaction event should arrive")
        .expect("subscription should stay open")
        .expect("event should decode");
    match event {
        LocalConversationEvent::ConversationReactionToggled {
            conversation_id,
            message_id,
            reactions,
        } => {
            assert_eq!(conversation_id, "conversation-main");
            assert_eq!(message_id, "msg-react");
            assert_eq!(reactions.len(), 1);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let listed: ListConversationMessagesResponse = client
        .request(
            "minos_local_list_conversation_messages",
            [ListConversationMessagesParams {
                conversation_id: "conversation-main".into(),
                before_seq: None,
                limit: Some(10),
            }],
        )
        .await
        .unwrap();
    assert_eq!(listed.messages.len(), 1);
    assert_eq!(listed.messages[0].reactions.len(), 1);
    assert_eq!(listed.messages[0].reactions[0].emoji, "👍");

    let removed: ToggleConversationMessageReactionResponse = client
        .request(
            "minos_local_toggle_conversation_message_reaction",
            [ToggleConversationMessageReactionParams {
                message_id: "msg-react".into(),
                emoji: "👍".into(),
            }],
        )
        .await
        .unwrap();
    assert!(removed.reactions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn append_conversation_message_publishes_conversation_event() {
    let (glue, _handle, tmp, _fake) = setup().await;
    let url = discovery_addr(&tmp);
    let client = WsClientBuilder::default().build(&url).await.unwrap();
    let store = glue.store();
    store
        .create_project("project-main", "main", "main", Some("/tmp/ws"), 1)
        .await
        .unwrap();
    store
        .create_conversation("conversation-main", "project-main", "main", 2)
        .await
        .unwrap();

    let mut events = client
        .subscribe::<LocalConversationEvent, ArrayParams>(
            "minos_local_subscribe_conversation_events",
            ArrayParams::new(),
            "minos_local_unsubscribe_conversation_events",
        )
        .await
        .unwrap()
        .into_stream();

    let response: AppendConversationMessageResponse = client
        .request(
            "minos_local_append_conversation_message",
            [AppendConversationMessageParams {
                conversation_id: "conversation-main".into(),
                message_id: "msg-rpc".into(),
                session_id: None,
                sender_role: "user".into(),
                agent: None,
                body: "visible update".into(),
                reply_to_message_id: None,
                delegation_id: None,
                mentions: vec![],
            }],
        )
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(1), events.next())
        .await
        .expect("append event should arrive")
        .expect("subscription should stay open")
        .expect("event should decode");
    assert_eq!(
        event,
        LocalConversationEvent::ConversationMessageAppended {
            conversation_id: "conversation-main".into(),
            message_seq: response.message_seq,
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_session_returns_thread_info() {
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

    // Stop-path suspend (not close) so the session stays resumable.
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

#[tokio::test(flavor = "multi_thread")]
async fn list_local_sessions_includes_persisted_suspended_threads() {
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
        .insert_session(
            "thr-persisted",
            "c-persisted",
            "/tmp/persisted",
            "claude",
            Some("local-rt-claude"),
            None,
            None,
            "idle",
            10,
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sessions SET status = 'suspended', last_pause_reason = 'daemon_restart' WHERE session_id = ?",
    )
    .bind("thr-persisted")
    .execute(glue.store().pool())
    .await
    .unwrap();

    let sessions: Vec<minos_protocol::LocalSessionSnapshot> = client
        .request("minos_local_list_local_sessions", ArrayParams::new())
        .await
        .unwrap();

    let persisted = sessions
        .iter()
        .find(|thread| thread.session_id == "thr-persisted")
        .expect("persisted session missing");
    assert_eq!(persisted.agent, AgentName::Claude);
    assert_eq!(persisted.workspace, "/tmp/persisted");
    assert_eq!(
        persisted.state,
        minos_protocol::SessionState::Suspended {
            reason: minos_protocol::PauseReason::DaemonRestart,
        }
    );
}
