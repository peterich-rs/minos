//! Retired legacy ingest-over-WebSocket coverage.
//!
//! These tests exercised the old `Envelope::Ingest` wire path. The topic
//! gateway no longer accepts legacy envelopes, so the behavior is kept here
//! only as historical reference and is ignored in favor of the new realtime
//! gateway coverage.

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    auth::use_case::AuthUseCase,
    host_link::HostLinkService,
    http::{router, BackendState},
    session::SessionRegistry,
    store,
};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::{Envelope, EventKind};
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

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Relay {
    addr: SocketAddr,
    pool: SqlitePool,
    auth: Arc<AuthUseCase>,
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
        "ingest-roundtrip-instance".to_string(),
    );
    state.version = "ingest-roundtrip-test";
    let auth = Arc::clone(&state.auth);
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        pool,
        auth,
        _db_file: tmp,
        _db_path: tmp_path,
        task,
    })
}

fn gateway_path_for_role(role: DeviceRole) -> &'static str {
    if role.is_account_client() {
        "/ws/client"
    } else {
        assert_eq!(role, DeviceRole::AgentHost, "unsupported gateway role");
        "/ws/host"
    }
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

async fn connect_client(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    account_id: Option<&str>,
) -> anyhow::Result<WsClient> {
    let ticket = if role.is_account_client() {
        let acct = account_id.expect("account client connect requires an account_id");
        issue_client_ws_ticket(relay, acct, device_id, role).await?
    } else {
        issue_host_ws_ticket(relay, device_id).await?
    };
    let url: Uri = format!(
        "ws://{}{}?ticket={ticket}",
        relay.addr,
        gateway_path_for_role(role)
    )
    .parse()
    .unwrap();
    let (ws, _resp) = tokio_tungstenite::connect_async(url.to_string()).await?;
    Ok(ws)
}

async fn recv_envelope(ws: &mut WsClient) -> anyhow::Result<Envelope> {
    loop {
        let next = timeout(RECV_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for envelope"))?;
        match next {
            Some(Ok(Message::Text(t))) => return Ok(serde_json::from_str(&t)?),
            Some(Ok(Message::Ping(p))) => {
                ws.send(Message::Pong(p)).await?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(f))) => {
                return Err(anyhow::anyhow!("unexpected close frame: {f:?}"));
            }
            Some(Ok(other)) => return Err(anyhow::anyhow!("unexpected frame: {other:?}")),
            Some(Err(e)) => return Err(anyhow::anyhow!("ws error: {e}")),
            None => return Err(anyhow::anyhow!("stream ended unexpectedly")),
        }
    }
}

async fn send_envelope(ws: &mut WsClient, env: &Envelope) -> anyhow::Result<()> {
    let text = serde_json::to_string(env)?;
    ws.send(Message::Text(text.into())).await?;
    Ok(())
}

/// Drain `Envelope::Event` frames until one matches `UiEventMessage`, or
/// until the timeout elapses. Lets the test tolerate presence-related
/// frames (`PeerOnline`, `PeerOffline`) that the pairing path emits.
async fn recv_ui_event(ws: &mut WsClient) -> anyhow::Result<(String, u64, UiEventMessage)> {
    loop {
        match recv_envelope(ws).await? {
            Envelope::Event {
                event:
                    EventKind::UiEventMessage {
                        session_id,
                        seq,
                        ui,
                        ..
                    },
                ..
            } => return Ok((session_id, seq, ui)),
            Envelope::Event { event, .. } => {
                // Non-UI event (e.g., presence). Log and keep draining.
                tracing::debug!(
                    ?event,
                    "skipping non-UI Event while waiting for UiEventMessage"
                );
            }
            other => {
                return Err(anyhow::anyhow!(
                    "expected Envelope::Event (UiEventMessage), got {other:?}"
                ))
            }
        }
    }
}

#[tokio::test]
#[ignore = "legacy Envelope::Ingest websocket path removed from the topic gateway"]
async fn ingest_translates_and_fans_out_to_paired_mobile() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    // Pre-seed: a Mac (with a hashed secret) and a paired iOS device under
    // a real account; ADR-0020 keys fan-out off
    // `account_host_pairings(host, account)` and walks devices(account_id) to
    // find the iOS receivers, so the iOS row needs `account_id` set and no
    // secret hash.
    let host_id = DeviceId::new();
    store::device_installations::insert_device(
        &relay.pool,
        host_id,
        "mac",
        DeviceRole::AgentHost,
        0,
    )
    .await?;

    let account_id = store::accounts::create(&relay.pool, "ingest@example.com")
        .await?
        .account_id;
    let phone_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

    // Phone connects first so it has a live session by the time the host
    // sends Ingest.
    let mut phone = connect_client(
        &relay,
        phone_id,
        DeviceRole::MobileClient,
        Some(&account_id),
    )
    .await?;

    // Drain the initial Unpaired presence frame (Phase G activate hook
    // emits Unpaired on every upgrade until Phase M re-introduces
    // multi-host presence).
    let _initial_presence = recv_envelope(&mut phone).await?;

    let mut host = connect_client(&relay, host_id, DeviceRole::AgentHost, None).await?;
    // Host also gets the initial Unpaired frame — drain it.
    let _ = recv_envelope(&mut host).await?;

    // Host pushes one Ingest frame: a codex thread/started notification.
    let ingest = Envelope::Ingest {
        version: 1,
        agent: AgentName::Codex,
        session_id: "thr_test".into(),
        seq: 1,
        payload: serde_json::json!({
            "method":"thread/started",
            "params":{"sessionId":"thr_test","createdAtMs":1}
        }),
        ts_ms: 1,
    };
    send_envelope(&mut host, &ingest).await?;

    // Phone should receive Envelope::Event with UiEventMessage::SessionOpened.
    // (PeerOnline for the host's reconnect may arrive first; `recv_ui_event`
    // skips non-UI events.)
    let (session_id, seq, ui) = recv_ui_event(&mut phone).await?;
    assert_eq!(session_id, "thr_test");
    assert_eq!(seq, 1);
    match ui {
        UiEventMessage::SessionOpened {
            session_id, agent, ..
        } => {
            assert_eq!(session_id, "thr_test");
            assert_eq!(agent, AgentName::Codex);
        }
        other => panic!("expected SessionOpened, got {other:?}"),
    }

    // Verify the raw event was persisted + the session row created.
    let raw_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM raw_events WHERE session_id = 'thr_test'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(raw_count, 1);

    let thread_row: (String, String) =
        sqlx::query_as("SELECT session_id, agent FROM sessions WHERE session_id = 'thr_test'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(thread_row.0, "thr_test");
    assert_eq!(thread_row.1, "codex");

    Ok(())
}

#[tokio::test]
#[ignore = "legacy Envelope::Ingest websocket path removed from the topic gateway"]
async fn ingest_retransmit_is_no_op() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let host_id = DeviceId::new();

    store::device_installations::insert_device(
        &relay.pool,
        host_id,
        "mac",
        DeviceRole::AgentHost,
        0,
    )
    .await?;

    let mut host = connect_client(&relay, host_id, DeviceRole::AgentHost, None).await?;
    // Drain Unpaired presence frame.
    let _ = recv_envelope(&mut host).await?;

    let ingest = Envelope::Ingest {
        version: 1,
        agent: AgentName::Codex,
        session_id: "thr_dedup".into(),
        seq: 1,
        payload: serde_json::json!({"method":"item/plan/delta","params":{"step":"compile"}}),
        ts_ms: 1,
    };
    send_envelope(&mut host, &ingest).await?;
    send_envelope(&mut host, &ingest).await?; // duplicate

    // Give the backend a moment to process both frames.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM raw_events WHERE session_id = 'thr_dedup'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(row_count, 1, "retransmit must be a no-op at the DB layer");

    Ok(())
}

#[tokio::test]
#[ignore = "legacy Envelope::Ingest websocket path removed from the topic gateway"]
async fn ingest_derives_title_from_first_user_message_and_fans_out_synthetic_update(
) -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let host_id = DeviceId::new();
    store::device_installations::insert_device(
        &relay.pool,
        host_id,
        "mac",
        DeviceRole::AgentHost,
        0,
    )
    .await?;

    let account_id = store::accounts::create(&relay.pool, "title@example.com")
        .await?
        .account_id;
    let phone_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;
    store::host_links::insert_pair(&relay.pool, host_id, &account_id, phone_id, 0).await?;

    let mut phone = connect_client(
        &relay,
        phone_id,
        DeviceRole::MobileClient,
        Some(&account_id),
    )
    .await?;
    let _ = recv_envelope(&mut phone).await?;

    let mut host = connect_client(&relay, host_id, DeviceRole::AgentHost, None).await?;
    let _ = recv_envelope(&mut host).await?;

    let prompt = "Explain why the mobile pair contract broke and how to fix it cleanly";
    send_envelope(
        &mut host,
        &Envelope::Ingest {
            version: 1,
            agent: AgentName::Codex,
            session_id: "thr_title".into(),
            seq: 1,
            payload: serde_json::json!({
                "method": "item/started",
                "params": {
                    "itemId": "u1",
                    "role": "user",
                    "startedAtMs": 1,
                    "input": [{"type": "text", "text": prompt}]
                }
            }),
            ts_ms: 1,
        },
    )
    .await?;

    let (session_id, seq, ui) = recv_ui_event(&mut phone).await?;
    assert_eq!(session_id, "thr_title");
    assert_eq!(seq, 1);
    match ui {
        UiEventMessage::SessionTitleUpdated { session_id, title } => {
            assert_eq!(session_id, "thr_title");
            assert_eq!(title, prompt);
        }
        other => panic!("expected SessionTitleUpdated, got {other:?}"),
    }

    let stored_title: Option<String> =
        sqlx::query_scalar("SELECT title FROM sessions WHERE session_id = 'thr_title'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(stored_title.as_deref(), Some(prompt));

    Ok(())
}
