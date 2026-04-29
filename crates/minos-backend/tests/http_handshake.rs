//! Integration tests for the HTTP surface (`/health` + `/devices` upgrade).
//!
//! Each test spawns a real axum server on an ephemeral port and drives it
//! with a real `tokio-tungstenite` client. This mirrors what the full e2e
//! (step 12) will do, but with a focused coverage of the handshake path
//! added in step 9.

use std::{sync::Arc, time::Duration};

use minos_backend::{
    auth::jwt,
    http::{router, BackendState},
    pairing::{secret::hash_secret, PairingService},
    session::SessionRegistry,
    store,
};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{
    client::ClientRequestBuilder, http::Uri, protocol::Message, Error as WsError,
};

/// Fixed JWT secret used by the test relay; mirrors `test_support::TEST_JWT_SECRET`.
const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";

/// Bring up a relay on an ephemeral port and return `(base_url, server_task)`.
///
/// The server task is a detached `tokio::spawn`; the test drops its handle
/// at end of scope, which lets `axum::serve` shut down when the tokio
/// runtime tears down.
async fn spawn_relay() -> (String, tokio::task::JoinHandle<()>, sqlx::SqlitePool) {
    let pool = store::connect("sqlite::memory:").await.unwrap();

    let state = BackendState {
        registry: Arc::new(SessionRegistry::new()),
        pairing: Arc::new(PairingService::new(pool.clone())),
        store: pool.clone(),
        token_ttl: Duration::from_mins(5),
        translators: minos_backend::ingest::translate::ThreadTranslators::new(),
        jwt_secret: Arc::new(TEST_JWT_SECRET.to_string()),
        auth_login_per_email: minos_backend::http::default_login_per_email(),
        auth_login_per_ip: minos_backend::http::default_login_per_ip(),
        auth_register_per_ip: minos_backend::http::default_register_per_ip(),
        auth_refresh_per_acc: minos_backend::http::default_refresh_per_acc(),
        version: "test",
    };
    let app = router(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (base, handle, pool)
}

fn http_to_ws(base: &str) -> String {
    base.replacen("http://", "ws://", 1)
}

// ── /health ────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok_with_name_and_version() {
    let (base, _task, _pool) = spawn_relay().await;
    let resp = reqwest_style_get(&format!("{base}/health")).await;
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

// ── /devices: missing X-Device-Id → 401 ─────────────────────────────────

#[tokio::test]
async fn devices_missing_device_id_rejects_with_401() {
    let (base, _task, _pool) = spawn_relay().await;
    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let builder = ClientRequestBuilder::new(url);
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("no auth headers must fail");
    assert_http_status(&err, 401, "missing X-Device-Id");
}

// ── /devices: first connect with X-Device-Id → Event::Unpaired ──────────

#[tokio::test]
async fn devices_first_connect_emits_unpaired_event() {
    use futures::StreamExt;

    let (base, _task, pool) = spawn_relay().await;
    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let id = DeviceId::new();
    // iOS upgrades require a bearer post-Phase-2 Task 2.2; sign one bound
    // to this device id so the first-connect path still surfaces the
    // initial Event::Unpaired we want to assert here.
    let token = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        "acct-first-connect",
        &id.to_string(),
    )
    .expect("sign test bearer");
    let builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", id.to_string())
        .with_header("X-Device-Role", DeviceRole::IosClient.to_string())
        .with_header("X-Device-Name", "my-phone".to_string())
        .with_header("Authorization", format!("Bearer {token}"));

    let (mut ws, _resp) = tokio_tungstenite::connect_async(builder)
        .await
        .expect("WS must upgrade for first-connect");

    // First frame must be Event::Unpaired.
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

    // The device row was inserted with display_name = "my-phone" and the
    // role we sent.
    let row = store::devices::get_device(&pool, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.display_name, "my-phone");
    assert_eq!(row.role, DeviceRole::IosClient);
    assert!(
        row.secret_hash.is_none(),
        "first connect must have NULL hash"
    );
}

// ── /devices: paired reconnect with peer offline → Event::PeerOffline ───

#[tokio::test]
async fn devices_authenticated_connect_emits_peer_offline_event_when_peer_is_not_live() {
    use futures::StreamExt;

    let (base, _task, pool) = spawn_relay().await;

    // Seed a paired Mac + iOS, both with known secrets.
    let mac_id = DeviceId::new();
    let ios_id = DeviceId::new();
    let mac_secret = DeviceSecret::generate();
    let ios_secret = DeviceSecret::generate();
    let mac_hash = hash_secret(&mac_secret).unwrap();
    let ios_hash = hash_secret(&ios_secret).unwrap();

    store::devices::insert_device(&pool, mac_id, "mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    store::devices::insert_device(&pool, ios_id, "ios", DeviceRole::IosClient, 0)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(&pool, mac_id, &mac_hash)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(&pool, ios_id, &ios_hash)
        .await
        .unwrap();
    store::pairings::insert_pairing(&pool, mac_id, ios_id, 0)
        .await
        .unwrap();

    // Mac reconnects with the right secret.
    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", mac_id.to_string())
        .with_header("X-Device-Role", DeviceRole::AgentHost.to_string())
        .with_header("X-Device-Secret", mac_secret.as_str().to_string());

    let (mut ws, _resp) = tokio_tungstenite::connect_async(builder)
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

// ── /devices: existing row rejects spoofed X-Device-Role ───────────────

#[tokio::test]
async fn devices_role_spoof_rejects_with_401() {
    let (base, _task, pool) = spawn_relay().await;

    let id = DeviceId::new();
    let secret = DeviceSecret::generate();
    let secret_hash = hash_secret(&secret).unwrap();
    store::devices::insert_device(&pool, id, "mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(&pool, id, &secret_hash)
        .await
        .unwrap();

    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", id.to_string())
        .with_header("X-Device-Role", DeviceRole::IosClient.to_string())
        .with_header("X-Device-Secret", secret.as_str().to_string());
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("spoofed role must fail");
    assert_http_status(&err, 401, "spoofed X-Device-Role");
}

// ── /devices: reconnect with WRONG secret → 401 ─────────────────────────

#[tokio::test]
async fn devices_wrong_secret_rejects_with_401() {
    let (base, _task, pool) = spawn_relay().await;

    let id = DeviceId::new();
    let good = DeviceSecret::generate();
    let good_hash = hash_secret(&good).unwrap();
    store::devices::insert_device(&pool, id, "x", DeviceRole::IosClient, 0)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(&pool, id, &good_hash)
        .await
        .unwrap();

    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", id.to_string())
        .with_header("X-Device-Secret", "definitely-not-the-right-secret");
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("wrong secret must fail");
    assert_http_status(&err, 401, "wrong X-Device-Secret");
}

// ── /devices: ios upgrade without bearer → 401 ─────────────────────────

/// iOS upgrades must present a bearer; missing the `Authorization` header
/// rejects the upgrade with 401 (Phase 2 Task 2.2). The Mac (`AgentHost`)
/// path keeps device-secret-only auth — exercised by
/// `devices_authenticated_connect_emits_peer_offline_event_when_peer_is_not_live`.
#[tokio::test]
async fn devices_ios_upgrade_without_bearer_rejects_with_401() {
    let (base, _task, _pool) = spawn_relay().await;
    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let id = DeviceId::new();
    // iOS role + valid X-Device-Id but no Authorization header.
    let builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", id.to_string())
        .with_header("X-Device-Role", DeviceRole::IosClient.to_string());
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("ios upgrade without bearer must fail");
    assert_http_status(&err, 401, "missing bearer on iOS upgrade");
}

// ── /devices: ios upgrade with bearer-did-mismatch → 401 ───────────────

/// JWT `did` claim must equal the `X-Device-Id` header. If they disagree,
/// the upgrade rejects with 401 (Phase 2 Task 2.2).
#[tokio::test]
async fn devices_ios_upgrade_with_did_mismatch_rejects_with_401() {
    let (base, _task, _pool) = spawn_relay().await;
    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let header_id = DeviceId::new();
    let token_did = DeviceId::new(); // different device baked into token
    let token = jwt::sign(TEST_JWT_SECRET.as_bytes(), "acct-1", &token_did.to_string()).unwrap();
    let builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", header_id.to_string())
        .with_header("X-Device-Role", DeviceRole::IosClient.to_string())
        .with_header("Authorization", format!("Bearer {token}"));
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("did mismatch must fail");
    assert_http_status(&err, 401, "bearer did mismatch on iOS upgrade");
}

// ── /devices: row-with-hash but NO secret → 401 ─────────────────────────

#[tokio::test]
async fn devices_missing_secret_on_authed_device_rejects_with_401() {
    let (base, _task, pool) = spawn_relay().await;

    let id = DeviceId::new();
    let good = DeviceSecret::generate();
    let good_hash = hash_secret(&good).unwrap();
    store::devices::insert_device(&pool, id, "x", DeviceRole::IosClient, 0)
        .await
        .unwrap();
    store::devices::upsert_secret_hash(&pool, id, &good_hash)
        .await
        .unwrap();

    let url: Uri = format!("{}/devices", http_to_ws(&base)).parse().unwrap();
    let builder = ClientRequestBuilder::new(url).with_header("X-Device-Id", id.to_string());
    let err = tokio_tungstenite::connect_async(builder)
        .await
        .expect_err("missing secret must fail");
    assert_http_status(&err, 401, "missing X-Device-Secret");
}

// ── helpers ────────────────────────────────────────────────────────────

/// Minimal HTTP GET for `/health`. We deliberately avoid pulling in
/// `reqwest` as a dev-dep and just write the request by hand against a
/// fresh TCP connection.
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

    // Body after first blank line.
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    Response {
        status,
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
