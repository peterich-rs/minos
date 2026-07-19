//! Envelope-client integration tests.
//!
//! These tests exercise three paths:
//!
//! 1. `pair_with_qr_json` end-to-end against the real `minos-backend`
//!    test router (HTTP `POST /v1/pairing/confirm` followed by the
//!    formal `/ws/client` ticket flow).
//! 2. The post-pair WebSocket fan-out path: backend pushes
//!    `EventKind::UiEventMessage` and the mobile client surfaces it on
//!    `ui_events_stream`.
//! 3. `resume_persisted_session` against a *fake* WS-only backend that
//!    does not need the HTTP control plane (these scenarios pre-date a
//!    pairing — the persisted secret already exists).
//!
//! These tests do not exercise CF Access (no edge is involved) and do not
//! exercise reconnection loops — the plan's scope is MVP envelope wiring.

// MSRV portability: prefer `Duration::from_secs(N * 60)` over
// `Duration::from_mins(N)` (which was only stabilized in Rust 1.84). See
// the matching crate-level allow in `src/lib.rs`.
#![allow(clippy::duration_suboptimal_units)]

use std::sync::Arc;
use std::time::Duration;

use minos_backend::http::{router as backend_router, BackendState};
use minos_backend::pairing::PairingService;
use minos_backend::session::SessionRegistry;
use minos_backend::store::test_support::memory_pool;
use minos_domain::{ConnectionState, DeviceId, DeviceRole};
use minos_mobile::{MobileClient, PersistedPairingState};
use minos_protocol::realtime::{RealtimeTopic, ServerFrame};
use minos_protocol::{ListThreadsParams, PairingQrPayload};
use minos_ui_protocol::UiEventMessage;
use tokio::net::TcpListener;

// ── real-backend helpers ────────────────────────────────────────────────

/// Spin up a fresh `minos-backend` on `127.0.0.1:0`, register a host
/// installation row, and mint a formal pairing code.
struct RealBackend {
    addr: std::net::SocketAddr,
    token: String,
    state: BackendState,
}

async fn spawn_backend_with_paired_mac() -> RealBackend {
    let pool = memory_pool().await;
    let registry = Arc::new(SessionRegistry::new());
    let pairing = Arc::new(PairingService::new(pool.clone()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut state = BackendState::new(
        registry.clone(),
        pairing.clone(),
        pool.clone(),
        Duration::from_secs(300),
        "a".repeat(32),
        None,
        "mobile-envelope-test-instance".to_string(),
    );
    state.version = "mobile-envelope-test";

    let app = backend_router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Seed host device row + pairing code so account-side pairing confirm
    // can create the account-host link.
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
        .request_code(mac_id, Duration::from_secs(300))
        .await
        .unwrap();

    RealBackend {
        addr,
        token: token.as_str().to_string(),
        state,
    }
}

fn make_qr_for_real_backend(_addr: std::net::SocketAddr, token: &str) -> String {
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

/// Phase 2 made formal pairing and the iOS WS upgrade bearer-gated. Tests
/// build a MobileClient that's already authenticated by registering an
/// account over HTTP using the same device id, then rehydrating the client
/// from a PersistedPairingState that includes the minted tokens.
/// `new_with_persisted_state` populates the live auth_session so
/// `pair_with_qr_json` finds the Bearer in place.
async fn authenticated_client(backend: &RealBackend, email: &str) -> MobileClient {
    let device_id = minos_domain::DeviceId::new();
    let http = minos_mobile::http::MobileHttpClient::new(
        &format!("ws://{}/devices", backend.addr),
        device_id,
        "iPhone",
    )
    .unwrap();
    let resp = http
        .register(email, "testpass1")
        .await
        .expect("register against test backend");

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

// ── tests against the real backend ──────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_qr_json_happy_path_reaches_connected() {
    let backend = spawn_backend_with_paired_mac().await;

    let client = authenticated_client(&backend, "happy@example.com").await;
    let qr = make_qr_for_real_backend(backend.addr, &backend.token);
    let backend_url = format!("ws://{}/devices", backend.addr);
    client.pair_with_qr_json_at(qr, &backend_url).await.unwrap();

    assert_eq!(client.current_state(), ConnectionState::Connected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_events_stream_delivers_backend_fanout() {
    let backend = spawn_backend_with_paired_mac().await;

    let client = authenticated_client(&backend, "fanout@example.com").await;
    let mut rx = client.ui_events_stream();

    let qr = make_qr_for_real_backend(backend.addr, &backend.token);
    let backend_url = format!("ws://{}/devices", backend.addr);
    client.pair_with_qr_json_at(qr, &backend_url).await.unwrap();

    let account_id = client
        .persisted_pairing_state()
        .await
        .unwrap()
        .account_id
        .expect("authenticated_client seeded account_id");
    let topic = RealtimeTopic::Account(account_id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut observed_targets = 0usize;
    let mut sent_frames = 0usize;
    let frame = loop {
        let targets = backend.state.subscription_mgr.fanout_targets(&topic);
        observed_targets = observed_targets.max(targets.len());
        for target in targets {
            target
                .send(ServerFrame::StreamEvent {
                    topic: "agent_session:thr_1".into(),
                    kind: "agent_text_delta".into(),
                    seq: Some(7),
                    payload: serde_json::json!({
                        "message_id": "msg_1",
                        "text": "Hi"
                    }),
                })
                .expect("formal connection push channel should accept frame");
            sent_frames += 1;
        }

        if let Ok(Ok(frame)) = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await {
            break frame;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "should receive one frame within 2s (observed_targets={observed_targets}, sent_frames={sent_frames}, client_state={:?})",
            client.current_state()
        );
    };

    assert_eq!(frame.thread_id, "thr_1");
    assert_eq!(frame.seq, 7);
    match frame.ui {
        UiEventMessage::TextDelta { message_id, text } => {
            assert_eq!(message_id, "msg_1");
            assert_eq!(text, "Hi");
        }
        other => panic!("unexpected ui variant: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_threads_round_trips_over_envelope() {
    let backend = spawn_backend_with_paired_mac().await;

    let client = authenticated_client(&backend, "list@example.com").await;
    let qr = make_qr_for_real_backend(backend.addr, &backend.token);
    let backend_url = format!("ws://{}/devices", backend.addr);
    client.pair_with_qr_json_at(qr, &backend_url).await.unwrap();

    // After ADR-0020, MobileClient::list_threads is bearer-only. The
    // mobile client itself uses build_config::BACKEND_URL; tests drive
    // the round-trip through MobileHttpClient directly with the same
    // bearer the rehydrated client persisted post-pair.
    let persisted = client.persisted_pairing_state().await.unwrap();
    let device_id = client.device_id();
    let access = persisted
        .access_token
        .clone()
        .expect("authenticated_client seeded the access token");

    let http =
        minos_mobile::http::MobileHttpClient::new(&backend_url, device_id, "iPhone").unwrap();
    let resp = http
        .list_threads(
            &access,
            ListThreadsParams {
                limit: 50,
                before_ts_ms: None,
                agent: None,
            },
        )
        .await
        .unwrap();
    assert!(resp.threads.is_empty());
    assert!(resp.next_before_ts_ms.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_exports_persisted_state_and_rehydrates_new_client() {
    let backend = spawn_backend_with_paired_mac().await;
    let backend_url = format!("ws://{}/devices", backend.addr);

    let client = authenticated_client(&backend, "rehyd@example.com").await;
    let qr = make_qr_for_real_backend(backend.addr, &backend.token);
    client.pair_with_qr_json_at(qr, &backend_url).await.unwrap();

    let persisted = client.persisted_pairing_state().await.unwrap();
    // Backend URL and CF Access fields no longer round-trip through
    // PersistedPairingState — they live in compile-time build_config.
    // ADR-0020 also dropped the device_secret from the snapshot.
    assert!(persisted.device_id.is_some());

    // The auth tuple is populated since the test pre-registered an account.
    let access_token = persisted.access_token.clone().expect("auth set by helper");
    assert!(!access_token.is_empty());

    let rehydrated = MobileClient::new_with_persisted_state("iPhone".into(), persisted.clone());
    let restored = rehydrated.persisted_pairing_state().await.unwrap();
    assert_eq!(restored, persisted);
}

// ── resume_persisted_session: formal ticket flow ───────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_persisted_session_returns_error_when_ticket_request_is_unauthorized() {
    let backend = spawn_backend_with_paired_mac().await;
    let device_id = DeviceId::new();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let client = MobileClient::new_with_persisted_state(
        "iPhone".into(),
        PersistedPairingState {
            device_id: Some(device_id.to_string()),
            access_token: Some("invalid_bearer".into()),
            access_expires_at_ms: Some(now_ms + 15 * 60 * 1000),
            refresh_token: Some("rev_refresh".into()),
            account_id: Some("acct-rev".into()),
            account_email: Some("rev@example.com".into()),
        },
    );

    let backend_url = format!("ws://{}/devices", backend.addr);
    let resume = tokio::time::timeout(
        Duration::from_secs(2),
        client.resume_persisted_session_at(&backend_url),
    )
    .await
    .expect("resume_persisted_session must not hang when ticket request is rejected");

    assert!(resume.is_err(), "invalid bearer should fail resume");
    assert_eq!(client.current_state(), ConnectionState::Disconnected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_persisted_session_reconnects_with_formal_ticket_flow() {
    let backend = spawn_backend_with_paired_mac().await;
    let client = authenticated_client(&backend, "resume@example.com").await;

    let backend_url = format!("ws://{}/devices", backend.addr);
    client
        .resume_persisted_session_at(&backend_url)
        .await
        .unwrap();
    assert_eq!(client.current_state(), ConnectionState::Connected);

    let _ = client.logout().await;
}
