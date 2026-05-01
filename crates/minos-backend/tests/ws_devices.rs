//! `/v1/devices/ws` upgrade-time behaviour: the agent-host receives an
//! `Event::IngestCheckpoint` frame as the second server frame (after the
//! initial `Event::Unpaired` Phase G presence stub) so the daemon can
//! reconcile its local DB watermark against what the backend has durably
//! persisted (Phase D / spec §9 reconciliation).
//!
//! Mobile clients ingest no events and MUST NOT receive a checkpoint.

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    auth::jwt,
    http::{router, BackendState},
    ingest::translate::ThreadTranslators,
    pairing::{secret::hash_secret, PairingService},
    session::SessionRegistry,
    store,
};
use minos_domain::{AgentName, DeviceId, DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind};
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    tungstenite::{client::ClientRequestBuilder, http::Uri, protocol::Message, Error as WsError},
    MaybeTlsStream, WebSocketStream,
};

const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

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
        token_ttl: Duration::from_mins(5),
        translators: ThreadTranslators::new(),
        jwt_secret: Arc::new(TEST_JWT_SECRET.to_string()),
        auth_login_per_email: minos_backend::http::default_login_per_email(),
        auth_login_per_ip: minos_backend::http::default_login_per_ip(),
        auth_register_per_ip: minos_backend::http::default_register_per_ip(),
        auth_refresh_per_acc: minos_backend::http::default_refresh_per_acc(),
        version: "ws-devices-test",
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

async fn connect_devices_ws(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    secret: Option<&str>,
    account_id: Option<&str>,
) -> Result<WsClient, WsError> {
    let url: Uri = format!("ws://{}/devices", relay.addr).parse().unwrap();
    let mut builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", device_id.to_string())
        .with_header("X-Device-Role", role.to_string());
    if let Some(s) = secret {
        builder = builder.with_header("X-Device-Secret", s.to_string());
    }
    if role == DeviceRole::MobileClient {
        let acct = account_id.expect("iOS connect requires an account_id");
        let token = jwt::sign(TEST_JWT_SECRET.as_bytes(), acct, &device_id.to_string())
            .expect("test bearer signs cleanly");
        builder = builder.with_header("Authorization", format!("Bearer {token}"));
    }
    let (ws, _resp) = tokio_tungstenite::connect_async(builder).await?;
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

/// Pre-seed an authenticated agent-host row + return its `(DeviceId, secret)`.
async fn register_agent_host(pool: &SqlitePool) -> (DeviceId, DeviceSecret) {
    let host_id = DeviceId::new();
    let secret = DeviceSecret::generate();
    let hash = hash_secret(&secret).unwrap();
    store::devices::insert_device(pool, host_id, "mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(pool, host_id, &hash)
        .await
        .unwrap();
    (host_id, secret)
}

#[tokio::test]
async fn devices_ws_emits_checkpoint_after_unpaired_for_agent_host() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (host_id, host_secret) = register_agent_host(&relay.pool).await;

    // Seed two threads owned by `host_id` and a few raw events on each so
    // `last_seq_per_owner` returns `{thr_1: 7, thr_2: 3}`.
    minos_backend::store::threads::upsert(
        &relay.pool,
        "thr_1",
        AgentName::Codex,
        &host_id.to_string(),
        0,
    )
    .await?;
    minos_backend::store::threads::upsert(
        &relay.pool,
        "thr_2",
        AgentName::Codex,
        &host_id.to_string(),
        0,
    )
    .await?;
    minos_backend::store::raw_events::insert_if_absent(
        &relay.pool,
        "thr_1",
        7,
        AgentName::Codex,
        &serde_json::json!({"method":"x"}),
        0,
    )
    .await?;
    minos_backend::store::raw_events::insert_if_absent(
        &relay.pool,
        "thr_2",
        3,
        AgentName::Codex,
        &serde_json::json!({"method":"x"}),
        0,
    )
    .await?;

    let mut ws = connect_devices_ws(
        &relay,
        host_id,
        DeviceRole::AgentHost,
        Some(host_secret.as_str()),
        None,
    )
    .await?;

    // First server frame: the Phase G activation stub `Unpaired`.
    match recv_envelope(&mut ws).await? {
        Envelope::Event {
            event: EventKind::Unpaired,
            ..
        } => {}
        other => panic!("expected initial Unpaired, got {other:?}"),
    }

    // Second frame: IngestCheckpoint with the two seeded thread maxes.
    match recv_envelope(&mut ws).await? {
        Envelope::Event {
            version: 1,
            event: EventKind::IngestCheckpoint {
                last_seq_per_thread,
            },
        } => {
            assert_eq!(last_seq_per_thread.get("thr_1").copied(), Some(7));
            assert_eq!(last_seq_per_thread.get("thr_2").copied(), Some(3));
            assert_eq!(last_seq_per_thread.len(), 2);
        }
        other => panic!("expected IngestCheckpoint, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn devices_ws_emits_empty_checkpoint_when_no_threads() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let (host_id, host_secret) = register_agent_host(&relay.pool).await;

    let mut ws = connect_devices_ws(
        &relay,
        host_id,
        DeviceRole::AgentHost,
        Some(host_secret.as_str()),
        None,
    )
    .await?;

    let _ = recv_envelope(&mut ws).await?; // Unpaired
    match recv_envelope(&mut ws).await? {
        Envelope::Event {
            event: EventKind::IngestCheckpoint {
                last_seq_per_thread,
            },
            ..
        } => assert!(last_seq_per_thread.is_empty()),
        other => panic!("expected empty IngestCheckpoint, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn devices_ws_does_not_emit_checkpoint_for_mobile_client() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    // Seed an authenticated mobile client (account-bound, no secret hash).
    let account_id = store::accounts::create(&relay.pool, "ws-devices@example.com", "phc")
        .await?
        .account_id;
    let phone_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;

    let mut ws = connect_devices_ws(
        &relay,
        phone_id,
        DeviceRole::MobileClient,
        None,
        Some(&account_id),
    )
    .await?;

    // First (and only) presence frame is `Unpaired`. Anything that smells
    // of `IngestCheckpoint` here is a regression: mobile clients ingest
    // no events.
    match recv_envelope(&mut ws).await? {
        Envelope::Event {
            event: EventKind::Unpaired,
            ..
        } => {}
        other => panic!("expected initial Unpaired, got {other:?}"),
    }

    // A second frame is allowed (e.g. a presence event) but it must NOT be
    // an `IngestCheckpoint`. We tolerate a brief idle window.
    match timeout(Duration::from_millis(200), recv_envelope(&mut ws)).await {
        Err(_) => {
            // Idle within the window → mobile got no checkpoint, as required.
        }
        Ok(Ok(env)) => {
            if let Envelope::Event {
                event: EventKind::IngestCheckpoint { .. },
                ..
            } = env
            {
                panic!("mobile client must not receive IngestCheckpoint")
            }
        }
        Ok(Err(e)) => return Err(e),
    }

    Ok(())
}
