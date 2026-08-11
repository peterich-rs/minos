//! End-to-end integration tests for the Minos backend's WebSocket
//! lifecycle.
//!
//! Spawns a real axum server on an ephemeral port with a `tempfile`-backed
//! SQLite DB, drives it with raw `tokio-tungstenite` clients, and exercises
//! the parts of the WS contract that survive after the LocalRpc dispatcher
//! has been retired (HTTP `/v1/*` routes now own the pairing + sessions
//! surface; see `tests/v1_pairing.rs` and `tests/v1_threads.rs`).
//!
//! # Test layout
//!
//! 1. `e2e_reconnect_with_invalid_ticket_returns_401` — the formal gateway
//!    rejects a malformed ticket pre-upgrade with HTTP 401.
//! 2. `e2e_reconnect_supersedes_old_socket_records_close_reason_metric` — a
//!    second authenticated socket for the same `DeviceId` actively revokes
//!    the first, and the replacement keeps serving traffic while `/metrics`
//!    records `reason="session_superseded"`.
//! 3. `e2e_unrecognized_raw_json_frame_returns_validation_error` — a mobile
//!    client sends unrecognized raw JSON and the topic gateway rejects
//!    it with a `validation_format` error while keeping the socket alive.
//! 4. `e2e_presence_tracks_live_peer_membership` — paired devices observe
//!    `Event::PeerOnline` / `Event::PeerOffline` on each other's connect
//!    and disconnect.

#![allow(clippy::too_many_lines)]

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    auth::use_case::AuthUseCase,
    host_link::HostLinkService,
    http::{router, BackendState},
    realtime::RealtimeConnectionRegistry,
    store,
};
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    tungstenite::{http::Uri, protocol::Message, Error as WsError},
    MaybeTlsStream, WebSocketStream,
};

/// Fixed JWT secret used by the test relay; mirrors `test_support::TEST_JWT_SECRET`.
const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";

/// Short timeout for individual `recv` calls. Sized for slow shared CI
/// runners; local runs complete well under the bound.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_TOKEN_TTL: Duration = Duration::from_mins(5);

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ── relay harness ────────────────────────────────────────────────────────

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
    let registry = Arc::new(RealtimeConnectionRegistry::new());
    let mut state = BackendState::new(
        registry,
        Arc::new(HostLinkService::new(pool.clone())),
        pool.clone(),
        DEFAULT_TOKEN_TTL,
        TEST_JWT_SECRET.to_string(),
        None,
        "e2e-instance".to_string(),
    );
    state.version = "e2e-test";
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

// ── client helpers ───────────────────────────────────────────────────────

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

async fn connect_client(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    account_id: Option<&str>,
) -> anyhow::Result<WsClient> {
    if role.is_account_client() {
        let acct = account_id.expect("account client connect requires an account_id");
        let ticket = issue_client_ws_ticket(relay, acct, device_id, role).await?;
        let url: Uri = format!(
            "ws://{}{}?ticket={ticket}",
            relay.addr,
            gateway_path_for_role(role)
        )
        .parse()
        .unwrap();
        let (ws, _resp) = tokio_tungstenite::connect_async(url.to_string()).await?;
        return Ok(ws);
    }
    // Host: Bearer hit_* only.
    let hit = store::test_support::issue_test_host_token(&relay.pool, device_id, 0).await;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("ws://{}/ws/host", relay.addr);
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {hit}").parse()?);
    let (ws, _resp) = tokio_tungstenite::connect_async(request).await?;
    Ok(ws)
}


async fn recv_server_frame(ws: &mut WsClient) -> anyhow::Result<ServerFrame> {
    loop {
        let next = timeout(RECV_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for server frame"))?;
        match next {
            Some(Ok(Message::Text(t))) => {
                if let Ok(frame) = serde_json::from_str::<ServerFrame>(&t) {
                    return Ok(frame);
                }
            }
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

async fn send_client_frame(ws: &mut WsClient, frame: &ClientFrame) -> anyhow::Result<()> {
    let text = serde_json::to_string(frame)?;
    ws.send(Message::Text(text.into())).await?;
    Ok(())
}

async fn expect_close_frame(ws: &mut WsClient) -> anyhow::Result<()> {
    loop {
        let next = timeout(RECV_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for close frame"))?;
        match next {
            Some(Ok(Message::Close(_))) | None => return Ok(()),
            Some(Ok(Message::Ping(p))) => {
                ws.send(Message::Pong(p)).await?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(other)) => {
                return Err(anyhow::anyhow!(
                    "expected relay to close the socket, got {other:?}"
                ));
            }
            Some(Err(WsError::ConnectionClosed | WsError::AlreadyClosed)) => {
                return Ok(());
            }
            Some(Err(e)) => return Err(anyhow::anyhow!("ws error while waiting for close: {e}")),
        }
    }
}

async fn expect_hello_frame(ws: &mut WsClient) -> anyhow::Result<()> {
    match recv_server_frame(ws).await? {
        ServerFrame::Hello {
            heartbeat_interval_ms,
            ..
        } => {
            anyhow::ensure!(heartbeat_interval_ms == 25_000);
        }
        other => {
            return Err(anyhow::anyhow!(
                "expected Hello as first frame, got {other:?}"
            ));
        }
    }
    // Hello is register-only: gateway may immediately ack default topic live arm.
    match timeout(Duration::from_millis(100), recv_server_frame(ws)).await {
        Ok(Ok(ServerFrame::SubscribeAck { .. })) => Ok(()),
        Ok(Ok(other)) => Err(anyhow::anyhow!(
            "unexpected post-hello frame (expected default SubscribeAck or silence): {other:?}"
        )),
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(()),
    }
}

struct Response {
    status: u16,
    body: String,
}

async fn reqwest_style_get(url: &str) -> Response {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let url = url::Url::parse(url).unwrap();
    let host = url.host_str().unwrap();
    let port = url.port().unwrap();
    let path = url.path();

    let mut stream = tokio::net::TcpStream::connect((host, port)).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();

    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    Response {
        status,
        body: body.to_string(),
    }
}

async fn assert_metrics_contains(
    relay: &Relay,
    metric: &str,
    labels: &[(&str, &str)],
) -> anyhow::Result<()> {
    let response = reqwest_style_get(&format!("http://{}/metrics", relay.addr)).await;
    if response.status != 200 {
        return Err(anyhow::anyhow!(
            "expected /metrics to return 200, got {} with body {:?}",
            response.status,
            response.body
        ));
    }

    let found = response.body.lines().any(|line| {
        line.starts_with(metric)
            && labels
                .iter()
                .all(|(name, value)| line.contains(&format!("{name}=\"{value}\"")))
    });
    if found {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "missing metric {metric} with labels {:?} in /metrics body:\n{}",
            labels,
            response.body
        ))
    }
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_reconnect_with_invalid_ticket_returns_401() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let url: Uri = format!("ws://{}/ws/client?ticket=not-a-ticket", relay.addr)
        .parse()
        .unwrap();
    let err = tokio_tungstenite::connect_async(url.to_string())
        .await
        .expect_err("invalid ticket must be rejected at handshake");

    match err {
        WsError::Http(resp) => assert_eq!(resp.status().as_u16(), 401, "expected HTTP 401"),
        other => panic!("expected WsError::Http(401), got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn e2e_reconnect_supersedes_old_socket_records_close_reason_metric() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let account_id = store::accounts::create(&relay.pool, "reconnect-e2e@example.com")
        .await?
        .account_id;
    let id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;

    let mut first = connect_client(&relay, id, DeviceRole::MobileClient, Some(&account_id)).await?;
    expect_hello_frame(&mut first).await?;

    let mut second =
        connect_client(&relay, id, DeviceRole::MobileClient, Some(&account_id)).await?;
    expect_hello_frame(&mut second).await?;

    expect_close_frame(&mut first).await?;
    assert_metrics_contains(
        &relay,
        "minos_backend_ws_close_total",
        &[("role", "mobile-client"), ("reason", "session_superseded")],
    )
    .await?;

    send_client_frame(&mut second, &ClientFrame::Ping { ts: 7 }).await?;
    match recv_server_frame(&mut second).await? {
        ServerFrame::Pong { ts, .. } => assert_eq!(ts, 7),
        other => panic!("expected Pong on replacement socket, got {other:?}"),
    }

    second.send(Message::Close(None)).await.ok();
    drop(second);

    Ok(())
}

#[tokio::test]
async fn e2e_unrecognized_raw_json_frame_returns_validation_error() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let account_id = store::accounts::create(&relay.pool, "server-frame@example.com")
        .await?
        .account_id;
    let phone_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;
    let mut ws = connect_client(
        &relay,
        phone_id,
        DeviceRole::MobileClient,
        Some(&account_id),
    )
    .await?;
    expect_hello_frame(&mut ws).await?;

    // Unrecognized raw JSON (former envelope shape) must not be accepted.
    ws.send(Message::Text(
        r#"{"kind":"event","v":1,"type":"unpaired"}"#.into(),
    ))
    .await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Error { code, message, .. } => {
            assert_eq!(code, "validation_format");
            assert_eq!(message, "unrecognized websocket frame");
        }
        other => panic!("expected validation error, got {other:?}"),
    }

    send_client_frame(&mut ws, &ClientFrame::Ping { ts: 9 }).await?;
    match recv_server_frame(&mut ws).await? {
        ServerFrame::Pong { ts, .. } => assert_eq!(ts, 9),
        other => panic!("expected Pong after validation error, got {other:?}"),
    }

    Ok(())
}

// Single-peer presence tracking (`PeerOnline`/`PeerOffline` on
// connect/disconnect) was deleted with the device-keyed pairings module.
// Presence is now ephemeral StreamEvent on formal topics; multi-host coverage
// is deferred.
#[tokio::test]
#[ignore = "single-peer presence model removed; multi-host presence coverage deferred"]
async fn e2e_presence_tracks_live_peer_membership() -> anyhow::Result<()> {
    let _ = spawn_relay().await?;
    Ok(())
}
