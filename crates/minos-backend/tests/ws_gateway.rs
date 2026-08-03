use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    agent_sessions::{SendInputInput, StartAgentSessionInput},
    auth::use_case::AuthUseCase,
    conversations::{ConversationService, DefaultConversationService},
    host_link::HostLinkService,
    http::{router, BackendState},
    realtime::wire::{ClientFrame, ServerFrame},
    session::SessionRegistry,
    store,
};
use minos_domain::{DeviceId, DeviceRole};
use minos_ui_protocol::UiEventMessage;
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
        Arc::new(HostLinkService::new(pool.clone())),
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
                    // IM presence is fanout noise for most golden-path assertions.
                    if matches!(
                        &frame,
                        ServerFrame::StreamEvent { kind, .. } if kind == "presence"
                    ) {
                        continue;
                    }
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
    let account_id = store::accounts::create(&relay.pool, email)
        .await?
        .account_id;
    let device_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;
    Ok((account_id, device_id))
}

async fn seed_host(relay: &Relay) -> anyhow::Result<DeviceId> {
    let host_id = DeviceId::new();
    store::device_installations::insert_device(
        &relay.pool,
        host_id,
        "Mac",
        DeviceRole::AgentHost,
        0,
    )
    .await?;
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
            workspace_path: None,
            initial_user_message: initial_user_message.map(str::to_string),
            origin_message_id: None,
            client_request_id: client_request_id.to_string(),
            caller_account_id: account_id.to_string(),
            conversation_title: None,
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
            origin_message_id: None,
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
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

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
    drain_default_topic_subscribe_ack(&mut ws).await?;

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
    drain_default_topic_subscribe_ack(&mut ws).await?;

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

/// Seed account durable events so Hello register-only cannot hide a
/// `replay_topic(..., 0)` regression behind an empty log.
async fn seed_account_durable_events(
    relay: &Relay,
    account_id: &str,
    count: i64,
) -> anyhow::Result<()> {
    let topic = format!("account:{account_id}");
    for seq in 1..=count {
        let event_id = format!("seed-account-{account_id}-{seq}");
        let payload = serde_json::json!({
            "kind": "account_registered",
            "account_id": account_id,
            "at_ms": seq * 1000,
        });
        store::durable_event_log::append(
            &relay.pool,
            &event_id,
            &topic,
            "account",
            seq,
            account_id,
            &payload,
            seq * 1000,
        )
        .await?;
    }
    Ok(())
}

async fn seed_host_durable_events(relay: &Relay, host_id: DeviceId, count: i64) -> anyhow::Result<()> {
    let host = host_id.to_string();
    let topic = format!("host:{host}");
    for seq in 1..=count {
        let event_id = format!("seed-host-{host}-{seq}");
        let payload = serde_json::json!({
            "kind": "host_force_close",
            "host_installation_id": host,
            "reason": "seed",
            "at_ms": seq * 1000,
        });
        store::durable_event_log::append(
            &relay.pool,
            &event_id,
            &topic,
            "host",
            seq,
            &host,
            &payload,
            seq * 1000,
        )
        .await?;
    }
    Ok(())
}

/// Drain the register-only default-topic SubscribeAck emitted after Hello.
async fn drain_default_topic_subscribe_ack(ws: &mut WsClient) -> anyhow::Result<()> {
    match recv_server_frame(ws).await? {
        ServerFrame::SubscribeAck { topics, .. } => {
            assert_eq!(
                topics.len(),
                1,
                "expected single default-topic SubscribeAck after Hello"
            );
            Ok(())
        }
        other => anyhow::bail!("expected default-topic SubscribeAck after Hello, got {other:?}"),
    }
}

/// After Hello, accept the default-topic SubscribeAck (live arm) and reject
/// any durable/stream/bootstrap frames for QUIET_TIMEOUT.

/// Host connect path after Hello: drain register-only ack, then Subscribe host
/// topic for catch-up (Hello no longer replays durable history).
async fn host_subscribe_for_catchup(
    ws: &mut WsClient,
    host_id: DeviceId,
) -> anyhow::Result<()> {
    drain_default_topic_subscribe_ack(ws).await?;
    let host_topic = format!("host:{host_id}");
    send_client_frame(
        ws,
        &ClientFrame::Subscribe {
            topics: vec![host_topic.clone()],
            resume_after: None,
            client_request_id: Some("host-catchup".into()),
        },
    )
    .await?;
    match recv_server_frame(ws).await? {
        ServerFrame::SubscribeAck { topics, .. } => {
            assert_eq!(topics, vec![host_topic]);
        }
        other => panic!("expected host SubscribeAck, got {other:?}"),
    }
    Ok(())
}

async fn assert_post_hello_register_only(ws: &mut WsClient) -> anyhow::Result<()> {
    drain_default_topic_subscribe_ack(ws).await?;
    let deadline = tokio::time::Instant::now() + QUIET_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        match timeout(remaining, ws.next()).await {
            Err(_) => return Ok(()),
            Ok(None) => anyhow::bail!("websocket ended unexpectedly"),
            Ok(Some(Err(error))) => anyhow::bail!("websocket error: {error}"),
            Ok(Some(Ok(Message::Ping(payload)))) => {
                ws.send(Message::Pong(payload)).await?;
            }
            Ok(Some(Ok(Message::Pong(_)))) => {}
            Ok(Some(Ok(Message::Text(text)))) => {
                let frame: ServerFrame = serde_json::from_str(&text)?;
                match frame {
                    ServerFrame::StreamEvent { kind, .. } if kind == "presence" => {}
                    other => anyhow::bail!(
                        "unexpected post-hello frame after default SubscribeAck (Hello must not replay durable history): {other:?}"
                    ),
                }
            }
            Ok(Some(Ok(other))) => {
                anyhow::bail!("unexpected websocket frame after Hello: {other:?}")
            }
        }
    }
}

#[tokio::test]
async fn client_handshake_stays_quiet_after_hello_without_legacy_bootstrap_frames(
) -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-client-hello@example.com").await?;
    // Seed history: empty log would hide a resume-from-0 regression.
    seed_account_durable_events(&relay, &account_id, 5).await?;
    let ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut ws = connect_client(&relay, "/ws/client", &ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    assert_post_hello_register_only(&mut ws).await
}

#[tokio::test]
async fn host_handshake_stays_quiet_after_hello_without_legacy_checkpoint_frames(
) -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let host_id = seed_host(&relay).await?;
    seed_host_durable_events(&relay, host_id, 5).await?;
    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut ws = connect_client(&relay, "/ws/host", &host_ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    assert_post_hello_register_only(&mut ws).await
}

#[tokio::test]
async fn subscribe_with_resume_after_filters_below_cursor() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-resume-after@example.com").await?;
    seed_account_durable_events(&relay, &account_id, 10).await?;

    let ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut ws = connect_client(&relay, "/ws/client", &ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    // Consume optional default-topic SubscribeAck (register-only auto-sub).
    let _ = assert_post_hello_register_only(&mut ws).await;

    let account_topic = format!("account:{account_id}");
    send_client_frame(
        &mut ws,
        &ClientFrame::Subscribe {
            topics: vec![account_topic.clone()],
            resume_after: Some(std::collections::HashMap::from([(account_topic.clone(), 5)])),
            client_request_id: Some("resume-after-filter".into()),
        },
    )
    .await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::SubscribeAck { topics, .. } => {
            assert_eq!(topics, vec![account_topic.clone()]);
        }
        other => panic!("expected SubscribeAck, got {other:?}"),
    }

    let mut seqs = Vec::new();
    for _ in 0..5 {
        match recv_server_frame(&mut ws).await? {
            ServerFrame::DurableEvent {
                topic, topic_seq, ..
            } => {
                assert_eq!(topic, account_topic);
                seqs.push(topic_seq);
            }
            other => panic!("expected DurableEvent seq 6..=10, got {other:?}"),
        }
    }
    assert_eq!(seqs, vec![6, 7, 8, 9, 10]);

    // No extra frames after catch-up.
    match timeout(QUIET_TIMEOUT, ws.next()).await {
        Err(_) => Ok(()),
        Ok(Some(Ok(frame))) => anyhow::bail!("unexpected extra frame after resume: {frame:?}"),
        Ok(Some(Err(error))) => anyhow::bail!("websocket error: {error}"),
        Ok(None) => anyhow::bail!("websocket ended unexpectedly"),
    }
}

#[tokio::test]
async fn subscribe_below_retention_floor_emits_snapshot_required() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) =
        seed_client_account(&relay, "ws-snapshot-floor@example.com").await?;
    // Floor: first retained seq is 4 → retention_floor_seq = 3.
    // Client resume_after=1 is below floor → SnapshotRequired (not silent empty).
    let topic = format!("account:{account_id}");
    for seq in 4..=8 {
        let event_id = format!("seed-floor-{account_id}-{seq}");
        let payload = serde_json::json!({
            "kind": "account_registered",
            "account_id": account_id,
            "at_ms": seq * 1000,
        });
        store::durable_event_log::append(
            &relay.pool,
            &event_id,
            &topic,
            "account",
            seq,
            &account_id,
            &payload,
            seq * 1000,
        )
        .await?;
    }

    let ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut ws = connect_client(&relay, "/ws/client", &ticket).await?;
    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    let _ = assert_post_hello_register_only(&mut ws).await;

    send_client_frame(
        &mut ws,
        &ClientFrame::Subscribe {
            topics: vec![topic.clone()],
            resume_after: Some(std::collections::HashMap::from([(topic.clone(), 1)])),
            client_request_id: Some("snapshot-floor".into()),
        },
    )
    .await?;

    // SubscribeAck then SnapshotRequired (order: ack first, then replay).
    match recv_server_frame(&mut ws).await? {
        ServerFrame::SubscribeAck { topics, .. } => {
            assert_eq!(topics, vec![topic.clone()]);
        }
        other => panic!("expected SubscribeAck, got {other:?}"),
    }
    match recv_server_frame(&mut ws).await? {
        ServerFrame::SnapshotRequired {
            topic: t,
            last_known_seq,
            retention_floor_seq,
        } => {
            assert_eq!(t, topic);
            assert_eq!(last_known_seq, 1);
            assert_eq!(retention_floor_seq, 3);
        }
        other => panic!("expected SnapshotRequired, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn account_topic_delivers_social_message_payloads() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) = seed_client_account(&relay, "ws-social@example.com").await?;
    let members = vec![account_id.clone()];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Realtime Social",
        &members,
        100,
    )
    .await?;

    let ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut ws = connect_client(&relay, "/ws/client", &ticket).await?;
    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut ws).await?;
    send_client_frame(
        &mut ws,
        &ClientFrame::Subscribe {
            topics: vec![format!("account:{account_id}")],
            resume_after: None,
            client_request_id: Some("social-account-subscribe".into()),
        },
    )
    .await?;
    match recv_server_frame(&mut ws).await? {
        ServerFrame::SubscribeAck { topics, .. } => {
            assert_eq!(topics, vec![format!("account:{account_id}")]);
        }
        other => panic!("expected SubscribeAck, got {other:?}"),
    }

    let service = DefaultConversationService::new(relay.state.store.clone());
    let (message, _) = service
        .send_message(
            &account_id,
            &conversation.conversation_id,
            "hello while chat is open",
            None,
            None,
            minos_protocol::MessageSource::ClientLive,
            None,
        )
        .await?;
    minos_backend::http::v1::social::fan_out_social_message(&relay.state, &message).await;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::DurableEvent {
            topic,
            kind,
            payload,
            ..
        } => {
            assert_eq!(topic, format!("account:{account_id}"));
            assert_eq!(kind, "account_conversation_message_appended");
            assert_eq!(payload["conversation_id"], conversation.conversation_id);
            // R3 thin account digest: ids + preview, not nested ChatMessageSummary.
            assert_eq!(payload["message_id"], message.message_id);
            assert_eq!(payload["preview"], "hello while chat is open");
            assert!(payload.get("message").is_none());
        }
        other => panic!("expected social DurableEvent, got {other:?}"),
    }

    Ok(())
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
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

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
    // Hello is register-only; catch-up requires explicit Subscribe.
    let _ = assert_post_hello_register_only(&mut ws).await;
    let host_topic = format!("host:{host_id}");
    send_client_frame(
        &mut ws,
        &ClientFrame::Subscribe {
            topics: vec![host_topic.clone()],
            resume_after: None,
            client_request_id: Some("host-replay-subscribe".into()),
        },
    )
    .await?;
    match recv_server_frame(&mut ws).await? {
        ServerFrame::SubscribeAck { topics, .. } => {
            assert_eq!(topics, vec![host_topic.clone()]);
        }
        other => panic!("expected SubscribeAck, got {other:?}"),
    }

    let command_id = match recv_server_frame(&mut ws).await? {
        ServerFrame::DurableEvent {
            topic,
            kind,
            payload,
            ..
        } => {
            assert_eq!(topic, host_topic);
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
    host_subscribe_for_catchup(&mut ws, host_id).await?;

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
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

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
    drain_default_topic_subscribe_ack(&mut client_ws).await?;
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
    host_subscribe_for_catchup(&mut host_ws, host_id).await?;
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
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut host_ws = connect_client(&relay, "/ws/host", &host_ticket).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut host_ws).await?;

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
async fn raw_host_stream_event_updates_formal_turn_cold_replay() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) =
        seed_client_account(&relay, "ws-raw-formal-replay@example.com").await?;
    let bob = store::accounts::create(&relay.pool, "ws-raw-formal-replay-bob@example.com")
        .await?
        .account_id;
    let host_id = seed_host(&relay).await?;
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;
    let members = vec![bob];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Raw Formal Replay",
        &members,
        100,
    )
    .await?;
    let output = seed_session(
        &relay,
        &account_id,
        host_id,
        &conversation.conversation_id,
        "raw-formal-replay-start",
        Some("hello"),
    )
    .await?;
    let user_message = store::social::insert_message(
        &relay.pool,
        &conversation.conversation_id,
        &account_id,
        "hello",
        100,
        None,
        &[],
    )
    .await?;
    store::social::bind_session_to_message(
        &relay.pool,
        &user_message.message_id,
        &output.session_id,
    )
    .await?;

    let client_ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut client_ws = connect_client(&relay, "/ws/client", &client_ticket).await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut client_ws).await?;
    send_client_frame(
        &mut client_ws,
        &ClientFrame::Subscribe {
            topics: vec![format!("agent_session:{}", output.session_id)],
            resume_after: None,
            client_request_id: Some("raw-formal-replay-subscribe".into()),
        },
    )
    .await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::SubscribeAck { .. } => {}
        other => panic!("expected SubscribeAck, got {other:?}"),
    }

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut host_ws = connect_client(&relay, "/ws/host", &host_ticket).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut host_ws).await?;

    let topic = format!("agent_session:{}", output.session_id);
    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostStreamEvent {
            topic: topic.clone(),
            kind: "legacy_raw_event".into(),
            payload: serde_json::json!({
                "seq": 1,
                "agent": "codex",
                "method": "item/started",
                "params": {
                    "item": { "type": "agentMessage", "id": "agent-msg-raw" },
                    "sessionId": output.session_id,
                    "turnId": "turn-raw"
                }
            }),
        },
    )
    .await?;
    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostStreamEvent {
            topic: topic.clone(),
            kind: "legacy_raw_event".into(),
            payload: serde_json::json!({
                "seq": 2,
                "agent": "codex",
                "method": "item/agentMessage/delta",
                "params": {
                    "itemId": "agent-msg-raw",
                    "delta": "Done"
                }
            }),
        },
    )
    .await?;
    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostStreamEvent {
            topic,
            kind: "legacy_raw_event".into(),
            payload: serde_json::json!({
                "seq": 3,
                "agent": "codex",
                "method": "turn/completed",
                "params": {
                    "finishedAtMs": 300
                }
            }),
        },
    )
    .await?;

    let mut saw_completed = false;
    for _ in 0..8 {
        let frame = recv_server_frame(&mut client_ws).await?;
        let ServerFrame::StreamEvent { kind, payload, .. } = frame else {
            continue;
        };
        if kind != "ui_event" {
            continue;
        }
        if matches!(
            serde_json::from_value::<UiEventMessage>(payload)?,
            UiEventMessage::MessageCompleted { message_id, .. } if message_id == "agent-msg-raw"
        ) {
            saw_completed = true;
            break;
        }
    }
    assert!(
        saw_completed,
        "client should receive completed assistant ui_event"
    );

    let turn = store::agent_turns::get(&relay.pool, "agent-msg-raw")
        .await?
        .expect("raw assistant message should be persisted as a formal turn");
    assert_eq!(turn.agent_session_id, output.session_id);
    assert_eq!(turn.role, "assistant");
    assert_eq!(turn.status, "completed");
    assert_eq!(turn.summary_text.as_deref(), Some("Done"));

    let session = store::agent_sessions::get(&relay.pool, &output.session_id)
        .await?
        .expect("session should still exist");
    assert_eq!(session.status, "running");

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
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

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
    drain_default_topic_subscribe_ack(&mut client_ws).await?;
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
    host_subscribe_for_catchup(&mut host_ws, host_id).await?;
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
    // Lanes are independent: social fanout and host_command must both be drained.
    relay.state.realtime.dispatch_outbox_batch().await?;
    relay
        .state
        .realtime
        .dispatch_host_command_outbox_batch()
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

/// Golden path: host live ingest batch → StreamEvent fanout on /ws/client.
#[tokio::test]
async fn host_ingest_live_batch_fans_out_projection_to_subscribed_client() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) =
        seed_client_account(&relay, "ws-host-ingest-live@example.com").await?;
    let host_id = seed_host(&relay).await?;
    let members = vec![account_id.clone()];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Host Ingest Live",
        &members,
        100,
    )
    .await?;
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

    let output = seed_session(
        &relay,
        &account_id,
        host_id,
        &conversation.conversation_id,
        "host-ingest-live-seed",
        Some("seed"),
    )
    .await?;

    let client_ticket =
        issue_client_ws_ticket(&relay, &account_id, phone_id, DeviceRole::MobileClient).await?;
    let mut client_ws = connect_client(&relay, "/ws/client", &client_ticket).await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut client_ws).await?;
    send_client_frame(
        &mut client_ws,
        &ClientFrame::Subscribe {
            topics: vec![format!("agent_session:{}", output.session_id)],
            resume_after: None,
            client_request_id: Some("host-ingest-live-sub".into()),
        },
    )
    .await?;
    match recv_server_frame(&mut client_ws).await? {
        ServerFrame::SubscribeAck { .. } => {}
        other => panic!("expected SubscribeAck, got {other:?}"),
    }
    // Drain durable replay of agent_session_started.
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
    host_subscribe_for_catchup(&mut host_ws, host_id).await?;
    // Drain host command durable replay.
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::DurableEvent { .. } => {}
        other => panic!("expected host DurableEvent replay, got {other:?}"),
    }

    // Server translates raw Codex payloads; host projection is empty / ignored.
    // item/started opens the assistant message; delta requires that state.
    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostIngestLiveBatch {
            batch: minos_protocol::realtime::HostIngestLiveBatch {
                batch_id: "batch-live-1".into(),
                host_id,
                chunks: vec![
                    minos_protocol::realtime::HostIngestChunk {
                        event_id: format!("{host_id}:{}:1", output.session_id),
                        session_id: output.session_id.clone(),
                        seq: 1,
                        agent: minos_domain::AgentName::Codex,
                        kind: "agent_event".into(),
                        payload: serde_json::json!({
                            "method": "item/started",
                            "params": {
                                "item": {
                                    "type": "agentMessage",
                                    "id": "msg-live-1"
                                }
                            }
                        }),
                        conversation_id: Some(conversation.conversation_id.clone()),
                        projection: vec![],
                        first_ts_ms: 2_000,
                        last_ts_ms: 2_000,
                        byte_len: 64,
                        checksum_sha256: "a".repeat(64),
                    },
                    minos_protocol::realtime::HostIngestChunk {
                        event_id: format!("{host_id}:{}:2", output.session_id),
                        session_id: output.session_id.clone(),
                        seq: 2,
                        agent: minos_domain::AgentName::Codex,
                        kind: "agent_event".into(),
                        payload: serde_json::json!({
                            "method": "item/agentMessage/delta",
                            "params": {
                                "itemId": "msg-live-1",
                                "delta": "hello remote"
                            }
                        }),
                        conversation_id: Some(conversation.conversation_id.clone()),
                        projection: vec![],
                        first_ts_ms: 2_001,
                        last_ts_ms: 2_001,
                        byte_len: 48,
                        checksum_sha256: "b".repeat(64),
                    },
                ],
            },
        },
    )
    .await?;

    // One ack per accepted session (max seq across chunks in batch).
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::HostIngestAck {
            session_id,
            accepted_to_seq,
            batch_id,
        } => {
            assert_eq!(session_id, output.session_id);
            assert_eq!(accepted_to_seq, 2);
            assert_eq!(batch_id.as_deref(), Some("batch-live-1"));
        }
        other => panic!("expected HostIngestAck, got {other:?}"),
    }

    // Client receives server-translated UI events (not host-supplied projection).
    let mut kinds = Vec::new();
    for _ in 0..2 {
        match recv_server_frame(&mut client_ws).await? {
            ServerFrame::StreamEvent {
                topic,
                kind,
                seq,
                payload,
            } => {
                assert_eq!(topic, format!("agent_session:{}", output.session_id));
                assert_eq!(kind, "ui_event");
                assert!(seq == Some(1) || seq == Some(2));
                kinds.push(payload["kind"].as_str().unwrap_or_default().to_string());
            }
            other => panic!("expected StreamEvent fanout, got {other:?}"),
        }
    }
    assert!(
        kinds.iter().any(|k| k == "message_started"),
        "kinds={kinds:?}"
    );
    assert!(kinds.iter().any(|k| k == "text_delta"), "kinds={kinds:?}");

    let session = store::agent_sessions::get(&relay.pool, &output.session_id)
        .await?
        .expect("session row");
    assert_eq!(session.status, "running");

    Ok(())
}

/// Golden path: approval/request inside HostIngestLiveBatch is recorded so
/// remote `/v1/approvals/respond` can resolve it.
#[tokio::test]
async fn host_ingest_live_batch_records_approval_request() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) =
        seed_client_account(&relay, "ws-host-ingest-approval@example.com").await?;
    let host_id = seed_host(&relay).await?;
    let members = vec![account_id.clone()];
    let conversation = store::social::create_group_conversation(
        &relay.pool,
        &account_id,
        "Host Ingest Approval",
        &members,
        100,
    )
    .await?;
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

    let output = seed_session(
        &relay,
        &account_id,
        host_id,
        &conversation.conversation_id,
        "host-ingest-approval-seed",
        Some("seed"),
    )
    .await?;

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut host_ws = connect_client(&relay, "/ws/host", &host_ticket).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    host_subscribe_for_catchup(&mut host_ws, host_id).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::DurableEvent { .. } => {}
        other => panic!("expected host DurableEvent replay, got {other:?}"),
    }

    let request_id = "perm-live-1";
    // Use wall-clock ts so the approval timeout poller does not immediately
    // expire a synthetic epoch timestamp.
    let now_ms = chrono::Utc::now().timestamp_millis();
    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostIngestLiveBatch {
            batch: minos_protocol::realtime::HostIngestLiveBatch {
                batch_id: "batch-approval-1".into(),
                host_id,
                chunks: vec![minos_protocol::realtime::HostIngestChunk {
                    event_id: format!("{host_id}:{}:1", output.session_id),
                    session_id: output.session_id.clone(),
                    seq: 1,
                    agent: minos_domain::AgentName::Codex,
                    kind: "agent_event".into(),
                    payload: serde_json::json!({
                        "method": "approval/request",
                        "params": {
                            "request_id": request_id,
                            "turn_id": "turn-approve-1",
                            "method": "session/request_permission",
                            "params": { "toolCall": { "title": "ls" } },
                            // 0 = no host/backend auto-timeout (wait for user).
                            "timeout_ms": 0
                        }
                    }),
                    conversation_id: Some(conversation.conversation_id.clone()),
                    // Host projection ignored; approvals come from raw payload.
                    projection: vec![],
                    first_ts_ms: now_ms,
                    last_ts_ms: now_ms,
                    byte_len: 64,
                    checksum_sha256: "b".repeat(64),
                }],
            },
        },
    )
    .await?;

    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::HostIngestAck {
            accepted_to_seq, ..
        } => {
            assert_eq!(accepted_to_seq, 1);
        }
        other => panic!("expected HostIngestAck, got {other:?}"),
    }

    // Allow async approval side effects to settle.
    let row = wait_for_approval_row(&relay, request_id).await?;
    assert_eq!(row.agent_session_id, output.session_id);
    assert_eq!(
        row.state,
        store::approval_requests::ApprovalRequestState::Pending
    );
    assert_eq!(row.host_device_id, host_id);

    Ok(())
}

/// Host-local session id (not cloud-started) is auto-registered when host is Linked.
#[tokio::test]
async fn host_ingest_auto_registers_unknown_session_when_linked() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (account_id, phone_id) =
        seed_client_account(&relay, "ws-host-ingest-autoreg@example.com").await?;
    let host_id = seed_host(&relay).await?;
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

    let local_session_id = "57776df1-db66-4e7d-a2db-177a11f53c20";
    let local_conversation_id = "desktop-local-conv-1";

    let host_ticket = issue_host_ws_ticket(&relay, host_id).await?;
    let mut host_ws = connect_client(&relay, "/ws/host", &host_ticket).await?;
    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut host_ws).await?;

    send_client_frame(
        &mut host_ws,
        &ClientFrame::HostIngestLiveBatch {
            batch: minos_protocol::realtime::HostIngestLiveBatch {
                batch_id: "batch-autoreg-1".into(),
                host_id,
                chunks: vec![minos_protocol::realtime::HostIngestChunk {
                    event_id: format!("{host_id}:{local_session_id}:1"),
                    session_id: local_session_id.into(),
                    seq: 1,
                    agent: minos_domain::AgentName::Codex,
                    kind: "agent_event".into(),
                    payload: serde_json::json!({
                        "method": "item/started",
                        "params": {
                            "item": { "type": "agentMessage", "id": "msg-auto-1" }
                        }
                    }),
                    conversation_id: Some(local_conversation_id.into()),
                    projection: vec![],
                    first_ts_ms: 3_000,
                    last_ts_ms: 3_000,
                    byte_len: 80,
                    checksum_sha256: "c".repeat(64),
                }],
            },
        },
    )
    .await?;

    match recv_server_frame(&mut host_ws).await? {
        ServerFrame::HostIngestAck {
            session_id,
            accepted_to_seq,
            ..
        } => {
            assert_eq!(session_id, local_session_id);
            assert_eq!(accepted_to_seq, 1);
        }
        other => panic!("expected HostIngestAck after auto-register, got {other:?}"),
    }

    let session = store::agent_sessions::get(&relay.pool, local_session_id)
        .await?
        .expect("formal session auto-registered");
    assert_eq!(session.conversation_id, local_conversation_id);
    assert_eq!(
        session.host_device_id.as_deref(),
        Some(host_id.to_string().as_str())
    );
    assert_eq!(session.status, "running");

    Ok(())
}

async fn wait_for_approval_row(
    relay: &Relay,
    request_id: &str,
) -> anyhow::Result<store::approval_requests::ApprovalRequestRow> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(row) = store::approval_requests::get(&relay.pool, request_id).await? {
            return Ok(row);
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for approval_requests row {request_id}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
