use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    agent_sessions::{SendInputInput, StartAgentSessionInput},
    auth::use_case::AuthUseCase,
    http::{router, BackendState},
    pairing::PairingService,
    realtime::wire::{ClientFrame, ServerFrame},
    session::SessionRegistry,
    store,
};
use minos_domain::{DeviceId, DeviceRole};
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    tungstenite::{http::Uri, protocol::Message},
    MaybeTlsStream, WebSocketStream,
};

const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
const QUIET_TIMEOUT: Duration = Duration::from_millis(150);

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Relay {
    addr: SocketAddr,
    pool: SqlitePool,
    auth: Arc<AuthUseCase>,
    state: BackendState,
    _db_file: NamedTempFile,
    _db_path: PathBuf,
    task: JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn_relay() -> anyhow::Result<Relay> {
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let db_url = format!("sqlite://{}?mode=rwc", tmp_path.display());
    let pool = store::connect(&db_url).await?;
    let registry = Arc::new(SessionRegistry::new());
    let mut state = BackendState::new(
        registry,
        Arc::new(PairingService::new(pool.clone())),
        pool.clone(),
        Duration::from_mins(5),
        TEST_JWT_SECRET.to_string(),
        None,
        "ws-gateway-test".to_string(),
    );
    state.version = "ws-gateway-test";
    let auth = Arc::clone(&state.auth);
    let app = router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        pool,
        auth,
        state,
        _db_file: tmp,
        _db_path: tmp_path,
        task,
    })
}

async fn issue_client_ws_ticket(
    relay: &Relay,
    account_id: &str,
    device_id: DeviceId,
    role: DeviceRole,
) -> anyhow::Result<String> {
    Ok(relay
        .auth
        .issue_ws_ticket(account_id, device_id, role)
        .await
        .map_err(|error| anyhow::anyhow!("issue_ws_ticket failed: {error:?}"))?
        .ticket)
}

async fn issue_host_ws_ticket(relay: &Relay, host_id: DeviceId) -> anyhow::Result<String> {
    Ok(relay
        .auth
        .issue_host_ws_ticket(host_id)
        .await
        .map_err(|error| anyhow::anyhow!("issue_host_ws_ticket failed: {error:?}"))?
        .ticket)
}

async fn connect_client(relay: &Relay, path: &str, ticket: &str) -> anyhow::Result<WsClient> {
    let url: Uri = format!("ws://{}{}?ticket={ticket}", relay.addr, path)
        .parse()
        .unwrap();
    let (ws, _resp) = tokio_tungstenite::connect_async(url.to_string()).await?;
    Ok(ws)
}

async fn recv_server_frame(ws: &mut WsClient) -> anyhow::Result<ServerFrame> {
    loop {
        let next = timeout(RECV_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for server frame"))?;
        match next {
            Some(Ok(Message::Text(text))) => {
                if let Ok(frame) = serde_json::from_str::<ServerFrame>(&text) {
                    return Ok(frame);
                }
            }
            Some(Ok(Message::Ping(payload))) => {
                ws.send(Message::Pong(payload)).await?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(frame))) => {
                return Err(anyhow::anyhow!("unexpected close frame: {frame:?}"));
            }
            Some(Ok(other)) => {
                return Err(anyhow::anyhow!("unexpected websocket frame: {other:?}"))
            }
            Some(Err(error)) => return Err(anyhow::anyhow!("websocket error: {error}")),
            None => return Err(anyhow::anyhow!("websocket ended unexpectedly")),
        }
    }
}

async fn send_client_frame(ws: &mut WsClient, frame: &ClientFrame) -> anyhow::Result<()> {
    let json = serde_json::to_string(frame)?;
    ws.send(Message::Text(json.into())).await?;
    Ok(())
}

async fn seed_client_account(relay: &Relay, email: &str) -> anyhow::Result<(String, DeviceId)> {
    let account_id = store::accounts::create(&relay.pool, email, "phc")
        .await?
        .account_id;
    let device_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;
    Ok((account_id, device_id))
}

async fn seed_host(relay: &Relay) -> anyhow::Result<DeviceId> {
    let host_id = DeviceId::new();
    store::devices::insert_device(&relay.pool, host_id, "Mac", DeviceRole::AgentHost, 0).await?;
    Ok(host_id)
}

async fn seed_session(
    relay: &Relay,
    account_id: &str,
    host_id: DeviceId,
    conversation_id: &str,
    client_request_id: &str,
    initial_user_message: Option<&str>,
) -> anyhow::Result<minos_backend::agent_sessions::StartAgentSessionOutput> {
    relay
        .state
        .agent_sessions
        .start(StartAgentSessionInput {
            conversation_id: conversation_id.to_string(),
            project_id: None,
            agent_id: "agent_codex".into(),
            host_installation_id: Some(host_id.to_string()),
            initial_user_message: initial_user_message.map(str::to_string),
            client_request_id: client_request_id.to_string(),
            caller_account_id: account_id.to_string(),
        })
        .await
        .map_err(|error| anyhow::anyhow!("start session failed: {error}"))
}

async fn send_input(
    relay: &Relay,
    account_id: &str,
    session_id: &str,
    client_request_id: &str,
    text: &str,
) -> anyhow::Result<minos_backend::agent_sessions::SendInputOutput> {
    relay
        .state
        .agent_sessions
        .send_input(SendInputInput {
            session_id: session_id.to_string(),
            text: text.to_string(),
            mentions: Vec::new(),
            client_request_id: client_request_id.to_string(),
            caller_account_id: account_id.to_string(),
        })
        .await
        .map_err(|error| anyhow::anyhow!("send input failed: {error}"))
}

async fn wait_for_host_command_terminal_row(
    relay: &Relay,
    command_id: &str,
) -> anyhow::Result<store::host_commands::HostCommandRow> {
    let deadline = tokio::time::Instant::now() + RECV_TIMEOUT;
    loop {
        if let Some(row) = store::host_commands::get(&relay.pool, command_id).await? {
            if matches!(
                row.status,
                store::host_commands::HostCommandStatus::Succeeded
                    | store::host_commands::HostCommandStatus::Failed
            ) {
                return Ok(row);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for host command row {command_id} to finish");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_host_command_outbox_acked(relay: &Relay, command_id: &str) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + RECV_TIMEOUT;
    loop {
        let (total, unsettled): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*) AS total,
                    COALESCE(SUM(CASE WHEN o.status = 'acked' THEN 0 ELSE 1 END), 0) AS unsettled
               FROM outbox_events o
               JOIN durable_event_log d
                 ON d.topic_kind = o.topic_kind
                AND d.event_id = o.event_id
              WHERE json_extract(d.payload_json, '$.kind') = 'host_command_issued'
                AND json_extract(d.payload_json, '$.command_id') = ?",
        )
        .bind(command_id)
        .fetch_one(&relay.pool)
        .await?;
        if total > 0 && unsettled == 0 {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for host command outbox {command_id} to ack");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn client_subscribe_replays_agent_session_durable_events() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) =
        seed_client_account(&relay, "ws-client-replay@example.com").await?;
    let host_id = seed_host(&relay).await?;
    let members = vec![account_id.clone()];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Gateway Replay",
        &members,
        100,
    )
    .await?;
    store::account_host_pairings::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0)
        .await?;

    let output = seed_session(
        &relay,
        &account_id,
        host_id,
        &conversation.conversation_id,
        "gateway-replay-1",
        Some("hello from durable replay"),
    )
    .await?;

    let ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut ws = connect_client(&relay, "/ws/client", &ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello {
            heartbeat_interval_ms,
            ..
        } => {
            assert_eq!(heartbeat_interval_ms, 25_000);
        }
        other => panic!("expected Hello, got {other:?}"),
    }

    send_client_frame(
        &mut ws,
        &ClientFrame::Subscribe {
            topics: vec![format!("agent_session:{}", output.session_id)],
            resume_after: None,
            client_request_id: Some("req-1".into()),
        },
    )
    .await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::SubscribeAck {
            topics,
            client_request_id,
        } => {
            assert_eq!(topics, vec![format!("agent_session:{}", output.session_id)]);
            assert_eq!(client_request_id.as_deref(), Some("req-1"));
        }
        other => panic!("expected SubscribeAck, got {other:?}"),
    }

    match recv_server_frame(&mut ws).await? {
        ServerFrame::DurableEvent {
            topic,
            kind,
            payload,
            ..
        } => {
            assert_eq!(topic, format!("agent_session:{}", output.session_id));
            assert_eq!(kind, "agent_session_started");
            assert_eq!(payload["session_id"], output.session_id);
            assert_eq!(payload["host_installation_id"], host_id.to_string());
        }
        other => panic!("expected DurableEvent replay, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn client_subscription_denies_host_topics() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-client-deny@example.com").await?;
    let host_id = seed_host(&relay).await?;
    let ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut ws = connect_client(&relay, "/ws/client", &ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    send_client_frame(
        &mut ws,
        &ClientFrame::Subscribe {
            topics: vec![format!("host:{host_id}")],
            resume_after: None,
            client_request_id: Some("deny-host".into()),
        },
    )
    .await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::SubscriptionDenied { topic, reason } => {
            assert_eq!(topic, format!("host:{host_id}"));
            assert_eq!(reason, "forbidden");
        }
        other => panic!("expected SubscriptionDenied, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn client_handshake_stays_quiet_after_hello_without_legacy_bootstrap_frames(
) -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-client-hello@example.com").await?;
    let ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut ws = connect_client(&relay, "/ws/client", &ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    match timeout(QUIET_TIMEOUT, ws.next()).await {
        Err(_) => Ok(()),
        Ok(Some(Ok(frame))) => anyhow::bail!("unexpected post-hello frame: {frame:?}"),
        Ok(Some(Err(error))) => anyhow::bail!("websocket error: {error}"),
        Ok(None) => anyhow::bail!("websocket ended unexpectedly"),
    }
}

#[tokio::test]
async fn host_handshake_stays_quiet_after_hello_without_legacy_checkpoint_frames(
) -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let host_id = seed_host(&relay).await?;
    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut ws = connect_client(&relay, "/ws/host", &host_ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    match timeout(QUIET_TIMEOUT, ws.next()).await {
        Err(_) => Ok(()),
        Ok(Some(Ok(frame))) => anyhow::bail!("unexpected post-hello frame: {frame:?}"),
        Ok(Some(Err(error))) => anyhow::bail!("websocket error: {error}"),
        Ok(None) => anyhow::bail!("websocket ended unexpectedly"),
    }
}

#[tokio::test]
async fn host_replays_durable_command_and_accepts_ack_result() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-host-replay@example.com").await?;
    let host_id = seed_host(&relay).await?;
    let members = vec![account_id.clone()];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Host Replay",
        &members,
        100,
    )
    .await?;
    store::account_host_pairings::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0)
        .await?;

    let output = seed_session(
        &relay,
        &account_id,
        host_id,
        &conversation.conversation_id,
        "host-command-1",
        Some("host durable replay"),
    )
    .await?;

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut ws = connect_client(&relay, "/ws/host", &host_ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    let command_id = match recv_server_frame(&mut ws).await? {
        ServerFrame::DurableEvent {
            topic,
            kind,
            payload,
            ..
        } => {
            assert_eq!(topic, format!("host:{host_id}"));
            assert_eq!(kind, "host_command_issued");
            let command_id = payload["command_id"].as_str().unwrap().to_string();
            assert_eq!(payload["agent_session_id"], output.session_id);
            command_id
        }
        other => panic!("expected host DurableEvent replay, got {other:?}"),
    };

    send_client_frame(
        &mut ws,
        &ClientFrame::HostCommandAck {
            command_id: command_id.clone(),
            ack_at_ms: 1_500,
        },
    )
    .await?;
    send_client_frame(
        &mut ws,
        &ClientFrame::HostCommandResult {
            command_id: command_id.clone(),
            status: "succeeded".into(),
            result: Some(serde_json::json!({ "session_id": output.session_id })),
            error: None,
            finished_at_ms: 1_600,
        },
    )
    .await?;

    let row = wait_for_host_command_terminal_row(&relay, &command_id).await?;
    assert_eq!(
        row.status,
        store::host_commands::HostCommandStatus::Succeeded
    );
    assert_eq!(row.ack_at_ms, Some(1_500));
    assert_eq!(row.finished_at_ms, Some(1_600));
    assert_eq!(
        row.response_json,
        Some(serde_json::json!({ "session_id": output.session_id }))
    );

    Ok(())
}

#[tokio::test]
async fn dispatch_json_issues_durable_host_command_and_returns_result() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let host_id = seed_host(&relay).await?;

    let host_commands = Arc::clone(&relay.state.host_commands);
    let dispatch = tokio::spawn(async move {
        host_commands
            .dispatch_json(
                "cmd-ws-dispatch-json",
                host_id,
                None,
                "minos.test_dispatch",
                &serde_json::json!({ "payload": "hello" }),
                None,
                Duration::from_secs(2),
            )
            .await
    });

    let durable_deadline = tokio::time::Instant::now() + RECV_TIMEOUT;
    loop {
        let rows = store::durable_event_log::read_topic_after(
            &relay.pool,
            "host",
            &format!("host:{host_id}"),
            0,
            10,
        )
        .await?;
        if !rows.is_empty() {
            break;
        }
        if dispatch.is_finished() {
            match dispatch.await.unwrap() {
                Ok(response) => {
                    anyhow::bail!(
                        "dispatch_json finished before durable row appeared with response {response}"
                    );
                }
                Err(error) => {
                    anyhow::bail!("dispatch_json failed before durable row appeared: {error}");
                }
            }
        }
        if tokio::time::Instant::now() >= durable_deadline {
            anyhow::bail!("timed out waiting for durable host command row");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut ws = connect_client(&relay, "/ws/host", &host_ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    let command_id = match recv_server_frame(&mut ws).await? {
        ServerFrame::DurableEvent {
            topic,
            kind,
            payload,
            ..
        } => {
            assert_eq!(topic, format!("host:{host_id}"));
            assert_eq!(kind, "host_command_issued");
            assert_eq!(payload["command_id"], "cmd-ws-dispatch-json");
            assert_eq!(payload["agent_session_id"], serde_json::Value::Null);
            assert_eq!(payload["method"], "minos.test_dispatch");
            assert_eq!(payload["params"]["payload"], "hello");
            payload["command_id"].as_str().unwrap().to_string()
        }
        other => panic!("expected host DurableEvent replay, got {other:?}"),
    };

    send_client_frame(
        &mut ws,
        &ClientFrame::HostCommandAck {
            command_id: command_id.clone(),
            ack_at_ms: 2_500,
        },
    )
    .await?;
    send_client_frame(
        &mut ws,
        &ClientFrame::HostCommandResult {
            command_id: command_id.clone(),
            status: "succeeded".into(),
            result: Some(serde_json::json!({ "accepted": true })),
            error: None,
            finished_at_ms: 2_600,
        },
    )
    .await?;

    assert_eq!(
        dispatch.await.unwrap()?,
        serde_json::json!({ "accepted": true })
    );

    let row = store::host_commands::get(&relay.pool, &command_id)
        .await?
        .expect("host command row should exist");
    assert_eq!(
        row.status,
        store::host_commands::HostCommandStatus::Succeeded
    );
    assert_eq!(row.ack_at_ms, Some(2_500));
    assert_eq!(row.finished_at_ms, Some(2_600));
    assert_eq!(
        row.response_json,
        Some(serde_json::json!({ "accepted": true }))
    );

    Ok(())
}

#[tokio::test]
async fn host_stream_event_persists_slice_and_fanouts_to_subscribed_client() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-stream@example.com").await?;
    let host_id = seed_host(&relay).await?;
    let members = vec![account_id.clone()];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Host Stream",
        &members,
        100,
    )
    .await?;
    store::account_host_pairings::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0)
        .await?;

    let output = seed_session(
        &relay,
        &account_id,
        host_id,
        &conversation.conversation_id,
        "host-stream-1",
        Some("turn seed"),
    )
    .await?;
    let turn_id = output
        .initial_turn_id
        .clone()
        .expect("initial turn should exist for stream test");

    let client_ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut client_ws = connect_client(&relay, "/ws/client", &client_ticket).await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    send_client_frame(
        &mut client_ws,
        &ClientFrame::Subscribe {
            topics: vec![format!("agent_session:{}", output.session_id)],
            resume_after: None,
            client_request_id: Some("stream-subscribe".into()),
        },
    )
    .await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::SubscribeAck { .. } => {}
        other => panic!("expected SubscribeAck, got {other:?}"),
    }
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::DurableEvent { .. } => {}
        other => panic!("expected DurableEvent replay, got {other:?}"),
    }

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut host_ws = connect_client(&relay, "/ws/host", &host_ticket).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::DurableEvent { .. } => {}
        other => panic!("expected host DurableEvent replay, got {other:?}"),
    }

    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostStreamEvent {
            topic: format!("agent_session:{}", output.session_id),
            kind: "agent_text_delta".into(),
            payload: serde_json::json!({
                "turn_id": turn_id,
                "seq": 1,
                "delta": "hello from host stream"
            }),
        },
    )
    .await?;

    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::StreamEvent {
            topic,
            kind,
            seq,
            payload,
        } => {
            assert_eq!(topic, format!("agent_session:{}", output.session_id));
            assert_eq!(kind, "agent_text_delta");
            assert_eq!(seq, Some(1));
            assert_eq!(payload["delta"], "hello from host stream");
        }
        other => panic!("expected StreamEvent fanout, got {other:?}"),
    }

    let rows = store::agent_turn_events::list_for_turn(&relay.pool, &turn_id, None, 10).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "agent_text_delta");

    Ok(())
}

#[tokio::test]
async fn orphan_raw_host_stream_event_does_not_create_legacy_conversation() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-raw-orphan@example.com").await?;
    let host_id = seed_host(&relay).await?;
    store::account_host_pairings::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0)
        .await?;

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut host_ws = connect_client(&relay, "/ws/host", &host_ticket).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostStreamEvent {
            topic: "agent_session:sess_raw_orphan".into(),
            kind: "legacy_raw_event".into(),
            payload: serde_json::json!({
                "seq": 1,
                "method": "session/update",
                "params": {"role": "assistant", "text": "orphan"}
            }),
        },
    )
    .await?;

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(store::agent_sessions::get(&relay.pool, "sess_raw_orphan")
        .await?
        .is_none());
    assert!(
        store::social::list_conversations_for(&relay.pool, &account_id)
            .await?
            .is_empty()
    );

    Ok(())
}

#[tokio::test]
async fn live_durable_events_flow_from_outbox_to_subscribed_host_and_client() -> anyhow::Result<()>
{
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-live-durable@example.com").await?;
    let host_id = seed_host(&relay).await?;
    let members = vec![account_id.clone()];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Live Durable",
        &members,
        100,
    )
    .await?;
    store::account_host_pairings::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0)
        .await?;

    let output = seed_session(
        &relay,
        &account_id,
        host_id,
        &conversation.conversation_id,
        "live-durable-seed",
        Some("seed turn"),
    )
    .await?;

    let client_ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut client_ws = connect_client(&relay, "/ws/client", &client_ticket).await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    send_client_frame(
        &mut client_ws,
        &ClientFrame::Subscribe {
            topics: vec![format!("agent_session:{}", output.session_id)],
            resume_after: None,
            client_request_id: Some("live-durable-client-subscribe".into()),
        },
    )
    .await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::SubscribeAck { .. } => {}
        other => panic!("expected SubscribeAck, got {other:?}"),
    }
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::DurableEvent { kind, .. } => {
            assert_eq!(kind, "agent_session_started");
        }
        other => panic!("expected replay DurableEvent, got {other:?}"),
    }

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut host_ws = connect_client(&relay, "/ws/host", &host_ticket).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    let replayed_start_command_id = match recv_server_frame(&mut host_ws).await? {
        ServerFrame::DurableEvent { kind, payload, .. } => {
            assert_eq!(kind, "host_command_issued");
            payload["command_id"].as_str().unwrap().to_string()
        }
        other => panic!("expected replay DurableEvent, got {other:?}"),
    };
    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostCommandAck {
            command_id: replayed_start_command_id.clone(),
            ack_at_ms: 1_500,
        },
    )
    .await?;
    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostCommandResult {
            command_id: replayed_start_command_id.clone(),
            status: "succeeded".into(),
            result: Some(serde_json::json!({ "session_id": output.session_id })),
            error: None,
            finished_at_ms: 1_600,
        },
    )
    .await?;
    wait_for_host_command_terminal_row(&relay, &replayed_start_command_id).await?;
    wait_for_host_command_outbox_acked(&relay, &replayed_start_command_id).await?;

    let send_output = send_input(
        &relay,
        &account_id,
        &output.session_id,
        "live-durable-send-input",
        "follow-up text",
    )
    .await?;

    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::DurableEvent {
            topic,
            kind,
            payload,
            ..
        } => {
            assert_eq!(topic, format!("agent_session:{}", output.session_id));
            assert_eq!(kind, "agent_turn_appended");
            assert_eq!(payload["turn_id"], send_output.turn_id);
            assert_eq!(payload["turn_seq"], send_output.turn_seq);
        }
        other => panic!("expected live client DurableEvent, got {other:?}"),
    }

    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::DurableEvent {
            topic,
            kind,
            payload,
            ..
        } => {
            assert_eq!(topic, format!("host:{host_id}"));
            assert_eq!(kind, "host_command_issued");
            assert_eq!(payload["agent_session_id"], output.session_id);
            assert_eq!(payload["method"], "agent_session.send_input");
            assert_eq!(payload["params"]["turn_id"], send_output.turn_id);
        }
        other => panic!("expected live host DurableEvent, got {other:?}"),
    }

    Ok(())
}
