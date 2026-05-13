//! Relay-backed daemon end-to-end coverage.
//!
//! Drives a real in-process backend, a real `DaemonHandle`, and a simulated
//! mobile client over `/devices` so the host path is covered end-to-end:
//!
//! 1. Host connects to the backend relay.
//! 2. Host mints a pairing QR via `DaemonHandle::pairing_qr()`.
//! 3. Mobile consumes the token through `POST /v1/pairing/consume`.
//! 4. Mobile opens `/devices` and sends `minos_list_clis` to the host.
//! 5. Host responds over the relay with a JSON-RPC result payload.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use http_body_util::BodyExt as _;
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
use minos_protocol::Envelope;
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

const TEST_JWT_SECRET: &str = "daemon-e2e-jwt-secret-32-bytes";
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

type WsClient = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

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

struct MinosHomeGuard {
    previous: Option<std::ffi::OsString>,
}

impl MinosHomeGuard {
    fn install(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("MINOS_HOME");
        std::env::set_var("MINOS_HOME", path);
        Self { previous }
    }
}

impl Drop for MinosHomeGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var("MINOS_HOME", previous);
        } else {
            std::env::remove_var("MINOS_HOME");
        }
    }
}

async fn spawn_relay() -> anyhow::Result<Relay> {
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let db_url = format!("sqlite://{}?mode=rwc", tmp_path.display());
    let pool = store::connect(&db_url).await?;

    let registry = Arc::new(SessionRegistry::new());
    let state = BackendState {
        registry: registry.clone(),
        pairing: Arc::new(PairingService::new(pool.clone())),
        store: pool.clone(),
        token_ttl: Duration::from_mins(5),
        translators: ThreadTranslators::new(),
        approval_relay: minos_backend::approval_relay::ApprovalRelay::new(
            pool.clone(),
            registry.clone(),
        ),
        jwt_secret: Arc::new(TEST_JWT_SECRET.to_string()),
        auth_login_per_email: minos_backend::http::default_login_per_email(),
        auth_login_per_ip: minos_backend::http::default_login_per_ip(),
        auth_register_per_ip: minos_backend::http::default_register_per_ip(),
        auth_refresh_per_acc: minos_backend::http::default_refresh_per_acc(),
        cors_origins: None,
        version: "daemon-e2e",
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

#[tokio::test(flavor = "multi_thread")]
async fn pair_and_list_clis_over_relay() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let minos_home = TempDir::new()?;
    let _home_guard = MinosHomeGuard::install(minos_home.path());

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
        store::accounts::create(&relay.state.store, "relay-e2e@example.com", "phc").await?;
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
    assert_eq!(body["peer_name"], "Test Mac");

    wait_for_paired_peer(&handle).await?;

    let mut mobile = connect_mobile(&relay, mobile_id, &bearer).await?;
    let _ = recv_envelope(&mut mobile).await?; // initial Unpaired activation frame

    let request = Envelope::Forward {
        version: 1,
        target_device_id: host_id,
        payload: json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "minos_list_clis",
            "params": {},
        }),
    };
    mobile
        .send(Message::Text(serde_json::to_string(&request)?.into()))
        .await?;

    let response = recv_envelope(&mut mobile).await?;
    match response {
        Envelope::Forwarded { from, payload, .. } => {
            assert_eq!(from, host_id);
            let result = payload["result"]
                .as_array()
                .expect("list_clis should reply with an array result");
            assert_eq!(result.len(), 3);
            let names: Vec<&str> = result
                .iter()
                .filter_map(|entry| entry.get("name").and_then(serde_json::Value::as_str))
                .collect();
            assert!(names.contains(&"codex"));
            assert!(names.contains(&"claude"));
            assert!(names.contains(&"gemini"));
        }
        other => panic!("expected Forwarded response from host, got {other:?}"),
    }

    mobile.send(Message::Close(None)).await.ok();
    handle.stop().await?;
    Ok(())
}
