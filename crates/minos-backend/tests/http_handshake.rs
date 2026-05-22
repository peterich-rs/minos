//! Integration tests for the HTTP surface (`/health/*` + formal realtime
//! gateway upgrades).
//!
//! Each test spawns a real axum server on an ephemeral port and drives it
//! with a real `tokio-tungstenite` client. This mirrors what the full e2e
//! (step 12) will do, but with a focused coverage of the handshake path
//! added in step 9.

use std::{collections::HashMap, sync::Arc, time::Duration};

use minos_backend::{
    auth::use_case::AuthUseCase,
    http::{router, BackendState},
    pairing::{secret::hash_secret, PairingService},
    session::SessionRegistry,
    store,
};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{
    client::ClientRequestBuilder, http::Uri, protocol::Message, Error as WsError,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Fixed JWT secret used by the test relay; mirrors `test_support::TEST_JWT_SECRET`.
const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Bring up a relay on an ephemeral port and return `(base_url, server_task)`.
///
/// The server task is a detached `tokio::spawn`; the test drops its handle
/// at end of scope, which lets `axum::serve` shut down when the tokio
/// runtime tears down.
async fn spawn_relay() -> (
    String,
    tokio::task::JoinHandle<()>,
    sqlx::SqlitePool,
    Arc<AuthUseCase>,
) {
    let pool = store::connect("sqlite::memory:").await.unwrap();
    let registry = Arc::new(SessionRegistry::new());
    let mut state = BackendState::new(
        registry,
        Arc::new(PairingService::new(pool.clone())),
        pool.clone(),
        Duration::from_mins(5),
        TEST_JWT_SECRET.to_string(),
        None,
        "test-instance".to_string(),
    );
    state.version = "test";
    let auth = Arc::clone(&state.auth);
    let app = router(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (base, handle, pool, auth)
}

fn http_to_ws(base: &str) -> String {
    base.replacen("http://", "ws://", 1)
}

async fn issue_client_ws_ticket(
    auth: &AuthUseCase,
    account_id: &str,
    device_id: DeviceId,
    role: DeviceRole,
) -> anyhow::Result<String> {
    Ok(auth
        .issue_ws_ticket(account_id, device_id, role)
        .await
        .map_err(|error| anyhow::anyhow!("issue_ws_ticket failed: {error:?}"))?
        .ticket)
}

fn issue_host_ws_ticket(auth: &AuthUseCase, host_id: DeviceId) -> anyhow::Result<String> {
    Ok(auth
        .issue_host_ws_ticket(host_id)
        .map_err(|error| anyhow::anyhow!("issue_host_ws_ticket failed: {error:?}"))?
        .ticket)
}

async fn connect_gateway_ws(base: &str, path: &str, ticket: &str) -> Result<WsClient, WsError> {
    let url: Uri = format!("{}{path}?ticket={ticket}", http_to_ws(base))
        .parse()
        .unwrap();
    let (ws, _resp) = tokio_tungstenite::connect_async(url.to_string()).await?;
    Ok(ws)
}

// ── /health/* ──────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok_with_name_and_version() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let resp = reqwest_style_get(&format!("{base}/health/ready")).await;
    assert_eq!(resp.status, 200);
    assert!(
        resp.body.contains("minos-backend"),
        "body missing crate name: {:?}",
        resp.body
    );
    assert!(
        resp.body.contains("test"),
        "body missing version: {:?}",
        resp.body
    );
}

#[tokio::test]
async fn health_includes_instance_id_and_request_id_header() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let resp = reqwest_style_get(&format!("{base}/health/ready")).await;
    assert_eq!(resp.status, 200);

    let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(body["instance_id"], "test-instance");

    let request_id = resp
        .headers
        .get("x-request-id")
        .cloned()
        .unwrap_or_default();
    assert!(
        !request_id.is_empty(),
        "health response should propagate x-request-id"
    );
}

#[tokio::test]
async fn metrics_endpoint_exposes_prometheus_text() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let _ = reqwest_style_get(&format!("{base}/health/ready")).await;

    let resp = reqwest_style_get(&format!("{base}/metrics")).await;
    assert_eq!(resp.status, 200);
    assert!(resp.body.contains("minos_backend_session_registry_size"));
    assert!(resp
        .body
        .contains("minos_backend_http_request_duration_seconds"));
}

#[tokio::test]
async fn health_live_returns_ok_without_db_dependency() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let resp = reqwest_style_get(&format!("{base}/health/live")).await;
    assert_eq!(resp.status, 200);
    let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn health_ready_returns_ok_when_db_is_reachable() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let resp = reqwest_style_get(&format!("{base}/health/ready")).await;
    assert_eq!(resp.status, 200);
    let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["instance_id"], "test-instance");
}

#[tokio::test]
async fn health_info_exposes_instance_metadata() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let resp = reqwest_style_get(&format!("{base}/health/info")).await;
    assert_eq!(resp.status, 200);
    let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(body["name"], "minos-backend");
    assert_eq!(body["instance_id"], "test-instance");
    assert!(body["version"].is_string());
    assert!(body["build_profile"].is_string());
    assert!(body["env"].is_string());
}

// ── /ws/client: missing ticket → 401 ────────────────────────────────────

#[tokio::test]
async fn ws_client_missing_ticket_rejects_with_401() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let url: Uri = format!("{}/ws/client", http_to_ws(&base)).parse().unwrap();
    let builder = ClientRequestBuilder::new(url);
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("missing ticket must fail");
    assert_http_status(&err, 401, "missing ticket");
}

// ── /ws/client: valid ticket → Event::Unpaired ──────────────────────────

#[tokio::test]
async fn ws_client_ticket_connect_emits_unpaired_event() {
    use futures::StreamExt;

    let (base, _task, pool, auth) = spawn_relay().await;
    let account_id = store::accounts::create(&pool, "handshake-client@example.com", "phc")
        .await
        .unwrap()
        .account_id;
    let id = store::test_support::insert_ios_device(&pool, &account_id).await;
    let ticket = issue_client_ws_ticket(&auth, &account_id, id, DeviceRole::MobileClient)
        .await
        .unwrap();

    let mut ws = connect_gateway_ws(&base, "/ws/client", &ticket)
        .await
        .expect("WS must upgrade for a valid client ticket");

    let msg = ws.next().await.expect("expected first frame").unwrap();
    let text = match msg {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {other:?}"),
    };
    let env: Envelope = serde_json::from_str(&text).unwrap();
    match env {
        Envelope::Event {
            event: EventKind::Unpaired,
            ..
        } => {}
        other => panic!("expected Event::Unpaired, got {other:?}"),
    }

    let row = store::devices::get_device(&pool, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.role, DeviceRole::MobileClient);
    assert_eq!(row.account_id.as_deref(), Some(account_id.as_str()));
    assert!(
        row.secret_hash.is_none(),
        "formal client rows must stay bearer-only"
    );
}

// ── /ws/host: paired reconnect → Event::Unpaired (single-peer model removed) ─

// ADR-0020 / Phase G: the activate hook now always emits `Unpaired` as the
// initial frame; the legacy `PeerOffline` based on a single-peer paired_with
// slot was deleted alongside the device-keyed pairings module. Multi-host
// account-scoped presence rebuild on connect is deferred to Phase M.
#[tokio::test]
#[ignore = "ADR-0020 single-peer presence model removed; Phase M will reintroduce multi-host coverage"]
async fn devices_authenticated_connect_emits_peer_offline_event_when_peer_is_not_live() {
    use futures::StreamExt;

    let (base, _task, pool, auth) = spawn_relay().await;

    let mac_id = DeviceId::new();
    let mac_secret = DeviceSecret::generate();
    let mac_hash = hash_secret(&mac_secret).unwrap();

    store::devices::insert_device(&pool, mac_id, "mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(&pool, mac_id, &mac_hash)
        .await
        .unwrap();
    let account_id = store::accounts::create(&pool, "presence@example.com", "phc")
        .await
        .unwrap()
        .account_id;
    let ios_id = store::test_support::insert_ios_device(&pool, &account_id).await;
    store::account_host_pairings::insert_pair(&pool, mac_id, &account_id, ios_id, 0)
        .await
        .unwrap();

    let ticket = issue_host_ws_ticket(&auth, mac_id).unwrap();
    let mut ws = connect_gateway_ws(&base, "/ws/host", &ticket)
        .await
        .expect("authenticated upgrade must succeed");

    let msg = ws.next().await.expect("expected first frame").unwrap();
    let text = match msg {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {other:?}"),
    };
    let env: Envelope = serde_json::from_str(&text).unwrap();
    match env {
        Envelope::Event {
            event: EventKind::PeerOffline { peer_device_id },
            ..
        } => {
            assert_eq!(peer_device_id, ios_id);
        }
        other => panic!("expected Event::PeerOffline, got {other:?}"),
    }
}

// ── /ws/client: host ticket on wrong rail → 401 ─────────────────────────

#[tokio::test]
async fn ws_client_rejects_host_ticket_with_401() {
    let (base, _task, pool, auth) = spawn_relay().await;

    let id = DeviceId::new();
    let secret = DeviceSecret::generate();
    let secret_hash = hash_secret(&secret).unwrap();
    store::devices::insert_device(&pool, id, "mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(&pool, id, &secret_hash)
        .await
        .unwrap();

    let ticket = issue_host_ws_ticket(&auth, id).unwrap();
    let err = connect_gateway_ws(&base, "/ws/client", &ticket)
        .await
        .expect_err("host ticket on client rail must fail");
    assert_http_status(&err, 401, "host ticket on client rail");
}

// ── /ws/host: client ticket on wrong rail → 401 ─────────────────────────

#[tokio::test]
async fn ws_host_rejects_client_ticket_with_401() {
    let (base, _task, pool, auth) = spawn_relay().await;

    let account_id = store::accounts::create(&pool, "wrong-rail@example.com", "phc")
        .await
        .unwrap()
        .account_id;
    let id = store::test_support::insert_ios_device(&pool, &account_id).await;
    let ticket = issue_client_ws_ticket(&auth, &account_id, id, DeviceRole::MobileClient)
        .await
        .unwrap();

    let err = connect_gateway_ws(&base, "/ws/host", &ticket)
        .await
        .expect_err("client ticket on host rail must fail");
    assert_http_status(&err, 401, "client ticket on host rail");
}

// ── /ws/host: missing ticket → 401 ──────────────────────────────────────

#[tokio::test]
async fn ws_host_missing_ticket_rejects_with_401() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let url: Uri = format!("{}/ws/host", http_to_ws(&base)).parse().unwrap();
    let builder = ClientRequestBuilder::new(url);
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("host upgrade without ticket must fail");
    assert_http_status(&err, 401, "missing ticket on host upgrade");
}

// ── /ws/client: malformed ticket → 401 ──────────────────────────────────

#[tokio::test]
async fn ws_client_invalid_ticket_rejects_with_401() {
    let (base, _task, _pool, _auth) = spawn_relay().await;
    let url: Uri = format!("{}/ws/client?ticket=not-a-ticket", http_to_ws(&base))
        .parse()
        .unwrap();
    let err = tokio_tungstenite::connect_async(url.to_string())
        .await
        .expect_err("malformed ticket must fail");
    assert_http_status(&err, 401, "malformed ticket on client upgrade");
}

// ── /ws/client: reused ticket → 401 ─────────────────────────────────────

#[tokio::test]
async fn ws_client_reused_ticket_rejects_with_401() {
    let (base, _task, pool, auth) = spawn_relay().await;

    let account_id = store::accounts::create(&pool, "reuse-ticket@example.com", "phc")
        .await
        .unwrap()
        .account_id;
    let id = store::test_support::insert_ios_device(&pool, &account_id).await;
    let ticket = issue_client_ws_ticket(&auth, &account_id, id, DeviceRole::MobileClient)
        .await
        .unwrap();

    let _ws = connect_gateway_ws(&base, "/ws/client", &ticket)
        .await
        .expect("first ticket use must succeed");
    let err = connect_gateway_ws(&base, "/ws/client", &ticket)
        .await
        .expect_err("reused ticket must fail");
    assert_http_status(&err, 401, "reused ticket");
}

// ── helpers ────────────────────────────────────────────────────────────

/// Minimal HTTP GET for the health endpoints. We deliberately avoid pulling in
/// `reqwest` as a dev-dep and just write the request by hand against a
/// fresh TCP connection.
struct Response {
    status: u16,
    headers: HashMap<String, String>,
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

    let mut headers = HashMap::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    // Body after first blank line.
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    Response {
        status,
        headers,
        body: body.to_string(),
    }
}

#[track_caller]
fn assert_http_status(err: &WsError, expected: u16, context: &str) {
    match err {
        WsError::Http(resp) => {
            assert_eq!(
                resp.status().as_u16(),
                expected,
                "expected HTTP {expected} for `{context}`, got {}: body={:?}",
                resp.status(),
                resp.body().as_ref().map(|b| String::from_utf8_lossy(b))
            );
        }
        other => panic!("expected WsError::Http for `{context}`, got {other:?}"),
    }
}
