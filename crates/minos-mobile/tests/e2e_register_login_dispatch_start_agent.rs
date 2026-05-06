//! End-to-end smoke for the Wave 2 mobile dispatch surface.
//!
//! Plan 08b Phase 12 Task 12.2.
//!
//! Mirrors the in-process backend setup from `envelope_client.rs`, then
//! attaches an in-process fake-Mac handler that drains the Mac
//! session's outbox. Whenever a `minos_start_agent` JSON-RPC request
//! lands, the handler echoes a synthetic `Forwarded` reply with a
//! deterministic `session_id` so the iPhone's `start_agent` future
//! resolves with that same id. `minos_send_user_message` is handled
//! separately so we can prove the session bootstrap no longer waits for
//! first-message delivery.
//!
//! What this test guards:
//!
//! - The `register → pair_with_qr_json → start_agent` composition all
//!   the way down. A regression in any one of those layers (auth
//!   adoption, pair_consume bearer-stamping, JSON-RPC id correlation)
//!   surfaces here as a hang or a wrong session_id.
//! - That the synthetic reply's session_id is what the iPhone returns
//!   from `start_agent` — i.e. the inner JSON-RPC payload makes it
//!   through `Envelope::Forwarded` correctly.

#![allow(clippy::duration_suboptimal_units)]

use std::sync::Arc;
use std::time::Duration;

use minos_backend::http::{router as backend_router, BackendState};
use minos_backend::pairing::PairingService;
use minos_backend::session::{SessionHandle, SessionRegistry};
use minos_backend::store::test_support::memory_pool;
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_mobile::{MobileClient, PersistedPairingState};
use minos_protocol::{Envelope, PairingQrPayload};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Reusable in-process backend fixture matching the shape of
/// `envelope_client::spawn_backend_with_paired_mac`. Returns the bound
/// address, a freshly-minted pairing token, the backend state (so the
/// caller can drive the registry), the Mac's device id, and the Mac
/// outbox receiver.
struct Backend {
    addr: std::net::SocketAddr,
    token: String,
    state: BackendState,
    mac_id: DeviceId,
    mac_outbox: mpsc::Receiver<Envelope>,
}

async fn spawn_backend() -> Backend {
    let pool = memory_pool().await;
    let registry = Arc::new(SessionRegistry::new());
    let pairing = Arc::new(PairingService::new(pool.clone()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state = BackendState {
        registry: registry.clone(),
        pairing: pairing.clone(),
        store: pool.clone(),
        token_ttl: Duration::from_secs(300),
        translators: minos_backend::ingest::translate::ThreadTranslators::new(),
        jwt_secret: Arc::new("a".repeat(32)),
        auth_login_per_email: minos_backend::http::default_login_per_email(),
        auth_login_per_ip: minos_backend::http::default_login_per_ip(),
        auth_register_per_ip: minos_backend::http::default_register_per_ip(),
        auth_refresh_per_acc: minos_backend::http::default_refresh_per_acc(),
        version: "mobile-e2e-dispatch-test",
    };

    let app = backend_router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let mac_id = DeviceId::new();
    minos_backend::store::devices::insert_device(
        &state.store,
        mac_id,
        "FakeMac",
        DeviceRole::AgentHost,
        0,
    )
    .await
    .unwrap();
    let (token, _exp) = pairing
        .request_token(mac_id, Duration::from_secs(300))
        .await
        .unwrap();
    let (mac_handle, mac_outbox) = SessionHandle::new(mac_id, DeviceRole::AgentHost);
    state.registry.insert(mac_handle);

    Backend {
        addr,
        token: token.as_str().to_string(),
        state,
        mac_id,
        mac_outbox,
    }
}

fn qr_for(_addr: std::net::SocketAddr, token: &str) -> String {
    // The QR no longer carries the backend URL — the mobile crate's
    // `build_config::BACKEND_URL` is the source of truth, and tests use
    // `pair_with_qr_json_at` to inject a per-test address.
    serde_json::to_string(&PairingQrPayload {
        v: 2,
        host_display_name: "FakeMac".into(),
        pairing_token: token.into(),
        expires_at_ms: i64::MAX,
    })
    .unwrap()
}

/// Register an account via HTTP and rehydrate the MobileClient with the
/// freshly minted auth tuple. Same recipe `envelope_client.rs` uses.
async fn registered_client(addr: std::net::SocketAddr, email: &str) -> MobileClient {
    let device_id = DeviceId::new();
    let http = minos_mobile::http::MobileHttpClient::new(
        &format!("ws://{addr}/devices"),
        device_id,
        "iPhone",
        None,
    )
    .unwrap();
    let resp = http.register(email, "testpass1").await.expect("register");

    let now_ms = chrono::Utc::now().timestamp_millis();
    let persisted = PersistedPairingState {
        device_id: Some(device_id.to_string()),
        access_token: Some(resp.access_token),
        access_expires_at_ms: Some(now_ms + 15 * 60 * 1000),
        refresh_token: Some(resp.refresh_token),
        account_id: Some(resp.account.account_id),
        account_email: Some(resp.account.email),
    };
    MobileClient::new_with_persisted_state("iPhone".into(), persisted)
}

const SYNTHETIC_SESSION_ID: &str = "ses_fake_12345";
const SYNTHETIC_CWD: &str = "/Users/fake/workspace";

/// Spawn an in-process fake-Mac handler that drains the Mac outbox,
/// parses every inbound `Forwarded { payload: jsonrpc-request }`, and
/// replies via `try_send_current` directly into the iPhone's session.
///
/// `try_send_current` bypasses the Mac→iOS account-scope check baked
/// into `route()`, which would otherwise reject this synthetic reply
/// because the test's Mac handle never gets its `account_id` set. The
/// iPhone is the actual authenticated peer here, so this short-circuit
/// is the realistic shape: a real Mac would be authenticated through
/// its own bearer flow, which we don't exercise in this test.
fn spawn_fake_mac(
    registry: Arc<SessionRegistry>,
    mac_id: DeviceId,
    mut mac_outbox: mpsc::Receiver<Envelope>,
    send_delay: Option<Duration>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(env) = mac_outbox.recv().await {
            let Envelope::Forwarded { from, payload, .. } = env else {
                continue;
            };
            let method = payload
                .get("method")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let id = payload
                .get("id")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            if method == "minos_send_user_message" {
                if let Some(delay) = send_delay {
                    tokio::time::sleep(delay).await;
                }
            }
            let result = match method {
                "minos_start_agent" => serde_json::json!({
                    "session_id": SYNTHETIC_SESSION_ID,
                    "cwd": SYNTHETIC_CWD,
                }),
                "minos_send_user_message" | "minos_stop_agent" => serde_json::Value::Null,
                other => {
                    eprintln!("fake-mac: unhandled method {other}");
                    continue;
                }
            };
            let reply_payload = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            });
            // The iPhone's session was registered when its WS upgrade
            // completed; look it up and push the synthetic reply.
            let Some(iphone_handle) = registry.get(from) else {
                eprintln!("fake-mac: iPhone session {from} not in registry");
                continue;
            };
            let _ = registry.try_send_current(
                &iphone_handle,
                Envelope::Forwarded {
                    version: 1,
                    from: mac_id,
                    payload: reply_payload,
                },
            );
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn register_pair_start_agent_round_trips_synthetic_session_id() {
    let Backend {
        addr,
        token,
        state,
        mac_id,
        mac_outbox,
    } = spawn_backend().await;
    let registry = state.registry.clone();

    // The mac_outbox has to live somewhere; move it into the fake-Mac
    // task. We can't keep both a receiver here AND in the task.
    let mac_handler = spawn_fake_mac(registry.clone(), mac_id, mac_outbox, None);

    let client = registered_client(addr, "dispatch@example.com").await;

    let qr = qr_for(addr, &token);
    let backend_url = format!("ws://{addr}/devices");
    client
        .pair_with_qr_json_at(qr, &backend_url)
        .await
        .expect("pair");

    // Wait briefly for the iPhone WS activation to register the session
    // (paired_with is set during activation, after pair_consume returns).
    let consumer_id = client.device_id();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if registry.get(consumer_id).is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("iPhone session registers within 2s");

    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        client.start_agent(AgentName::Codex, "Hello from e2e".into()),
    )
    .await
    .expect("start_agent must complete within 5s")
    .expect("start_agent returns Ok");

    assert_eq!(resp.session_id, SYNTHETIC_SESSION_ID);
    assert_eq!(resp.cwd, SYNTHETIC_CWD);

    drop(client);
    mac_handler.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_agent_returns_before_first_message_delivery_round_trip() {
    let Backend {
        addr,
        token,
        state,
        mac_id,
        mac_outbox,
    } = spawn_backend().await;
    let registry = state.registry.clone();
    let mac_handler = spawn_fake_mac(
        registry.clone(),
        mac_id,
        mac_outbox,
        Some(Duration::from_secs(2)),
    );

    let client = registered_client(addr, "dispatch-fast-start@example.com").await;

    let qr = qr_for(addr, &token);
    let backend_url = format!("ws://{addr}/devices");
    client
        .pair_with_qr_json_at(qr, &backend_url)
        .await
        .expect("pair");

    let consumer_id = client.device_id();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if registry.get(consumer_id).is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("iPhone session registers within 2s");

    let resp = tokio::time::timeout(
        Duration::from_millis(500),
        client.start_agent(AgentName::Codex, "Hello from e2e".into()),
    )
    .await
    .expect("start_agent should resolve before first-message delivery")
    .expect("start_agent returns Ok");

    assert_eq!(resp.session_id, SYNTHETIC_SESSION_ID);

    tokio::time::timeout(
        Duration::from_secs(5),
        client.send_user_message(resp.session_id.clone(), "Hello from e2e".into()),
    )
    .await
    .expect("send_user_message must complete within 5s")
    .expect("send_user_message returns Ok");

    drop(client);
    mac_handler.abort();
}
