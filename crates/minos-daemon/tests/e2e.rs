//! Relay-backed daemon end-to-end coverage.
//!
//! Drives a real in-process backend and a real `DaemonHandle` so the formal
//! host-command path is covered end-to-end:
//!
//! 1. Backend has a formal host/account pairing and host installation token.
//! 2. Host connects to `/ws/host` with a short-lived ticket.
//! 3. Account caller posts `/v1/host-commands/list-clis`.
//! 4. Backend delivers a durable command to the host and returns its result.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt as _;
use minos_backend::{
    auth::jwt,
    http::{router, BackendState},
    pairing::PairingService,
    session::SessionRegistry,
    store,
};
use minos_daemon::{DaemonHandle, RelayConfig};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret, RelayLinkState};
use serde_json::json;
use sqlx::SqlitePool;
use tempfile::{NamedTempFile, TempDir};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tower::ServiceExt as _;

const TEST_JWT_SECRET: &str = "daemon-e2e-jwt-secret-32-bytes";
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

struct Relay {
    addr: SocketAddr,
    state: BackendState,
    _pool: SqlitePool,
    _db_file: NamedTempFile,
    task: JoinHandle<()>,
}

struct PairedHost {
    host_secret: DeviceSecret,
    bearer: String,
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

    let mut state = BackendState::new(
        Arc::new(SessionRegistry::new()),
        Arc::new(PairingService::new(pool.clone())),
        pool.clone(),
        Duration::from_mins(5),
        TEST_JWT_SECRET.to_string(),
        None,
        "daemon-e2e-instance".to_string(),
    );
    state.version = "daemon-e2e";
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

async fn wait_for_host_subscription(relay: &Relay, host_id: DeviceId) -> anyhow::Result<()> {
    let topic = minos_protocol::realtime::RealtimeTopic::Host(host_id.to_string());
    timeout(STEP_TIMEOUT, async {
        loop {
            if !relay
                .state
                .subscription_mgr
                .fanout_targets(&topic)
                .is_empty()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("host realtime topic was not subscribed within timeout"))?;
    Ok(())
}

async fn wait_for_host_command_roundtrip(relay: &Relay, host_id: DeviceId) -> anyhow::Result<()> {
    timeout(STEP_TIMEOUT, async {
        let mut attempt = 0u32;
        loop {
            attempt = attempt.saturating_add(1);
            let command_id = format!("daemon-e2e-health-{attempt}");
            if relay
                .state
                .host_commands
                .dispatch_json(
                    &command_id,
                    host_id,
                    None,
                    "minos_health",
                    &serde_json::Value::Null,
                    None,
                    Duration::from_secs(2),
                )
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("host command round-trip did not become ready"))?;
    Ok(())
}

async fn register_formal_host(relay: &Relay, host_id: DeviceId) -> anyhow::Result<PairedHost> {
    store::device_installations::insert_device(
        &relay.state.store,
        host_id,
        "Test Mac",
        DeviceRole::AgentHost,
        0,
    )
    .await?;
    let account =
        store::accounts::create(&relay.state.store, "relay-e2e@example.com", "phc").await?;
    let mobile_id = DeviceId::new();
    store::device_installations::insert_device(
        &relay.state.store,
        mobile_id,
        "Test iPhone",
        DeviceRole::MobileClient,
        0,
    )
    .await?;
    store::device_installations::set_account_id(
        &relay.state.store,
        &mobile_id,
        &account.account_id,
    )
    .await?;

    let (code, _) = relay
        .state
        .pairing
        .request_code(host_id, Duration::from_secs(300))
        .await?;
    relay
        .state
        .pairing
        .confirm_pairing_code(
            &code,
            &account.account_id,
            mobile_id,
            Some("daemon-e2e-confirm"),
        )
        .await
        .map_err(|error| anyhow::anyhow!("confirm formal pairing code failed: {error:?}"))?;
    let redeemed = relay
        .state
        .pairing
        .redeem_host_installation(&code, host_id, Some("daemon-e2e-redeem"))
        .await
        .map_err(|error| anyhow::anyhow!("redeem formal host token failed: {error:?}"))?;
    let host_secret = DeviceSecret(redeemed.token);
    let bearer = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        &account.account_id,
        &mobile_id.to_string(),
    )?;

    Ok(PairedHost {
        host_secret,
        bearer,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_and_list_clis_over_relay() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let minos_home = TempDir::new()?;
    let _home_guard = MinosHomeGuard::install(minos_home.path());

    let host_id = DeviceId::new();
    let paired = register_formal_host(&relay, host_id).await?;
    let handle = DaemonHandle::start(
        RelayConfig::new(relay_ws_url(&relay)),
        host_id,
        None,
        Some(paired.host_secret),
        "Test Mac".into(),
    )
    .await?;

    wait_for_connected(&handle).await?;
    wait_for_paired_peer(&handle).await?;
    wait_for_host_subscription(&relay, host_id).await?;
    wait_for_host_command_roundtrip(&relay, host_id).await?;

    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/v1/host-commands/list-clis")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", paired.bearer))
        .body(axum::body::Body::from(
            json!({ "host_installation_id": host_id.to_string() }).to_string(),
        ))?;
    let (status, body) = send_http(&relay, request).await;
    if status != axum::http::StatusCode::OK {
        let command_rows: Vec<(
            String,
            String,
            Option<i64>,
            Option<i64>,
            Option<String>,
            i64,
        )> = sqlx::query_as(
            "SELECT command_id, status, ack_at_ms, finished_at_ms, error_json, deadline_at_ms
               FROM host_commands
              ORDER BY created_at_ms ASC",
        )
        .fetch_all(&relay._pool)
        .await
        .unwrap_or_default();
        let outbox_rows: Vec<(
            String,
            i64,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT o.status, o.attempts, o.ack_at_ms, o.dead_at_ms, o.last_error_json,
                    json_extract(d.payload_json, '$.kind'),
                    json_extract(d.payload_json, '$.command_id')
               FROM outbox_events o
               JOIN durable_event_log d
                 ON d.topic_kind = o.topic_kind
                AND d.event_id = o.event_id
              ORDER BY o.available_at_ms ASC",
        )
        .fetch_all(&relay._pool)
        .await
        .unwrap_or_default();
        panic!(
            "expected list-clis OK, got status={status}, body={body}, command_rows={command_rows:?}, outbox_rows={outbox_rows:?}"
        );
    }
    let result = body
        .as_array()
        .expect("list-clis should return an array response");
    assert!(result.len() >= 3, "expected at least 3 CLI entries: {body}");
    let names: Vec<&str> = result
        .iter()
        .filter_map(|entry| entry.get("name").and_then(serde_json::Value::as_str))
        .collect();
    assert!(names.contains(&"codex"));
    assert!(names.contains(&"claude"));
    assert!(names.contains(&"gemini"));

    handle.stop().await?;
    Ok(())
}
