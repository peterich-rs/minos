//! Formal realtime gateway handshake/auth coverage for client and host rails.
//!
//! The topic gateway now speaks `Hello`/`ServerFrame` exclusively. These
//! tests verify the remaining upgrade semantics around ws-tickets, host
//! revocation, and per-device session replacement.

mod common;

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use axum::{body::Body, http::Request};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use futures::{SinkExt, StreamExt};
use minos_backend::store::test_support::insert_test_client;
use minos_backend::{
    auth::{jwt, use_case::AuthUseCase},
    host_link::HostLinkService,
    http::{router, BackendState},
    session::SessionRegistry,
    store,
};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::realtime::ServerFrame;
use serde_json::json;
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    tungstenite::{
        http::{StatusCode, Uri},
        protocol::Message,
        Error as WsError,
    },
    MaybeTlsStream, WebSocketStream,
};

const TEST_JWT_SECRET: &str = "test-jwt-secret-32-bytes-padding";
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
const QUIET_TIMEOUT: Duration = Duration::from_millis(200);

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct FormalHostFixture {
    host: DeviceId,
    token: String,
    account_auth_header: String,
}

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public_key(signing_key: &SigningKey) -> String {
    format!(
        "ed25519:{}",
        URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes())
    )
}

fn signature_for_path(
    signing_key: &SigningKey,
    installation_id: &str,
    nonce: &str,
    path: &str,
) -> String {
    let payload = format!("{installation_id}:{nonce}:{path}");
    format!(
        "ed25519-sig:{}",
        URL_SAFE_NO_PAD.encode(signing_key.sign(payload.as_bytes()).to_bytes())
    )
}

fn json_body(value: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&value).unwrap())
}

async fn post_json(
    app: &mut axum::Router,
    path: &str,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("x-request-id", "req_ws_devices");
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    common::send(app, builder.body(json_body(body)).unwrap()).await
}

fn host_headers(fixture: &FormalHostFixture) -> Vec<(&'static str, String)> {
    vec![("authorization", format!("Bearer {}", fixture.token))]
}

async fn formally_paired_host(relay: &Relay) -> anyhow::Result<FormalHostFixture> {
    const LINK_PATH: &str = "v1/hosts/link";

    let mut app = router(relay.state.clone());
    let account_id = store::accounts::create(&relay.pool, "ws-formal-host@example.com")
        .await?
        .account_id;
    let mobile = store::test_support::insert_ios_device(&relay.pool, &account_id).await;
    // Desktop installation that performs Host Link on behalf of the account.
    let desktop = DeviceId::new();
    {
        let _acct = minos_backend::store::accounts::create(
            &relay.pool,
            &format!("fixture-{}@localhost", desktop),
        )
        .await
        .unwrap();
        insert_test_client(
            &relay.pool,
            desktop,
            DeviceRole::DesktopConsole,
            &_acct.account_id,
            "desktop",
            0,
        )
        .await;
    };
    store::device_installations::set_account_id(&relay.pool, &desktop, &account_id).await?;

    let host = DeviceId::new();
    let installation_id = host.to_string();
    let signing_key = signing_key(31);
    let host_public_key = public_key(&signing_key);

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    anyhow::ensure!(status == StatusCode::OK, "nonce body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let signature = signature_for_path(&signing_key, &installation_id, &nonce, LINK_PATH);

    let bearer = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        &account_id,
        &desktop.to_string(),
    )
    .expect("test bearer signs cleanly");
    let account_auth_header = format!("Bearer {bearer}");
    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &account_auth_header)],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": host_public_key,
            "signature": signature,
            "host_display_name": "WS Formal Host"
        }),
    )
    .await;
    anyhow::ensure!(status == StatusCode::OK, "host link body={body}");
    let _ = mobile;

    Ok(FormalHostFixture {
        host,
        token: body["data"]["host_installation_token"]
            .as_str()
            .unwrap()
            .to_string(),
        account_auth_header,
    })
}

struct Relay {
    addr: SocketAddr,
    state: BackendState,
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
        "ws-devices-instance".to_string(),
    );
    state.version = "ws-devices-test";
    let relay_state = state.clone();
    let auth = Arc::clone(&state.auth);
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        state: relay_state,
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

async fn connect_formal_gateway_ws(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    account_id: Option<&str>,
) -> anyhow::Result<WsClient> {
    if role.is_account_client() {
        let acct = account_id.expect("account client connect requires an account_id");
        let ticket = issue_client_ws_ticket(relay, acct, device_id, role).await?;
        return Ok(connect_gateway_ws_with_ticket(relay, gateway_path_for_role(role), &ticket).await?);
    }
    let hit = store::test_support::issue_test_host_token(&relay.pool, device_id, 0).await;
    connect_host_bearer(relay, &hit).await
}

async fn connect_host_bearer(relay: &Relay, hit_token: &str) -> anyhow::Result<WsClient> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("ws://{}/ws/host", relay.addr);
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {hit_token}").parse()?);
    let (ws, _resp) = tokio_tungstenite::connect_async(request).await?;
    Ok(ws)
}

async fn connect_gateway_ws_with_legacy_ticket_query(
    relay: &Relay,
    path: &str,
    ticket: &str,
) -> Result<WsClient, WsError> {
    let url: Uri = format!("ws://{}{}?ws_ticket={ticket}", relay.addr, path)
        .parse()
        .unwrap();
    let (ws, _resp) = tokio_tungstenite::connect_async(url.to_string()).await?;
    Ok(ws)
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

async fn issue_host_bearer(relay: &Relay, host_id: DeviceId) -> anyhow::Result<String> {
    Ok(store::test_support::issue_test_host_token(&relay.pool, host_id, 0).await)
}

async fn connect_gateway_ws_with_ticket(
    relay: &Relay,
    path: &str,
    ticket: &str,
) -> Result<WsClient, WsError> {
    let url: Uri = format!("ws://{}{}?ticket={ticket}", relay.addr, path)
        .parse()
        .unwrap();
    let (ws, _resp) = tokio_tungstenite::connect_async(url.to_string()).await?;
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

async fn expect_no_server_frame(ws: &mut WsClient) -> anyhow::Result<()> {
    match timeout(QUIET_TIMEOUT, recv_server_frame(ws)).await {
        Err(_) => Ok(()),
        Ok(Ok(frame)) => Err(anyhow::anyhow!("expected idle socket, got {frame:?}")),
        Ok(Err(error)) => Err(error),
    }
}

/// Hello is register-only; gateway immediately acks the default topic live arm.
async fn drain_default_topic_subscribe_ack(ws: &mut WsClient) -> anyhow::Result<()> {
    match recv_server_frame(ws).await? {
        ServerFrame::SubscribeAck { topics, .. } => {
            anyhow::ensure!(
                topics.len() == 1,
                "expected single default-topic SubscribeAck after Hello, got {topics:?}"
            );
            Ok(())
        }
        other => anyhow::bail!("expected default-topic SubscribeAck after Hello, got {other:?}"),
    }
}

async fn expect_close_code(
    ws: &mut WsClient,
    expected_code: u16,
    expected_reason: &str,
) -> anyhow::Result<()> {
    loop {
        let next = timeout(RECV_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for close frame"))?;
        match next {
            Some(Ok(Message::Close(Some(frame)))) => {
                assert_eq!(u16::from(frame.code), expected_code);
                assert_eq!(frame.reason.to_string(), expected_reason);
                return Ok(());
            }
            Some(Ok(Message::Close(None))) => {
                return Err(anyhow::anyhow!("close frame missing code/reason"));
            }
            None => return Err(anyhow::anyhow!("stream ended before close frame arrived")),
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
                return Err(anyhow::anyhow!("socket closed before close frame arrived"));
            }
            Some(Err(e)) => return Err(anyhow::anyhow!("ws error while waiting for close: {e}")),
        }
    }
}

fn assert_unauthorized_upgrade(error: WsError) {
    match error {
        WsError::Http(response) => assert_eq!(response.status(), StatusCode::UNAUTHORIZED),
        other => panic!("expected HTTP 401 websocket rejection, got {other:?}"),
    }
}

/// Pre-seed an agent-host installation row.
async fn register_agent_host(pool: &SqlitePool) -> DeviceId {
    let host_id = DeviceId::new();
    store::test_support::insert_test_host(pool, host_id, "mac", 0).await;
    host_id
}

#[tokio::test]
async fn ws_host_connects_with_hello_for_agent_host() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let host_id = register_agent_host(&relay.pool).await;

    // Seed two sessions owned by `host_id` and a few raw events on each so
    // `last_seq_per_owner` returns `{thr_1: 7, thr_2: 3}`.
    minos_backend::store::sessions::upsert(
        &relay.pool,
        "thr_1",
        AgentName::Codex,
        &host_id.to_string(),
        0,
    )
    .await?;
    minos_backend::store::sessions::upsert(
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

    let mut ws = connect_formal_gateway_ws(&relay, host_id, DeviceRole::AgentHost, None).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello {
            heartbeat_interval_ms,
            ..
        } => assert_eq!(heartbeat_interval_ms, 25_000),
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut ws).await?;
    expect_no_server_frame(&mut ws).await?;

    Ok(())
}

#[tokio::test]
async fn ws_host_stays_idle_when_no_host_durable_events() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let host_id = register_agent_host(&relay.pool).await;

    let mut ws = connect_formal_gateway_ws(&relay, host_id, DeviceRole::AgentHost, None).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut ws).await?;
    expect_no_server_frame(&mut ws).await?;

    Ok(())
}

#[tokio::test]
async fn ws_client_emits_only_hello_for_mobile_client() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    // Seed an authenticated mobile client (account-bound, no secret hash).
    let account_id = store::accounts::create(&relay.pool, "ws-devices@example.com")
        .await?
        .account_id;
    let phone_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;

    let mut ws = connect_formal_gateway_ws(
        &relay,
        phone_id,
        DeviceRole::MobileClient,
        Some(&account_id),
    )
    .await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut ws).await?;
    expect_no_server_frame(&mut ws).await?;

    Ok(())
}

#[tokio::test]
async fn ws_client_accepts_browser_admin_legacy_ws_ticket_query_auth() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let account_id = store::accounts::create(&relay.pool, "browser-ws@example.com")
        .await?
        .account_id;
    let browser_id = DeviceId::new();
    {
        let _acct = minos_backend::store::accounts::create(
            &relay.pool,
            &format!("fixture-{}@localhost", browser_id),
        )
        .await
        .unwrap();
        insert_test_client(
            &relay.pool,
            browser_id,
            DeviceRole::BrowserAdmin,
            &_acct.account_id,
            "web",
            0,
        )
        .await;
    };
    store::device_installations::set_account_id(&relay.pool, &browser_id, &account_id).await?;

    let ticket =
        issue_client_ws_ticket(&relay, &account_id, browser_id, DeviceRole::BrowserAdmin).await?;
    let mut ws = connect_gateway_ws_with_legacy_ticket_query(&relay, "/ws/client", &ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn ws_client_accepts_formal_ticket_query_auth() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let account_id = store::accounts::create(&relay.pool, "formal-client-ws@example.com")
        .await?
        .account_id;
    let browser_id = DeviceId::new();
    {
        let _acct = minos_backend::store::accounts::create(
            &relay.pool,
            &format!("fixture-{}@localhost", browser_id),
        )
        .await
        .unwrap();
        insert_test_client(
            &relay.pool,
            browser_id,
            DeviceRole::BrowserAdmin,
            &_acct.account_id,
            "web",
            0,
        )
        .await;
    };
    store::device_installations::set_account_id(&relay.pool, &browser_id, &account_id).await?;

    let ticket =
        issue_client_ws_ticket(&relay, &account_id, browser_id, DeviceRole::BrowserAdmin).await?;
    let mut ws = connect_gateway_ws_with_ticket(&relay, "/ws/client", &ticket).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn ws_client_rejects_reused_formal_ticket() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let account_id = store::accounts::create(&relay.pool, "formal-client-reuse@example.com")
        .await?
        .account_id;
    let browser_id = DeviceId::new();
    {
        let _acct = minos_backend::store::accounts::create(
            &relay.pool,
            &format!("fixture-{}@localhost", browser_id),
        )
        .await
        .unwrap();
        insert_test_client(
            &relay.pool,
            browser_id,
            DeviceRole::BrowserAdmin,
            &_acct.account_id,
            "web",
            0,
        )
        .await;
    };
    store::device_installations::set_account_id(&relay.pool, &browser_id, &account_id).await?;

    let ticket =
        issue_client_ws_ticket(&relay, &account_id, browser_id, DeviceRole::BrowserAdmin).await?;
    let mut ws = connect_gateway_ws_with_ticket(&relay, "/ws/client", &ticket).await?;
    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }

    let err = connect_gateway_ws_with_ticket(&relay, "/ws/client", &ticket)
        .await
        .expect_err("reusing a consumed ws ticket must fail pre-upgrade");
    assert_unauthorized_upgrade(err);

    Ok(())
}

#[tokio::test]
async fn ws_host_accepts_bearer_hit_auth() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let host_id = register_agent_host(&relay.pool).await;
    let hit = issue_host_bearer(&relay, host_id).await?;
    let mut ws = connect_host_bearer(&relay, &hit).await?;

    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut ws).await?;
    expect_no_server_frame(&mut ws).await?;

    Ok(())
}

#[tokio::test]
async fn ws_client_reconnect_supersedes_prior_socket_with_auth_close() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let account_id = store::accounts::create(&relay.pool, "reconnect-ws@example.com")
        .await?
        .account_id;
    let phone_id = store::test_support::insert_ios_device(&relay.pool, &account_id).await;

    let mut first = connect_formal_gateway_ws(
        &relay,
        phone_id,
        DeviceRole::MobileClient,
        Some(&account_id),
    )
    .await?;
    match recv_server_frame(&mut first).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut first).await?;

    let mut replacement = connect_formal_gateway_ws(
        &relay,
        phone_id,
        DeviceRole::MobileClient,
        Some(&account_id),
    )
    .await?;
    match recv_server_frame(&mut replacement).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut replacement).await?;

    expect_close_code(&mut first, 4401, "session_superseded").await?;

    Ok(())
}

#[tokio::test]
async fn ws_host_last_link_revoke_closes_live_socket_with_auth_revoked() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let fixture = formally_paired_host(&relay).await?;
    let mut app = router(relay.state.clone());

    let mut ws = connect_host_bearer(&relay, &fixture.token).await?;
    match recv_server_frame(&mut ws).await? {
        ServerFrame::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    drain_default_topic_subscribe_ack(&mut ws).await?;

    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/unlink",
        &[("authorization", fixture.account_auth_header.as_str())],
        json!({
            "host_installation_id": fixture.host.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body={body}");

    match recv_server_frame(&mut ws).await? {
        ServerFrame::HostForceClose { reason, close_code } => {
            assert_eq!(reason, "auth_revoked");
            assert_eq!(close_code, 4401);
        }
        other => panic!("expected HostForceClose, got {other:?}"),
    }
    expect_close_code(&mut ws, 4401, "auth_revoked").await?;

    Ok(())
}

#[tokio::test]
async fn ws_client_rejects_legacy_ws_ticket_after_device_account_changes() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let account_a = store::accounts::create(&relay.pool, "browser-a@example.com")
        .await?
        .account_id;
    let account_b = store::accounts::create(&relay.pool, "browser-b@example.com")
        .await?
        .account_id;
    let browser_id = DeviceId::new();
    {
        let _acct = minos_backend::store::accounts::create(
            &relay.pool,
            &format!("fixture-{}@localhost", browser_id),
        )
        .await
        .unwrap();
        insert_test_client(
            &relay.pool,
            browser_id,
            DeviceRole::BrowserAdmin,
            &_acct.account_id,
            "web",
            0,
        )
        .await;
    };
    store::device_installations::set_account_id(&relay.pool, &browser_id, &account_a).await?;

    let ticket =
        issue_client_ws_ticket(&relay, &account_a, browser_id, DeviceRole::BrowserAdmin).await?;

    store::device_installations::set_account_id(&relay.pool, &browser_id, &account_b).await?;

    let mut ws = connect_gateway_ws_with_legacy_ticket_query(&relay, "/ws/client", &ticket).await?;
    expect_close_code(&mut ws, 4401, "auth_revoked").await?;

    Ok(())
}
