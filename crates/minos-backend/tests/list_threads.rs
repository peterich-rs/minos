//! End-to-end integration test for the history read path.
//!
//! Seeds a paired (agent-host, ios-client) pair directly in the DB, has the
//! host push a handful of ingest frames across 3 threads, then has the
//! phone call `list_threads` + `read_thread` via LocalRpc and asserts the
//! responses match the ingested data.
//!
//! Exercises the whole backend surface end-to-end:
//!   WS upgrade → envelope dispatch → ingest::dispatch
//!                                   → threads + raw_events stores
//!                                   → list_threads / read_thread LocalRpcs
//!                                   → fresh-state CodexTranslatorState

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
use minos_protocol::{Envelope, LocalRpcMethod, LocalRpcOutcome};
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
        version: "list-threads-test",
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

/// Wait for a `LocalRpcResponse` with matching `id`, swallowing any
/// intervening `Envelope::Event` frames (presence / UI events).
async fn recv_local_rpc_response(ws: &mut WsClient, id: u64) -> anyhow::Result<LocalRpcOutcome> {
    loop {
        match recv_envelope(ws).await? {
            Envelope::LocalRpcResponse {
                id: got_id,
                outcome,
                ..
            } if got_id == id => return Ok(outcome),
            Envelope::LocalRpcResponse { id: got_id, .. } => {
                return Err(anyhow::anyhow!(
                    "unexpected LocalRpcResponse id {got_id}, wanted {id}"
                ));
            }
            Envelope::Event { .. } => {
                // Presence or UI event during list/read; keep draining.
            }
            other => {
                return Err(anyhow::anyhow!("expected LocalRpcResponse, got {other:?}"));
            }
        }
    }
}

#[allow(clippy::too_many_lines)] // Single flow; splitting would obscure the happy-path narrative.
#[tokio::test]
async fn list_threads_and_read_thread_roundtrip() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    // Pre-seed a paired host + phone.
    let host_id = DeviceId::new();
    let phone_id = DeviceId::new();
    let host_secret = DeviceSecret::generate();
    let phone_secret = DeviceSecret::generate();
    store::devices::insert_device(&relay.pool, host_id, "host", DeviceRole::AgentHost, 0).await?;
    store::devices::insert_device(&relay.pool, phone_id, "phone", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, host_id, &hash_secret(&host_secret)?).await?;
    store::devices::upsert_secret_hash(&relay.pool, phone_id, &hash_secret(&phone_secret)?).await?;
    store::pairings::insert_pairing(&relay.pool, host_id, phone_id, 0).await?;

    // Connect both sides.
    let mut phone = connect_client(
        &relay,
        phone_id,
        DeviceRole::IosClient,
        Some(phone_secret.as_str()),
        Some("phone"),
    )
    .await?;
    let _ = recv_envelope(&mut phone).await?; // PeerOffline (host not yet live)

    let mut host = connect_client(
        &relay,
        host_id,
        DeviceRole::AgentHost,
        Some(host_secret.as_str()),
        Some("host"),
    )
    .await?;
    let _ = recv_envelope(&mut host).await?; // PeerOnline (phone is live)

    // Host ingests frames across three threads with distinct timestamps so
    // list_threads ordering is observable.
    for (thread_id, ts_base) in [
        ("thr_first", 1_000_i64),
        ("thr_second", 2_000_i64),
        ("thr_third", 3_000_i64),
    ] {
        // thread/started
        send_envelope(
            &mut host,
            &Envelope::Ingest {
                version: 1,
                agent: AgentName::Codex,
                thread_id: thread_id.into(),
                seq: 1,
                payload: serde_json::json!({
                    "method": "thread/started",
                    "params": {"threadId": thread_id, "createdAtMs": ts_base}
                }),
                ts_ms: ts_base,
            },
        )
        .await?;

        // One assistant message per thread — drives MessageStarted + TextDelta.
        send_envelope(
            &mut host,
            &Envelope::Ingest {
                version: 1,
                agent: AgentName::Codex,
                thread_id: thread_id.into(),
                seq: 2,
                payload: serde_json::json!({
                    "method": "item/started",
                    "params": {"itemId": "i1", "role": "agent", "startedAtMs": ts_base + 10}
                }),
                ts_ms: ts_base + 10,
            },
        )
        .await?;
        send_envelope(
            &mut host,
            &Envelope::Ingest {
                version: 1,
                agent: AgentName::Codex,
                thread_id: thread_id.into(),
                seq: 3,
                payload: serde_json::json!({
                    "method": "item/agentMessage/delta",
                    "params": {"itemId": "i1", "delta": format!("hi from {thread_id}")}
                }),
                ts_ms: ts_base + 20,
            },
        )
        .await?;
    }

    // Give the backend a beat to land all ingest frames + their side effects.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ── list_threads ────────────────────────────────────────────────────
    send_envelope(
        &mut phone,
        &Envelope::LocalRpc {
            version: 1,
            id: 100,
            method: LocalRpcMethod::ListThreads,
            params: serde_json::json!({"limit": 10}),
        },
    )
    .await?;

    let outcome = recv_local_rpc_response(&mut phone, 100).await?;
    let result = match outcome {
        LocalRpcOutcome::Ok { result } => result,
        other => return Err(anyhow::anyhow!("list_threads failed: {other:?}")),
    };
    let threads: minos_protocol::ListThreadsResponse = serde_json::from_value(result)?;
    assert_eq!(threads.threads.len(), 3, "expected 3 threads");
    // Ordered by last_ts_ms DESC.
    assert_eq!(threads.threads[0].thread_id, "thr_third");
    assert_eq!(threads.threads[1].thread_id, "thr_second");
    assert_eq!(threads.threads[2].thread_id, "thr_first");
    // message_count bumped once per MessageStarted → 1 per thread.
    for t in &threads.threads {
        assert_eq!(
            t.message_count, 1,
            "{}: unexpected message_count",
            t.thread_id
        );
    }

    // ── read_thread for one specific thread ─────────────────────────────
    send_envelope(
        &mut phone,
        &Envelope::LocalRpc {
            version: 1,
            id: 101,
            method: LocalRpcMethod::ReadThread,
            params: serde_json::json!({"thread_id": "thr_second", "limit": 100}),
        },
    )
    .await?;

    let outcome = recv_local_rpc_response(&mut phone, 101).await?;
    let result = match outcome {
        LocalRpcOutcome::Ok { result } => result,
        other => return Err(anyhow::anyhow!("read_thread failed: {other:?}")),
    };
    let read: minos_protocol::ReadThreadResponse = serde_json::from_value(result)?;

    // Expected sequence for one thread with the frames above:
    //   ThreadOpened → MessageStarted → TextDelta
    assert!(read.ui_events.len() >= 3);
    assert!(matches!(
        read.ui_events[0],
        UiEventMessage::ThreadOpened {
            ref thread_id,
            agent: AgentName::Codex,
            ..
        } if thread_id == "thr_second"
    ));
    assert!(matches!(
        read.ui_events[1],
        UiEventMessage::MessageStarted { .. }
    ));
    match &read.ui_events[2] {
        UiEventMessage::TextDelta { text, .. } => {
            assert_eq!(text, "hi from thr_second");
        }
        other => panic!("expected TextDelta, got {other:?}"),
    }

    // ── read_thread on unknown thread → thread_not_found ───────────────
    send_envelope(
        &mut phone,
        &Envelope::LocalRpc {
            version: 1,
            id: 102,
            method: LocalRpcMethod::ReadThread,
            params: serde_json::json!({"thread_id": "does_not_exist", "limit": 10}),
        },
    )
    .await?;

    let outcome = recv_local_rpc_response(&mut phone, 102).await?;
    match outcome {
        LocalRpcOutcome::Err { error } => {
            assert_eq!(error.code, "thread_not_found");
        }
        other => return Err(anyhow::anyhow!("expected thread_not_found, got {other:?}")),
    }

    // ── get_thread_last_seq matches the highest ingested seq ───────────
    send_envelope(
        &mut phone,
        &Envelope::LocalRpc {
            version: 1,
            id: 103,
            method: LocalRpcMethod::GetThreadLastSeq,
            params: serde_json::json!({"thread_id": "thr_first"}),
        },
    )
    .await?;
    let outcome = recv_local_rpc_response(&mut phone, 103).await?;
    let result = match outcome {
        LocalRpcOutcome::Ok { result } => result,
        other => return Err(anyhow::anyhow!("get_thread_last_seq failed: {other:?}")),
    };
    let resp: minos_protocol::GetThreadLastSeqResponse = serde_json::from_value(result)?;
    assert_eq!(resp.last_seq, 3);

    drop(host);
    drop(phone);
    Ok(())
}
