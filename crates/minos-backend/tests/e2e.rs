//! End-to-end integration tests for the Minos backend's WebSocket
//! lifecycle.
//!
//! Spawns a real axum server on an ephemeral port with a `tempfile`-backed
//! SQLite DB, drives it with raw `tokio-tungstenite` clients, and exercises
//! the parts of the WS contract that survive after the LocalRpc dispatcher
//! has been retired (HTTP `/v1/*` routes now own the pairing + threads
//! surface; see `tests/v1_pairing.rs` and `tests/v1_threads.rs`).
//!
//! # Test layout
//!
//! 1. `e2e_reconnect_with_wrong_secret_returns_401` — a device row exists
//!    with a known secret hash; reconnecting with a bogus secret is rejected
//!    pre-upgrade with HTTP 401 (see `src/http/ws_devices.rs` module
//!    header).
//! 2. `e2e_reconnect_supersedes_old_socket` — a second authenticated socket
//!    for the same `DeviceId` actively revokes the first, and the
//!    replacement keeps serving traffic (verified by sending a `Forward`
//!    frame and receiving a synthesised peer-offline `Forwarded` reply).
//! 3. `e2e_presence_tracks_live_peer_membership` — paired devices observe
//!    `Event::PeerOnline` / `Event::PeerOffline` on each other's connect
//!    and disconnect.

#![allow(clippy::too_many_lines)]

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    auth::jwt,
    http::{router, BackendState},
    pairing::{secret::hash_secret, PairingService},
    session::SessionRegistry,
    store,
};

/// Fixed JWT secret used by the test relay; mirrors `test_support::TEST_JWT_SECRET`.
const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";
use minos_domain::{DeviceId, DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind};
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    tungstenite::{client::ClientRequestBuilder, http::Uri, protocol::Message, Error as WsError},
    MaybeTlsStream, WebSocketStream,
};

/// Short timeout for individual `recv` calls. Sized for slow shared CI
/// runners; local runs complete well under the bound.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_TOKEN_TTL: Duration = Duration::from_mins(5);

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ── relay harness ────────────────────────────────────────────────────────

struct Relay {
    addr: SocketAddr,
    pool: SqlitePool,
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

    let state = BackendState {
        registry: Arc::new(SessionRegistry::new()),
        pairing: Arc::new(PairingService::new(pool.clone())),
        store: pool.clone(),
        token_ttl: DEFAULT_TOKEN_TTL,
        translators: minos_backend::ingest::translate::ThreadTranslators::new(),
        public_cfg: Arc::new(minos_backend::http::BackendPublicConfig {
            public_url: "ws://127.0.0.1:8787/devices".into(),
            cf_access_client_id: None,
            cf_access_client_secret: None,
        }),
        jwt_secret: Arc::new(TEST_JWT_SECRET.to_string()),
        auth_login_per_email: minos_backend::http::default_login_per_email(),
        auth_login_per_ip: minos_backend::http::default_login_per_ip(),
        auth_register_per_ip: minos_backend::http::default_register_per_ip(),
        auth_refresh_per_acc: minos_backend::http::default_refresh_per_acc(),
        version: "e2e-test",
    };
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        pool,
        _db_file: tmp,
        _db_path: tmp_path,
        task,
    })
}

// ── client helpers ───────────────────────────────────────────────────────

async fn connect_client(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    secret: Option<&str>,
    name: Option<&str>,
) -> Result<WsClient, WsError> {
    // Phase 2 Task 2.2: iOS upgrades require a bearer JWT. The e2e tests
    // pre-date the account model so they don't have a "real" account to
    // log in as — synthesise a token bound to the same `device_id` here so
    // the existing scenarios still exercise the post-upgrade behaviour.
    let token = (role == DeviceRole::IosClient).then(|| {
        jwt::sign(TEST_JWT_SECRET.as_bytes(), "e2e-acct", &device_id.to_string())
            .expect("test bearer signs cleanly")
    });
    connect_client_with_bearer(relay, device_id, role, secret, name, token.as_deref()).await
}

async fn connect_client_with_bearer(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    secret: Option<&str>,
    name: Option<&str>,
    bearer: Option<&str>,
) -> Result<WsClient, WsError> {
    let url: Uri = format!("ws://{}/devices", relay.addr).parse().unwrap();
    let mut builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", device_id.to_string())
        .with_header("X-Device-Role", role.to_string());
    if let Some(s) = secret {
        builder = builder.with_header("X-Device-Secret", s.to_string());
    }
    if let Some(n) = name {
        builder = builder.with_header("X-Device-Name", n.to_string());
    }
    if let Some(t) = bearer {
        builder = builder.with_header("Authorization", format!("Bearer {t}"));
    }
    let (ws, _resp) = tokio_tungstenite::connect_async(builder).await?;
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

async fn send_envelope(ws: &mut WsClient, env: &Envelope) -> anyhow::Result<()> {
    let text = serde_json::to_string(env)?;
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

async fn expect_unpaired_event(ws: &mut WsClient) -> anyhow::Result<()> {
    match recv_envelope(ws).await? {
        Envelope::Event {
            event: EventKind::Unpaired,
            version: 1,
        } => Ok(()),
        other => Err(anyhow::anyhow!(
            "expected Event::Unpaired as first frame, got {other:?}"
        )),
    }
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_reconnect_with_wrong_secret_returns_401() -> anyhow::Result<()> {
    // Spec §10.3 reserves WS close 4401 for auth failure, but step 9
    // rejects bad creds PRE-UPGRADE with HTTP 401 to avoid a wasted WS
    // round trip (see `src/http/ws_devices.rs` module header). That's the
    // semantically-equivalent contract this test asserts.
    let relay = spawn_relay().await?;

    // Seed a device row with a known secret hash (bypass the first-connect
    // flow — we want the reconnect path where a hash is already on file).
    let id = DeviceId::new();
    let good = DeviceSecret::generate();
    let good_hash = hash_secret(&good)?;
    store::devices::insert_device(&relay.pool, id, "seeded", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, id, &good_hash).await?;

    let err = connect_client(
        &relay,
        id,
        DeviceRole::IosClient,
        Some("definitely-not-the-right-secret"),
        None,
    )
    .await
    .expect_err("wrong secret must be rejected at handshake");

    match err {
        WsError::Http(resp) => assert_eq!(resp.status().as_u16(), 401, "expected HTTP 401"),
        other => panic!("expected WsError::Http(401), got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn e2e_reconnect_supersedes_old_socket() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let id = DeviceId::new();
    let secret = DeviceSecret::generate();
    let secret_hash = hash_secret(&secret)?;
    store::devices::insert_device(&relay.pool, id, "ios", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, id, &secret_hash).await?;

    let mut first = connect_client(
        &relay,
        id,
        DeviceRole::IosClient,
        Some(secret.as_str()),
        Some("ios"),
    )
    .await?;
    expect_unpaired_event(&mut first).await?;

    let mut second = connect_client(
        &relay,
        id,
        DeviceRole::IosClient,
        Some(secret.as_str()),
        Some("ios"),
    )
    .await?;
    expect_unpaired_event(&mut second).await?;

    expect_close_frame(&mut first).await?;

    // Confirm the replacement socket is still alive by sending a `Forward`
    // frame from the unpaired session and asserting the relay synthesises
    // the spec §7.3 peer-offline JSON-RPC error back over the same socket.
    let payload_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "list_clis",
        "id": 7,
    });
    send_envelope(
        &mut second,
        &Envelope::Forward {
            version: 1,
            payload: payload_req.clone(),
        },
    )
    .await?;
    match recv_envelope(&mut second).await? {
        Envelope::Forwarded { from, payload, .. } => {
            assert_eq!(from, id, "synthesised Forwarded should name the sender");
            assert_eq!(payload["error"]["code"], -32001);
            assert_eq!(payload["error"]["message"], "peer offline");
            assert_eq!(payload["id"], 7);
        }
        other => panic!("expected synthesised Forwarded on replacement socket, got {other:?}"),
    }

    second.send(Message::Close(None)).await.ok();
    drop(second);

    Ok(())
}

#[tokio::test]
async fn e2e_presence_tracks_live_peer_membership() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let mac_id = DeviceId::new();
    let ios_id = DeviceId::new();
    let mac_secret = DeviceSecret::generate();
    let ios_secret = DeviceSecret::generate();
    let mac_hash = hash_secret(&mac_secret)?;
    let ios_hash = hash_secret(&ios_secret)?;

    store::devices::insert_device(&relay.pool, mac_id, "mac", DeviceRole::AgentHost, 0).await?;
    store::devices::insert_device(&relay.pool, ios_id, "ios", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, mac_id, &mac_hash).await?;
    store::devices::upsert_secret_hash(&relay.pool, ios_id, &ios_hash).await?;
    store::pairings::insert_pairing(&relay.pool, mac_id, ios_id, 0).await?;

    let mut mac = connect_client(
        &relay,
        mac_id,
        DeviceRole::AgentHost,
        Some(mac_secret.as_str()),
        Some("mac"),
    )
    .await?;
    match recv_envelope(&mut mac).await? {
        Envelope::Event {
            event: EventKind::PeerOffline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected initial PeerOffline on mac, got {other:?}"),
    }

    let mut ios = connect_client(
        &relay,
        ios_id,
        DeviceRole::IosClient,
        Some(ios_secret.as_str()),
        Some("ios"),
    )
    .await?;
    match recv_envelope(&mut ios).await? {
        Envelope::Event {
            event: EventKind::PeerOnline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, mac_id),
        other => panic!("expected initial PeerOnline on ios, got {other:?}"),
    }

    match recv_envelope(&mut mac).await? {
        Envelope::Event {
            event: EventKind::PeerOnline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected PeerOnline on mac after ios connect, got {other:?}"),
    }

    ios.send(Message::Close(None)).await.ok();
    drop(ios);

    match recv_envelope(&mut mac).await? {
        Envelope::Event {
            event: EventKind::PeerOffline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected PeerOffline on mac after ios disconnect, got {other:?}"),
    }

    mac.send(Message::Close(None)).await.ok();
    drop(mac);

    Ok(())
}
