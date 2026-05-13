//! Relay-backed agent session end-to-end coverage.
//!
//! Spins up:
//!
//! - a real in-process backend,
//! - a real `DaemonHandle`,
//! - a scripted fake codex app-server,
//! - and a simulated mobile `/devices` websocket client.
//!
//! The flow under test is the host's core production loop:
//!
//! 1. host pairs with a mobile account;
//! 2. mobile forwards `minos_start_agent` to the host;
//! 3. mobile forwards `minos_send_user_message`;
//! 4. host replies over relay and fans translated `UiEventMessage` frames back.

#![cfg(feature = "test-support")]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use http_body_util::BodyExt as _;
use minos_agent_runtime::test_support::{FakeCodexServer, Step};
use minos_backend::{
    auth::jwt,
    http::{router, BackendState},
    ingest::translate::ThreadTranslators,
    pairing::PairingService,
    session::SessionRegistry,
    store,
};
use minos_daemon::{DaemonHandle, RelayConfig};
use minos_domain::{DeviceId, DeviceRole, RelayLinkState};
use minos_protocol::{Envelope, EventKind};
use minos_ui_protocol::{MessageRole, UiEventMessage};
use serde_json::json;
use sqlx::SqlitePool;
use tempfile::{NamedTempFile, TempDir};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::{
    tungstenite::{client::ClientRequestBuilder, http::Uri, protocol::Message},
    MaybeTlsStream, WebSocketStream,
};
use tower::ServiceExt as _;

const TEST_JWT_SECRET: &str = "daemon-agent-e2e-jwt-secret-32b";
const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const THREAD_ID: &str = "thr-agent-e2e";
const TURN_ID: &str = "turn-agent-e2e";
const ASSISTANT_MESSAGE_ID: &str = "assistant-msg-1";

type WsClient = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Relay {
    addr: SocketAddr,
    state: BackendState,
    _pool: SqlitePool,
    _db_file: NamedTempFile,
    task: JoinHandle<()>,
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

fn fake_thread_response(thread_id: &str) -> serde_json::Value {
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

async fn spawn_relay() -> anyhow::Result<Relay> {
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let db_url = format!("sqlite://{}?mode=rwc", tmp_path.display());
    let pool = store::connect(&db_url).await?;

    let registry = Arc::new(SessionRegistry::new());
    let approval_relay =
        minos_backend::approval_relay::ApprovalRelay::new(pool.clone(), Arc::clone(&registry));
    let state = BackendState {
        registry,
        pairing: Arc::new(PairingService::new(pool.clone())),
        store: pool.clone(),
        token_ttl: Duration::from_mins(5),
        translators: ThreadTranslators::new(),
        approval_relay,
        jwt_secret: Arc::new(TEST_JWT_SECRET.to_string()),
        auth_login_per_email: minos_backend::http::default_login_per_email(),
        auth_login_per_ip: minos_backend::http::default_login_per_ip(),
        auth_register_per_ip: minos_backend::http::default_register_per_ip(),
        auth_refresh_per_acc: minos_backend::http::default_refresh_per_acc(),
        cors_origins: None,
        version: "daemon-agent-e2e",
    };
    let app = router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        state,
        _pool: pool,
        _db_file: tmp,
        task,
    })
}

fn relay_ws_url(relay: &Relay) -> String {
    format!("ws://{}/devices", relay.addr)
}

async fn send_http(
    relay: &Relay,
    req: axum::http::Request<axum::body::Body>,
) -> (axum::http::StatusCode, serde_json::Value) {
    let response = router(relay.state.clone()).oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    (status, body)
}

async fn connect_mobile(
    relay: &Relay,
    device_id: DeviceId,
    bearer: &str,
) -> anyhow::Result<WsClient> {
    let url: Uri = relay_ws_url(relay).parse().unwrap();
    let builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", device_id.to_string())
        .with_header("X-Device-Role", DeviceRole::MobileClient.to_string())
        .with_header("X-Device-Name", "Test iPhone".to_string())
        .with_header("Authorization", format!("Bearer {bearer}"));
    let (ws, _resp) = tokio_tungstenite::connect_async(builder).await?;
    Ok(ws)
}

async fn recv_envelope(ws: &mut WsClient) -> anyhow::Result<Envelope> {
    loop {
        let next = timeout(STEP_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for envelope"))?;
        match next {
            Some(Ok(Message::Text(text))) => return Ok(serde_json::from_str(&text)?),
            Some(Ok(Message::Ping(ping))) => {
                ws.send(Message::Pong(ping)).await?;
            }
            Some(Ok(Message::Pong(_) | Message::Binary(_) | Message::Frame(_))) => {}
            Some(Ok(Message::Close(frame))) => {
                return Err(anyhow::anyhow!("unexpected close frame: {frame:?}"));
            }
            Some(Err(error)) => return Err(anyhow::anyhow!("ws error: {error}")),
            None => return Err(anyhow::anyhow!("stream ended unexpectedly")),
        }
    }
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

async fn recv_forwarded_result(
    ws: &mut WsClient,
    expected_id: i64,
) -> anyhow::Result<serde_json::Value> {
    loop {
        match recv_envelope(ws).await? {
            Envelope::Forwarded { payload, .. }
                if payload.get("id").and_then(serde_json::Value::as_i64) == Some(expected_id) =>
            {
                return Ok(payload);
            }
            Envelope::Event { .. } => {}
            other => {
                return Err(anyhow::anyhow!(
                    "expected Forwarded reply for id {expected_id}, got {other:?}"
                ));
            }
        }
    }
}

fn record_ui_event(ui: UiEventMessage, saw_user_text: &mut bool, saw_assistant_started: &mut bool) {
    match ui {
        UiEventMessage::MessageStarted {
            role: MessageRole::Assistant,
            message_id,
            ..
        } => {
            assert_eq!(message_id, ASSISTANT_MESSAGE_ID);
            *saw_assistant_started = true;
        }
        UiEventMessage::TextDelta { text, .. } if text == "hello from mobile" => {
            *saw_user_text = true;
        }
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_send_stream_stop_against_fake_codex_server() -> anyhow::Result<()> {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let relay = spawn_relay().await?;
    let minos_home = TempDir::new()?;

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
                "threadId": THREAD_ID,
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
    let _home_guard = EnvGuard::set_path("MINOS_HOME", minos_home.path());
    let _fake_ws_guard = EnvGuard::set_value("MINOS_TEST_CODEX_WS_URL", &fake_url);

    let host_id = DeviceId::new();
    let handle = DaemonHandle::start(
        RelayConfig::new(relay_ws_url(&relay)),
        host_id,
        None,
        None,
        "Test Mac".into(),
    )
    .await?;
    wait_for_connected(&handle).await?;

    let qr = handle.pairing_qr().await?;
    let mobile_id = DeviceId::new();
    let account =
        store::accounts::create(&relay.state.store, "agent-e2e@example.com", "phc").await?;
    let bearer = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        &account.account_id,
        &mobile_id.to_string(),
    )?;

    let consume_req = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/v1/pairing/consume")
        .header("content-type", "application/json")
        .header("x-device-id", mobile_id.to_string())
        .header("x-device-role", "mobile-client")
        .header("authorization", format!("Bearer {bearer}"))
        .body(axum::body::Body::from(
            json!({
                "token": qr.pairing_token.as_str(),
                "device_name": "Test iPhone",
            })
            .to_string(),
        ))?;
    let (status, body) = send_http(&relay, consume_req).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(body["peer_device_id"], host_id.to_string());

    wait_for_paired_peer(&handle).await?;

    let mut mobile = connect_mobile(&relay, mobile_id, &bearer).await?;
    let _ = recv_envelope(&mut mobile).await?; // initial Unpaired activation frame

    let start_req = Envelope::Forward {
        version: 1,
        target_device_id: host_id,
        payload: json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "minos_start_agent",
            "params": {
                "agent": "codex",
                "workspace": "/w-agent-e2e",
                "mode": "server"
            }
        }),
    };
    mobile
        .send(Message::Text(serde_json::to_string(&start_req)?.into()))
        .await?;
    let start_reply = recv_forwarded_result(&mut mobile, 1).await?;
    let session_id = start_reply["result"]["session_id"]
        .as_str()
        .expect("start_agent should return session_id");
    assert_eq!(session_id, THREAD_ID);

    let send_req = Envelope::Forward {
        version: 1,
        target_device_id: host_id,
        payload: json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "minos_send_user_message",
            "params": {
                "session_id": THREAD_ID,
                "text": "hello from mobile"
            }
        }),
    };
    mobile
        .send(Message::Text(serde_json::to_string(&send_req)?.into()))
        .await?;
    let mut saw_user_text = false;
    let mut saw_assistant_started = false;
    let mut saw_send_ack = false;
    timeout(STEP_TIMEOUT, async {
        while !(saw_send_ack && saw_user_text && saw_assistant_started) {
            match recv_envelope(&mut mobile).await? {
                Envelope::Forwarded { payload, .. }
                    if payload.get("id").and_then(serde_json::Value::as_i64) == Some(2) =>
                {
                    assert!(
                        payload.get("result").is_some(),
                        "send_user_message should ack with result"
                    );
                    saw_send_ack = true;
                }
                Envelope::Event {
                    event: EventKind::UiEventMessage { thread_id, ui, .. },
                    ..
                } => {
                    assert_eq!(thread_id, THREAD_ID);
                    record_ui_event(ui, &mut saw_user_text, &mut saw_assistant_started);
                }
                Envelope::Event { .. } => {}
                other => {
                    return Err(anyhow::anyhow!(
                        "unexpected envelope while waiting for send ack/ui events: {other:?}"
                    ));
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for forwarded UI events"))??;

    mobile.send(Message::Close(None)).await.ok();
    handle.stop().await?;
    fake_codex.stop().await;
    Ok(())
}

// ─── New e2e tests for the agent-interaction-refactor spec (task 15.1) ───────

/// Helper: set up the full relay + daemon + pairing + mobile WS harness.
/// Returns all the pieces needed for the new tests so each test body stays
/// focused on the protocol assertions.
struct E2eHarness {
    #[allow(dead_code)]
    relay: Relay,
    handle: Arc<DaemonHandle>,
    mobile: WsClient,
    host_id: DeviceId,
    _minos_home: TempDir,
    _env_lock: std::sync::MutexGuard<'static, ()>,
    _home_guard: EnvGuard,
    _fake_ws_guard: EnvGuard,
}

impl E2eHarness {
    async fn setup(fake_url: &str) -> anyhow::Result<Self> {
        let env_lock = ENV_LOCK.lock().unwrap();
        let relay = spawn_relay().await?;
        let minos_home = TempDir::new()?;
        let home_guard = EnvGuard::set_path("MINOS_HOME", minos_home.path());
        let fake_ws_guard = EnvGuard::set_value("MINOS_TEST_CODEX_WS_URL", fake_url);

        let host_id = DeviceId::new();
        let handle = DaemonHandle::start(
            RelayConfig::new(relay_ws_url(&relay)),
            host_id,
            None,
            None,
            "Test Mac".into(),
        )
        .await?;
        wait_for_connected(&handle).await?;

        let qr = handle.pairing_qr().await?;
        let mobile_id = DeviceId::new();
        let account =
            store::accounts::create(&relay.state.store, "agent-e2e-new@example.com", "phc").await?;
        let bearer = jwt::sign(
            TEST_JWT_SECRET.as_bytes(),
            &account.account_id,
            &mobile_id.to_string(),
        )?;

        let consume_req = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri("/v1/pairing/consume")
            .header("content-type", "application/json")
            .header("x-device-id", mobile_id.to_string())
            .header("x-device-role", "mobile-client")
            .header("authorization", format!("Bearer {bearer}"))
            .body(axum::body::Body::from(
                json!({
                    "token": qr.pairing_token.as_str(),
                    "device_name": "Test iPhone",
                })
                .to_string(),
            ))?;
        let (status, body) = send_http(&relay, consume_req).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body["peer_device_id"], host_id.to_string());

        wait_for_paired_peer(&handle).await?;

        let mut mobile = connect_mobile(&relay, mobile_id, &bearer).await?;
        // Drain the initial activation frame (Unpaired/Paired event).
        let _ = recv_envelope(&mut mobile).await?;

        Ok(Self {
            relay,
            handle,
            mobile,
            host_id,
            _minos_home: minos_home,
            _env_lock: env_lock,
            _home_guard: home_guard,
            _fake_ws_guard: fake_ws_guard,
        })
    }

    /// Send a JSON-RPC forward to the host and return the forwarded result.
    async fn forward_rpc(
        &mut self,
        id: i64,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let req = Envelope::Forward {
            version: 1,
            target_device_id: self.host_id,
            payload: json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }),
        };
        self.mobile
            .send(Message::Text(serde_json::to_string(&req)?.into()))
            .await?;
        recv_forwarded_result(&mut self.mobile, id).await
    }

    async fn teardown(mut self, fake: FakeCodexServer) -> anyhow::Result<()> {
        self.mobile.send(Message::Close(None)).await.ok();
        self.handle.stop().await?;
        fake.stop().await;
        Ok(())
    }
}

/// Test 1: `minos_agent_dispatch` with no session_id auto-creates a session
/// via thread/start + turn/start.
///
/// Validates: Requirement 3 (Host Session State Machine) — states 3.1, 3.2
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dispatch_message_creates_new_session() -> anyhow::Result<()> {
    let thread_id = "thr-dispatch-new";
    let turn_id = "turn-dispatch-new";

    let script = vec![
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(thread_id),
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
                "threadId": thread_id,
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

    // Verify the response contains a session_id matching the thread_id
    let session_id = reply["result"]["session_id"]
        .as_str()
        .expect("dispatch should return session_id");
    assert_eq!(session_id, thread_id);

    harness.teardown(fake_codex).await
}

/// Test 2: When a session is Running (turn in progress), a second
/// `minos_agent_dispatch` with the same session_id triggers `turn/steer`.
///
/// Validates: Requirement 4 (Turn Steer Support) — states 4.1, 4.2
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dispatch_message_steers_running_session() -> anyhow::Result<()> {
    let thread_id = "thr-dispatch-steer";
    let turn_id = "turn-dispatch-steer";

    let script = vec![
        // First dispatch: auto-create session
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(thread_id),
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
                "threadId": thread_id,
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
    assert_eq!(session_id, thread_id);

    // Small delay to let the turn/start response propagate and set state to Running
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Second dispatch with same session_id while Running → turn/steer
    let reply2 = harness
        .forward_rpc(
            21,
            "minos_agent_dispatch",
            json!({
                "agent": "codex",
                "session_id": thread_id,
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
async fn interrupt_then_resume_session() -> anyhow::Result<()> {
    let thread_id = "thr-interrupt-resume";
    let turn_id = "turn-interrupt-resume";

    let script = vec![
        // Initial session creation
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(thread_id),
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
            reply: fake_thread_response(thread_id),
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
                "threadId": thread_id,
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
    assert_eq!(session_id, thread_id);

    // Small delay to let the turn start and state become Running
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 2: Interrupt the thread → expect turn/interrupt on fake codex
    let interrupt_reply = harness
        .forward_rpc(
            31,
            "minos_interrupt_thread",
            json!({ "thread_id": thread_id }),
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
                "session_id": thread_id,
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
        thread_id
    );

    harness.teardown(fake_codex).await
}

/// Test 4: Approval forwarding and decision relay.
///
/// Validates: Requirement 6 (Approval Request Relay) — states 6.1, 6.6, 6.7
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_forwarding_and_decision() -> anyhow::Result<()> {
    let thread_id = "thr-approval-e2e";
    let turn_id = "turn-approval-e2e";

    let script = vec![
        // Session creation
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(thread_id),
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
                "threadId": thread_id,
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
    assert_eq!(session_id, thread_id);

    // Step 2: Wait for the approval request event to arrive at mobile.
    // The host forwards the codex ServerRequest as an ingest, which the
    // backend translates into an EventKind::ApprovalRequest event.
    let approval_event = timeout(STEP_TIMEOUT, async {
        loop {
            match recv_envelope(&mut harness.mobile).await? {
                Envelope::Event {
                    event:
                        EventKind::ApprovalRequest {
                            thread_id: tid,
                            request_id,
                            method,
                            ..
                        },
                    ..
                } => {
                    assert_eq!(tid, thread_id);
                    assert!(!request_id.is_empty());
                    assert_eq!(method, "item/commandExecution/requestApproval");
                    return Ok::<String, anyhow::Error>(request_id);
                }
                // Skip other events (UiEventMessage, etc.)
                Envelope::Event { .. } => continue,
                other => {
                    return Err(anyhow::anyhow!(
                        "unexpected envelope waiting for approval: {other:?}"
                    ));
                }
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for approval request event"))??;

    // Step 3: Send approval decision from mobile
    let decision_reply = harness
        .forward_rpc(
            41,
            "minos_approval_decision",
            json!({
                "request_id": approval_event,
                "thread_id": thread_id,
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
