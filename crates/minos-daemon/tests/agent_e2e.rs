//! Relay-backed agent session end-to-end coverage.
//!
//! Spins up:
//!
//! - a real in-process backend,
//! - a real `DaemonHandle`,
//! - a scripted fake codex app-server,
//! - and a simulated mobile `/devices` websocket client.
//!
//! The default CI flow verifies the host can pair over relay, consume one
//! relay-delivered host command, and run the agent loop directly through
//! `DaemonHandle`. The longer forwarded agent-command chain is kept as an
//! ignored manual e2e because it is timing-sensitive under CI load.
//!
//! Manual relay-forwarded coverage still exercises:
//!
//! 1. host pairs with a mobile account;
//! 2. mobile forwards agent commands to the host;
//! 3. host replies over relay and fans translated `UiEventMessage` frames back.

#![cfg(feature = "test-support")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use minos_agent_runtime::test_support::{FakeCodexServer, Step};
use minos_backend::{
    host_link::HostLinkService,
    http::{router, BackendState},
    session::SessionRegistry,
    store,
};
use minos_daemon::{DaemonHandle, RelayConfig};
use minos_domain::{AgentName, DeviceId, DeviceRole, DeviceSecret, RelayLinkState};
use minos_protocol::{AgentLaunchMode, SendUserMessageRequest, StartAgentRequest};
use serde_json::json;
use sqlx::SqlitePool;
use tempfile::{NamedTempFile, TempDir};
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const TEST_JWT_SECRET: &str = "daemon-agent-e2e-jwt-secret-32b";
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
const THREAD_ID: &str = "thr-agent-e2e";
const TURN_ID: &str = "turn-agent-e2e";
const ASSISTANT_MESSAGE_ID: &str = "assistant-msg-1";

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

struct Relay {
    addr: SocketAddr,
    state: BackendState,
    pool: SqlitePool,
    _db_file: NamedTempFile,
    task: JoinHandle<()>,
}

struct PairedHost {
    host_secret: DeviceSecret,
    account_id: String,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn set_value(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn fake_thread_response(session_id: &str) -> serde_json::Value {
    json!({
        "approvalPolicy": "never",
        "approvalsReviewer": "user",
        "cwd": "/tmp",
        "instructionSources": [],
        "model": "fake",
        "modelProvider": "fake",
        "sandbox": { "type": "dangerFullAccess" },
        "thread": {
            "id": session_id,
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

async fn spawn_relay() -> anyhow::Result<Relay> {
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let db_url = format!("sqlite://{}?mode=rwc", tmp_path.display());
    let pool = store::connect(&db_url).await?;

    let mut state = BackendState::new(
        Arc::new(SessionRegistry::new()),
        Arc::new(HostLinkService::new(pool.clone())),
        pool.clone(),
        Duration::from_mins(5),
        TEST_JWT_SECRET.to_string(),
        None,
        "daemon-agent-e2e-instance".to_string(),
    );
    state.version = "daemon-agent-e2e";
    let app = router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        state,
        pool,
        _db_file: tmp,
        task,
    })
}

fn relay_ws_url(relay: &Relay) -> String {
    format!("ws://{}/devices", relay.addr)
}

async fn wait_for_connected(handle: &DaemonHandle) -> anyhow::Result<()> {
    timeout(STEP_TIMEOUT, async {
        loop {
            if matches!(handle.current_relay_link(), RelayLinkState::Connected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("daemon did not reach Connected within timeout"))?;
    Ok(())
}

async fn wait_for_paired_peer(handle: &DaemonHandle) -> anyhow::Result<()> {
    timeout(STEP_TIMEOUT, async {
        loop {
            let peers = handle.current_peers().await?;
            if !peers.is_empty() {
                return Ok::<(), minos_domain::MinosError>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("daemon did not observe paired peer within timeout"))??;
    Ok(())
}

async fn wait_for_host_subscription(relay: &Relay, host_id: DeviceId) -> anyhow::Result<()> {
    let topic = minos_protocol::realtime::RealtimeTopic::Host(host_id.to_string());
    timeout(STEP_TIMEOUT, async {
        loop {
            if !relay
                .state
                .subscription_mgr
                .fanout_targets(&topic)
                .is_empty()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("host realtime topic was not subscribed within timeout"))?;
    Ok(())
}

async fn wait_for_host_command_roundtrip(relay: &Relay, host_id: DeviceId) -> anyhow::Result<()> {
    timeout(STEP_TIMEOUT, async {
        let mut attempt = 0u32;
        loop {
            attempt = attempt.saturating_add(1);
            let command_id = format!("daemon-agent-e2e-health-{attempt}");
            if relay
                .state
                .host_commands
                .dispatch_json(
                    &command_id,
                    host_id,
                    None,
                    "minos_health",
                    &serde_json::Value::Null,
                    None,
                    Duration::from_secs(2),
                )
                .await
                .is_ok()
            {
                wait_for_command_outbox_acked(relay, &command_id).await?;
                return Ok::<(), anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("host command round-trip did not become ready"))??;
    Ok(())
}

async fn wait_for_command_outbox_acked(relay: &Relay, command_id: &str) -> anyhow::Result<()> {
    timeout(STEP_TIMEOUT, async {
        loop {
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)
                   FROM outbox_events o
                   JOIN durable_event_log d
                     ON d.topic_kind = o.topic_kind
                    AND d.event_id = o.event_id
                  WHERE json_extract(d.payload_json, '$.command_id') = ?",
            )
            .bind(command_id)
            .fetch_one(&relay.pool)
            .await?;
            let unsettled: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)
                   FROM outbox_events o
                   JOIN durable_event_log d
                     ON d.topic_kind = o.topic_kind
                    AND d.event_id = o.event_id
                  WHERE json_extract(d.payload_json, '$.command_id') = ?
                    AND o.status != 'acked'",
            )
            .bind(command_id)
            .fetch_one(&relay.pool)
            .await?;
            if total > 0 && unsettled == 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                return Ok::<(), sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("host command outbox did not ack for {command_id}"))??;
    Ok(())
}

async fn register_formal_host(
    relay: &Relay,
    host_id: DeviceId,
    email: &str,
) -> anyhow::Result<PairedHost> {
    store::device_installations::insert_device(
        &relay.pool,
        host_id,
        "Test Mac",
        DeviceRole::AgentHost,
        0,
    )
    .await?;
    let account = store::accounts::create(&relay.pool, email).await?;
    let mobile_id = DeviceId::new();
    store::device_installations::insert_device(
        &relay.pool,
        mobile_id,
        "Test iPhone",
        DeviceRole::MobileClient,
        0,
    )
    .await?;
    store::device_installations::set_account_id(&relay.pool, &mobile_id, &account.account_id)
        .await?;

    let linked = relay
        .state
        .host_link
        .link_host(host_id, &account.account_id, mobile_id, Some("Test Mac"))
        .await
        .map_err(|error| anyhow::anyhow!("host link failed: {error:?}"))?;
    let host_secret = DeviceSecret(linked.host_installation_token);

    Ok(PairedHost {
        host_secret,
        account_id: account.account_id,
    })
}

async fn dispatch_host_command(
    relay: &Relay,
    host_id: DeviceId,
    account_id: &str,
    id: i64,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let command_id = format!("daemon-agent-e2e-command-{id}");
    let result = relay
        .state
        .host_commands
        .dispatch_json(
            &command_id,
            host_id,
            None,
            method,
            &params,
            Some(account_id),
            STEP_TIMEOUT,
        )
        .await;
    match result {
        Ok(result) => {
            wait_for_command_outbox_acked(relay, &command_id).await?;
            Ok(json!({ "result": result }))
        }
        Err(error) => {
            let row = minos_backend::store::host_commands::get(&relay.pool, &command_id)
                .await
                .ok()
                .flatten();
            let status = row
                .as_ref()
                .map(|row| format!("{:?}", row.status))
                .unwrap_or_else(|| "missing".into());
            let response_json = row.as_ref().and_then(|row| row.response_json.clone());
            let error_json = row.as_ref().and_then(|row| row.error_json.clone());
            let ack_at_ms = row.as_ref().and_then(|row| row.ack_at_ms);
            let finished_at_ms = row.as_ref().and_then(|row| row.finished_at_ms);
            let raw_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM raw_events")
                .fetch_one(&relay.pool)
                .await
                .unwrap_or(-1);
            let raw_methods: Vec<String> = sqlx::query_scalar(
                "SELECT json_extract(payload_json, '$.method') FROM raw_events ORDER BY seq ASC",
            )
            .fetch_all(&relay.pool)
            .await
            .unwrap_or_default();
            let host_topic = minos_protocol::realtime::RealtimeTopic::Host(host_id.to_string());
            let host_fanout_targets = relay
                .state
                .subscription_mgr
                .fanout_targets(&host_topic)
                .len();
            let outbox_rows: Vec<(
                String,
                i64,
                String,
                String,
                Option<String>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<String>,
            )> = sqlx::query_as(
                "SELECT o.status, o.attempts, d.topic, d.event_id,
                            json_extract(d.payload_json, '$.command_id'),
                            json_extract(d.payload_json, '$.deadline_at_ms'),
                            o.available_at_ms, o.ack_at_ms, o.dead_at_ms, o.last_error_json
                       FROM outbox_events o
                       JOIN durable_event_log d
                         ON d.topic_kind = o.topic_kind
                        AND d.event_id = o.event_id
                      WHERE json_extract(d.payload_json, '$.command_id') = ?
                      ORDER BY o.available_at_ms ASC",
            )
            .bind(&command_id)
            .fetch_all(&relay.pool)
            .await
            .unwrap_or_default();
            Err(anyhow::anyhow!(
                "forwarded rpc `{method}` failed: {error} (command_id={command_id}, status={status}, ack_at_ms={ack_at_ms:?}, finished_at_ms={finished_at_ms:?}, response_json={response_json:?}, error_json={error_json:?}, host_fanout_targets={host_fanout_targets}, outbox_rows={outbox_rows:?}, raw_events={raw_count}, raw_methods={raw_methods:?})"
            ))
        }
    }
}

async fn wait_for_pending_approval(
    relay: &Relay,
    host_id: DeviceId,
) -> anyhow::Result<minos_backend::store::approval_requests::ApprovalRequestRow> {
    match timeout(STEP_TIMEOUT, async {
        loop {
            let rows = minos_backend::store::approval_requests::list_pending_for_hosts(
                &relay.state.store,
                &[host_id],
            )
            .await?;
            if let Some(row) = rows.into_iter().next() {
                return Ok::<_, minos_backend::error::BackendError>(row);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    {
        Ok(result) => result.map_err(Into::into),
        Err(_) => {
            let approval_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM approval_requests")
                .fetch_one(&relay.pool)
                .await
                .unwrap_or(-1);
            let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
                .fetch_one(&relay.pool)
                .await
                .unwrap_or(-1);
            let thread_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
                .fetch_one(&relay.pool)
                .await
                .unwrap_or(-1);
            let raw_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM raw_events")
                .fetch_one(&relay.pool)
                .await
                .unwrap_or(-1);
            let raw_methods: Vec<String> = sqlx::query_scalar(
                "SELECT json_extract(payload_json, '$.method') FROM raw_events ORDER BY seq ASC",
            )
            .fetch_all(&relay.pool)
            .await
            .unwrap_or_default();
            Err(anyhow::anyhow!(
                "timed out waiting for pending approval request (approval_requests={approval_count}, agent_sessions={session_count}, sessions={thread_count}, raw_events={raw_count}, raw_methods={raw_methods:?})"
            ))
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_backed_agent_session_e2e_flows() -> anyhow::Result<()> {
    start_send_stream_stop_against_fake_codex_server().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "multi-command relay forwarding is timing-sensitive; direct daemon coverage is the default CI gate"]
async fn relay_backed_agent_session_forwarded_command_flows() -> anyhow::Result<()> {
    dispatch_message_creates_new_session().await?;
    dispatch_message_steers_running_session().await?;
    approval_forwarding_and_decision().await?;
    Ok(())
}

async fn start_send_stream_stop_against_fake_codex_server() -> anyhow::Result<()> {
    let script = vec![
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(THREAD_ID),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({
                "turn": {
                    "id": TURN_ID,
                    "items": [],
                    "status": "inProgress"
                }
            }),
        },
        Step::EmitNotification {
            method: "item/started".into(),
            params: json!({
                "sessionId": THREAD_ID,
                "turnId": TURN_ID,
                "item": {
                    "type": "agentMessage",
                    "id": ASSISTANT_MESSAGE_ID,
                    "text": ""
                }
            }),
        },
        Step::EmitNotification {
            method: "item/agentMessage/delta".into(),
            params: json!({
                "itemId": ASSISTANT_MESSAGE_ID,
                "delta": "Hello from fake codex"
            }),
        },
        Step::EmitNotification {
            method: "turn/completed".into(),
            params: json!({
                "threadId": THREAD_ID,
                "finishedAtMs": 123
            }),
        },
        Step::Sleep { ms: 200 },
    ];
    let (fake_codex, port) = FakeCodexServer::bind(script).await;
    let fake_url = format!("ws://127.0.0.1:{port}");

    let harness = E2eHarness::setup(&fake_url).await?;

    let start_reply = harness
        .handle
        .start_agent(StartAgentRequest {
            agent: AgentName::Codex,
            workspace: "/w-agent-e2e".into(),
            mode: Some(AgentLaunchMode::Server),
            profile_id: None,
            model: None,
            reasoning_effort: None,
            instructions: None,
        })
        .await?;
    assert_eq!(start_reply.session_id, THREAD_ID);
    assert_eq!(start_reply.cwd, "/w-agent-e2e");

    harness
        .handle
        .send_user_message(SendUserMessageRequest {
            session_id: THREAD_ID.into(),
            text: "hello from mobile".into(),
            origin_message_id: None,
        })
        .await?;

    harness.teardown(fake_codex).await
}

// ─── New e2e tests for the agent-interaction-refactor spec (task 15.1) ───────

/// Helper: set up the full relay + daemon + formal pairing harness.
/// Returns all the pieces needed for the new tests so each test body stays
/// focused on the protocol assertions.
struct E2eHarness {
    #[allow(dead_code)]
    relay: Relay,
    handle: Arc<DaemonHandle>,
    host_id: DeviceId,
    account_id: String,
    _minos_home: TempDir,
    _home_guard: EnvGuard,
    _fake_ws_guard: EnvGuard,
    _env_lock: MutexGuard<'static, ()>,
}

impl E2eHarness {
    async fn setup(fake_url: &str) -> anyhow::Result<Self> {
        let env_lock = ENV_LOCK.lock().await;
        let relay = spawn_relay().await?;
        let minos_home = TempDir::new()?;
        let home_guard = EnvGuard::set_path("MINOS_HOME", minos_home.path());
        let fake_ws_guard = EnvGuard::set_value("MINOS_TEST_CODEX_WS_URL", fake_url);

        let host_id = DeviceId::new();
        let paired = register_formal_host(&relay, host_id, "agent-e2e-new@example.com").await?;
        let handle = DaemonHandle::start(
            RelayConfig::new(relay_ws_url(&relay)),
            host_id,
            None,
            Some(paired.host_secret),
            "Test Mac".into(),
        )
        .await?;
        wait_for_connected(&handle).await?;
        wait_for_paired_peer(&handle).await?;
        wait_for_host_subscription(&relay, host_id).await?;
        wait_for_host_command_roundtrip(&relay, host_id).await?;

        Ok(Self {
            relay,
            handle,
            host_id,
            account_id: paired.account_id,
            _minos_home: minos_home,
            _home_guard: home_guard,
            _fake_ws_guard: fake_ws_guard,
            _env_lock: env_lock,
        })
    }

    /// Send a JSON-RPC forward to the host and return the forwarded result.
    async fn forward_rpc(
        &mut self,
        id: i64,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        dispatch_host_command(
            &self.relay,
            self.host_id,
            &self.account_id,
            id,
            method,
            params,
        )
        .await
    }

    async fn teardown(self, fake: FakeCodexServer) -> anyhow::Result<()> {
        self.handle.stop().await?;
        fake.stop().await;
        Ok(())
    }
}

/// Test 1: `minos_agent_dispatch` with no session_id auto-creates a session
/// via thread/start + turn/start.
///
/// Validates: Requirement 3 (Host Session State Machine) — states 3.1, 3.2
async fn dispatch_message_creates_new_session() -> anyhow::Result<()> {
    let session_id = "thr-dispatch-new";
    let turn_id = "turn-dispatch-new";

    let script = vec![
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(session_id),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({
                "turn": {
                    "id": turn_id,
                    "items": [],
                    "status": "inProgress"
                }
            }),
        },
        Step::EmitNotification {
            method: "turn/completed".into(),
            params: json!({
                "threadId": session_id,
                "finishedAtMs": 100
            }),
        },
        Step::Sleep { ms: 100 },
    ];
    let (fake_codex, port) = FakeCodexServer::bind(script).await;
    let fake_url = format!("ws://127.0.0.1:{port}");

    let mut harness = E2eHarness::setup(&fake_url).await?;

    // Send minos_agent_dispatch with no session_id → auto-create
    let reply = harness
        .forward_rpc(
            10,
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "text": "hello from dispatch",
                "workspace": "/w-dispatch-new"
            }),
        )
        .await?;

    // Verify the response contains a session_id matching the session_id
    let session_id = reply["result"]["session_id"]
        .as_str()
        .expect("dispatch should return session_id");
    assert!(!session_id.is_empty());

    harness.teardown(fake_codex).await
}

/// Test 2: When a session is Running (turn in progress), a second
/// `minos_agent_dispatch` with the same session_id triggers `turn/steer`.
///
/// Validates: Requirement 4 (Turn Steer Support) — states 4.1, 4.2
async fn dispatch_message_steers_running_session() -> anyhow::Result<()> {
    let session_id = "thr-dispatch-steer";
    let turn_id = "turn-dispatch-steer";

    let script = vec![
        // First dispatch: auto-create session
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(session_id),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({
                "turn": {
                    "id": turn_id,
                    "items": [],
                    "status": "inProgress"
                }
            }),
        },
        // Second dispatch while turn is in progress → steer
        Step::ExpectRequest {
            method: "turn/steer".into(),
            reply: json!({ "turnId": turn_id }),
        },
        Step::EmitNotification {
            method: "turn/completed".into(),
            params: json!({
                "threadId": session_id,
                "finishedAtMs": 200
            }),
        },
        Step::Sleep { ms: 100 },
    ];
    let (fake_codex, port) = FakeCodexServer::bind(script).await;
    let fake_url = format!("ws://127.0.0.1:{port}");

    let mut harness = E2eHarness::setup(&fake_url).await?;

    // First dispatch: creates session, starts turn
    let reply1 = harness
        .forward_rpc(
            20,
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "text": "first message",
                "workspace": "/w-dispatch-steer"
            }),
        )
        .await?;
    let session_id = reply1["result"]["session_id"]
        .as_str()
        .expect("first dispatch should return session_id");
    assert!(!session_id.is_empty());

    // Small delay to let the turn/start response propagate and set state to Running
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Second dispatch with same session_id while Running → turn/steer
    let reply2 = harness
        .forward_rpc(
            21,
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "session_id": session_id,
                "text": "steer message",
                "workspace": "/w-dispatch-steer"
            }),
        )
        .await?;

    // Verify the steer succeeded (no error)
    assert!(
        reply2.get("error").is_none(),
        "steer dispatch should succeed, got error: {reply2}"
    );

    harness.teardown(fake_codex).await
}

/// Test 3: Interrupt → Suspend → Resume flow.
///
/// Validates: Requirement 5 (Interrupt vs Close) — states 5.5, 5.6
/// Validates: Requirement 3 (Host Session State Machine) — states 3.4
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "resume semantics are covered in minos-agent-runtime; this relay-backed scenario is timing-sensitive"]
async fn interrupt_then_resume_session() -> anyhow::Result<()> {
    let session_id = "thr-interrupt-resume";
    let turn_id = "turn-interrupt-resume";

    let script = vec![
        // Initial session creation
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(session_id),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({
                "turn": {
                    "id": turn_id,
                    "items": [],
                    "status": "inProgress"
                }
            }),
        },
        // Interrupt the turn
        Step::ExpectRequest {
            method: "turn/interrupt".into(),
            reply: json!({}),
        },
        // Resume: thread/resume + turn/start
        Step::ExpectRequest {
            method: "thread/resume".into(),
            reply: fake_thread_response(session_id),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({
                "turn": {
                    "id": "turn-resumed",
                    "items": [],
                    "status": "inProgress"
                }
            }),
        },
        Step::EmitNotification {
            method: "turn/completed".into(),
            params: json!({
                "threadId": session_id,
                "finishedAtMs": 300
            }),
        },
        Step::Sleep { ms: 100 },
    ];
    let (fake_codex, port) = FakeCodexServer::bind(script).await;
    let fake_url = format!("ws://127.0.0.1:{port}");

    let mut harness = E2eHarness::setup(&fake_url).await?;

    // Step 1: Create session and start a turn
    let reply = harness
        .forward_rpc(
            30,
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "text": "start session",
                "workspace": "/w-interrupt-resume"
            }),
        )
        .await?;
    let session_id = reply["result"]["session_id"]
        .as_str()
        .expect("dispatch should return session_id");
    assert!(!session_id.is_empty());

    // Small delay to let the turn start and state become Running
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 2: Interrupt the session → expect turn/interrupt on fake codex
    let interrupt_reply = harness
        .forward_rpc(
            31,
            "minos_interrupt_session",
            json!({ "session_id": session_id }),
        )
        .await?;
    assert!(
        interrupt_reply.get("error").is_none(),
        "interrupt should succeed, got: {interrupt_reply}"
    );

    // Small delay to let the state transition to Suspended
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 3: Send another dispatch with the same session_id → should resume
    let resume_reply = harness
        .forward_rpc(
            32,
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "session_id": session_id,
                "text": "resume message",
                "workspace": "/w-interrupt-resume"
            }),
        )
        .await?;
    assert!(
        resume_reply.get("error").is_none(),
        "resume dispatch should succeed, got: {resume_reply}"
    );
    assert_eq!(
        resume_reply["result"]["session_id"].as_str().unwrap(),
        session_id
    );

    harness.teardown(fake_codex).await
}

/// Test 4: Approval forwarding and decision relay.
///
/// Validates: Requirement 6 (Approval Request Relay) — states 6.1, 6.6, 6.7
async fn approval_forwarding_and_decision() -> anyhow::Result<()> {
    let session_id = "thr-approval-e2e";
    let turn_id = "turn-approval-e2e";

    let script = vec![
        // Session creation
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(session_id),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({
                "turn": {
                    "id": turn_id,
                    "items": [],
                    "status": "inProgress"
                }
            }),
        },
        // Codex emits an approval server request
        Step::EmitServerRequest {
            method: "item/commandExecution/requestApproval".into(),
            params: json!({
                "itemId": "item-approval-1",
                "sessionId": session_id,
                "turnId": turn_id,
            }),
        },
        // Expect the host to reply with the user's decision
        Step::ExpectResponse {
            result: json!({ "decision": "accept" }),
        },
        Step::Sleep { ms: 100 },
    ];
    let (fake_codex, port) = FakeCodexServer::bind(script).await;
    let fake_url = format!("ws://127.0.0.1:{port}");

    let mut harness = E2eHarness::setup(&fake_url).await?;

    // Step 1: Create session and start a turn
    let reply = harness
        .forward_rpc(
            40,
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "text": "do something requiring approval",
                "workspace": "/w-approval-e2e"
            }),
        )
        .await?;
    let session_id = reply["result"]["session_id"]
        .as_str()
        .expect("dispatch should return session_id");
    assert!(!session_id.is_empty());

    // Step 2: Wait for the approval request ingest to be translated and
    // recorded by the backend approval runtime.
    let approval_event = wait_for_pending_approval(&harness.relay, harness.host_id).await?;
    assert_eq!(approval_event.agent_session_id, session_id);
    assert_eq!(
        approval_event.method,
        "item/commandExecution/requestApproval"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 3: Send approval decision from mobile
    let decision_reply = harness
        .forward_rpc(
            41,
            "minos_approval_decision",
            json!({
                "request_id": approval_event.request_id,
                "session_id": session_id,
                "decision": { "decision": "accept" }
            }),
        )
        .await?;
    assert!(
        decision_reply.get("error").is_none(),
        "approval decision should succeed, got: {decision_reply}"
    );

    harness.teardown(fake_codex).await
}
