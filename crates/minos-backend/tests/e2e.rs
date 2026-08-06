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
//! 3. `e2e_legacy_envelope_frame_returns_validation_error` — a mobile
//!    client sends a legacy `Envelope` payload and the topic gateway rejects
//!    it with a `validation_format` error while keeping the socket alive.
//! 4. `e2e_presence_tracks_live_peer_membership` — paired devices observe
//!    `Event::PeerOnline` / `Event::PeerOffline` on each other's connect
//!    and disconnect.

#![allow(clippy::too_many_lines)]

use minos_backend::store::test_support::insert_test_host;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    auth::use_case::AuthUseCase,
    host_link::HostLinkService,
    http::{router, BackendState},
    session::SessionRegistry,
    store,
};
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use minos_protocol::{Envelope, EventKind};
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
    let registry = Arc::new(SessionRegistry::new());
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

/// Receive the next text frame as an `Envelope`, transparently ignoring
/// any server-initiated Ping/Pong so tests see only application frames.
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

async fn send_envelope(ws: &mut WsClient, env: &Envelope) -> anyhow::Result<()> {
    let text = serde_json::to_string(env)?;
    ws.send(Message::Text(text.into())).await?;
    Ok(())
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
async fn e2e_legacy_envelope_frame_returns_validation_error() -> anyhow::Result<()> {
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

    send_envelope(
        &mut ws,
        &Envelope::Event {
            version: 1,
            event: EventKind::Unpaired,
        },
    )
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

// ADR-0020 / Phase G: single-peer presence tracking
// (`PeerOnline`/`PeerOffline` on connect/disconnect) was deleted with the
// device-keyed pairings module. The activate hook now always emits
// `Unpaired`. Multi-host account-scoped presence rebuild is deferred to
// Phase M.
#[tokio::test]
#[ignore = "ADR-0020 single-peer presence model removed; Phase M will reintroduce multi-host coverage"]
async fn e2e_presence_tracks_live_peer_membership() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let mac_id = DeviceId::new();

    insert_test_host(&relay.pool, mac_id, "mac", 0).await;
    // ADR-0020: insert via account_host_pairings instead of legacy device-keyed
    // pairings. The body of this test still asserts presence semantics that
    // were removed in Phase G; #[ignore]'d at the test attribute.
    let account_id = store::accounts::create(&relay.pool, "presence@example.com")
        .await?
        .account_id;
    let ios_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;
    store::host_links::insert_pair(&relay.pool, mac_id, &account_id, ios_id, 0).await?;

    let mut host = connect_client(&relay, mac_id, DeviceRole::AgentHost, None).await?;
    match recv_envelope(&mut host).await? {
        Envelope::Event {
            event: EventKind::PeerOffline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected initial PeerOffline on host, got {other:?}"),
    }

    let mut ios =
        connect_client(&relay, ios_id, DeviceRole::MobileClient, Some(&account_id)).await?;
    match recv_envelope(&mut ios).await? {
        Envelope::Event {
            event: EventKind::PeerOnline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, mac_id),
        other => panic!("expected initial PeerOnline on ios, got {other:?}"),
    }

    match recv_envelope(&mut host).await? {
        Envelope::Event {
            event: EventKind::PeerOnline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected PeerOnline on host after ios connect, got {other:?}"),
    }

    ios.send(Message::Close(None)).await.ok();
    drop(ios);

    match recv_envelope(&mut host).await? {
        Envelope::Event {
            event: EventKind::PeerOffline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected PeerOffline on host after ios disconnect, got {other:?}"),
    }

    host.send(Message::Close(None)).await.ok();
    drop(host);

    Ok(())
}
