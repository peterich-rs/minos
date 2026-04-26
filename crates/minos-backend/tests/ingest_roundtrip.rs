//! End-to-end ingest roundtrip: agent-host sends `Envelope::Ingest` → backend
//! persists + translates + fans out → mobile peer receives
//! `Envelope::Event { UiEventMessage }`.
//!
//! The test pre-seeds a paired (agent-host, ios-client) pair directly in the
//! DB with known `DeviceSecret`s so we skip the full pairing dance. It then
//! opens two live WS connections and drives one ingest frame through the
//! backend.

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    http::{router, BackendState},
    ingest::translate::ThreadTranslators,
    pairing::{secret::hash_secret, PairingService},
    session::SessionRegistry,
    store,
};
use minos_domain::{AgentName, DeviceId, DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind};
use minos_ui_protocol::UiEventMessage;
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    tungstenite::{client::ClientRequestBuilder, http::Uri, protocol::Message, Error as WsError},
    MaybeTlsStream, WebSocketStream,
};

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
        public_cfg: Arc::new(minos_backend::http::BackendPublicConfig {
            public_url: "ws://127.0.0.1:8787/devices".into(),
            cf_access_client_id: None,
            cf_access_client_secret: None,
        }),
        jwt_secret: Arc::new("test-jwt-secret-32-bytes-padding".to_string()),
        version: "ingest-roundtrip-test",
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

async fn connect_client(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    secret: Option<&str>,
    name: Option<&str>,
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
                        thread_id, seq, ui, ..
                    },
                ..
            } => return Ok((thread_id, seq, ui)),
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
async fn ingest_translates_and_fans_out_to_paired_mobile() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    // Pre-seed: two devices, both paired, each with a hashed secret.
    let host_id = DeviceId::new();
    let phone_id = DeviceId::new();
    let host_secret = DeviceSecret::generate();
    let phone_secret = DeviceSecret::generate();
    let host_hash = hash_secret(&host_secret)?;
    let phone_hash = hash_secret(&phone_secret)?;

    store::devices::insert_device(&relay.pool, host_id, "host", DeviceRole::AgentHost, 0).await?;
    store::devices::insert_device(&relay.pool, phone_id, "phone", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, host_id, &host_hash).await?;
    store::devices::upsert_secret_hash(&relay.pool, phone_id, &phone_hash).await?;
    store::pairings::insert_pairing(&relay.pool, host_id, phone_id, 0).await?;

    // Phone connects first so it has a live session by the time the host
    // sends Ingest.
    let mut phone = connect_client(
        &relay,
        phone_id,
        DeviceRole::IosClient,
        Some(phone_secret.as_str()),
        Some("phone"),
    )
    .await?;

    // Drain the initial presence frame (`PeerOffline`, since host isn't live yet).
    let _initial_presence = recv_envelope(&mut phone).await?;

    let mut host = connect_client(
        &relay,
        host_id,
        DeviceRole::AgentHost,
        Some(host_secret.as_str()),
        Some("host"),
    )
    .await?;
    // Host also gets a presence frame (PeerOnline for phone) — drain it.
    let _ = recv_envelope(&mut host).await?;

    // Host pushes one Ingest frame: a codex thread/started notification.
    let ingest = Envelope::Ingest {
        version: 1,
        agent: AgentName::Codex,
        thread_id: "thr_test".into(),
        seq: 1,
        payload: serde_json::json!({
            "method":"thread/started",
            "params":{"threadId":"thr_test","createdAtMs":1}
        }),
        ts_ms: 1,
    };
    send_envelope(&mut host, &ingest).await?;

    // Phone should receive Envelope::Event with UiEventMessage::ThreadOpened.
    // (PeerOnline for the host's reconnect may arrive first; `recv_ui_event`
    // skips non-UI events.)
    let (thread_id, seq, ui) = recv_ui_event(&mut phone).await?;
    assert_eq!(thread_id, "thr_test");
    assert_eq!(seq, 1);
    match ui {
        UiEventMessage::ThreadOpened {
            thread_id, agent, ..
        } => {
            assert_eq!(thread_id, "thr_test");
            assert_eq!(agent, AgentName::Codex);
        }
        other => panic!("expected ThreadOpened, got {other:?}"),
    }

    // Verify the raw event was persisted + the thread row created.
    let raw_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM raw_events WHERE thread_id = 'thr_test'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(raw_count, 1);

    let thread_row: (String, String) =
        sqlx::query_as("SELECT thread_id, agent FROM threads WHERE thread_id = 'thr_test'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(thread_row.0, "thr_test");
    assert_eq!(thread_row.1, "codex");

    Ok(())
}

#[tokio::test]
async fn ingest_retransmit_is_no_op() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let host_id = DeviceId::new();
    let host_secret = DeviceSecret::generate();
    let host_hash = hash_secret(&host_secret)?;

    store::devices::insert_device(&relay.pool, host_id, "host", DeviceRole::AgentHost, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, host_id, &host_hash).await?;

    let mut host = connect_client(
        &relay,
        host_id,
        DeviceRole::AgentHost,
        Some(host_secret.as_str()),
        Some("host"),
    )
    .await?;
    // Drain Unpaired presence frame.
    let _ = recv_envelope(&mut host).await?;

    let ingest = Envelope::Ingest {
        version: 1,
        agent: AgentName::Codex,
        thread_id: "thr_dedup".into(),
        seq: 1,
        payload: serde_json::json!({"method":"item/plan/delta","params":{"step":"compile"}}),
        ts_ms: 1,
    };
    send_envelope(&mut host, &ingest).await?;
    send_envelope(&mut host, &ingest).await?; // duplicate

    // Give the backend a moment to process both frames.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM raw_events WHERE thread_id = 'thr_dedup'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(row_count, 1, "retransmit must be a no-op at the DB layer");

    Ok(())
}

#[tokio::test]
async fn ingest_derives_title_from_first_user_message_and_fans_out_synthetic_update(
) -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let host_id = DeviceId::new();
    let phone_id = DeviceId::new();
    let host_secret = DeviceSecret::generate();
    let phone_secret = DeviceSecret::generate();
    let host_hash = hash_secret(&host_secret)?;
    let phone_hash = hash_secret(&phone_secret)?;

    store::devices::insert_device(&relay.pool, host_id, "host", DeviceRole::AgentHost, 0).await?;
    store::devices::insert_device(&relay.pool, phone_id, "phone", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, host_id, &host_hash).await?;
    store::devices::upsert_secret_hash(&relay.pool, phone_id, &phone_hash).await?;
    store::pairings::insert_pairing(&relay.pool, host_id, phone_id, 0).await?;

    let mut phone = connect_client(
        &relay,
        phone_id,
        DeviceRole::IosClient,
        Some(phone_secret.as_str()),
        Some("phone"),
    )
    .await?;
    let _ = recv_envelope(&mut phone).await?;

    let mut host = connect_client(
        &relay,
        host_id,
        DeviceRole::AgentHost,
        Some(host_secret.as_str()),
        Some("host"),
    )
    .await?;
    let _ = recv_envelope(&mut host).await?;

    let prompt = "Explain why the mobile pair contract broke and how to fix it cleanly";
    send_envelope(
        &mut host,
        &Envelope::Ingest {
            version: 1,
            agent: AgentName::Codex,
            thread_id: "thr_title".into(),
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

    let (thread_id, seq, ui) = recv_ui_event(&mut phone).await?;
    assert_eq!(thread_id, "thr_title");
    assert_eq!(seq, 1);
    match ui {
        UiEventMessage::ThreadTitleUpdated { thread_id, title } => {
            assert_eq!(thread_id, "thr_title");
            assert_eq!(title, prompt);
        }
        other => panic!("expected ThreadTitleUpdated, got {other:?}"),
    }

    let stored_title: Option<String> =
        sqlx::query_scalar("SELECT title FROM threads WHERE thread_id = 'thr_title'")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(stored_title.as_deref(), Some(prompt));

    Ok(())
}
