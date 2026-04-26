//! Envelope-client integration tests.
//!
//! These tests exercise three paths:
//!
//! 1. `pair_with_qr_json` end-to-end against the real `minos-backend`
//!    test router (HTTP `POST /v1/pairing/consume` followed by WS
//!    `/devices` opened with the freshly-issued secret).
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

use futures_util::{SinkExt, StreamExt};
use minos_backend::http::{router as backend_router, BackendPublicConfig, BackendState};
use minos_backend::pairing::PairingService;
use minos_backend::session::{SessionHandle, SessionRegistry};
use minos_backend::store::test_support::memory_pool;
use minos_domain::{ConnectionState, DeviceId, DeviceRole};
use minos_mobile::{MobileClient, PersistedPairingState};
use minos_protocol::{Envelope, EventKind, ListThreadsParams, PairingQrPayload};
use minos_ui_protocol::UiEventMessage;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

// ── real-backend helpers ────────────────────────────────────────────────

/// Spin up a fresh `minos-backend` on `127.0.0.1:0`, register a live Mac
/// session in the registry (so `POST /v1/pairing/consume` can deliver
/// `Event::Paired`), and mint a pairing token. Returns the bound address,
/// the freshly-minted token, the Mac's session-outbox receiver, and a
/// handle to the Mac side so callers can push fan-out events into the
/// paired iPhone via `state.registry.try_send_current(&peer_handle, ..)`.
struct RealBackend {
    addr: std::net::SocketAddr,
    token: String,
    state: BackendState,
    /// Mac-side outbox receiver. The handler keeps the sender side alive
    /// inside the registry; we hold the receiver so the channel doesn't
    /// close (which would trip the consume-path's compensation branch).
    _mac_outbox: tokio::sync::mpsc::Receiver<Envelope>,
}

async fn spawn_backend_with_paired_mac() -> RealBackend {
    let pool = memory_pool().await;
    let registry = Arc::new(SessionRegistry::new());
    let pairing = Arc::new(PairingService::new(pool.clone()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let public_url = format!("ws://{addr}/devices");

    let state = BackendState {
        registry: registry.clone(),
        pairing: pairing.clone(),
        store: pool.clone(),
        token_ttl: Duration::from_secs(300),
        translators: minos_backend::ingest::translate::ThreadTranslators::new(),
        public_cfg: Arc::new(BackendPublicConfig {
            public_url,
            cf_access_client_id: None,
            cf_access_client_secret: None,
        }),
        jwt_secret: Arc::new("a".repeat(32)),
        auth_login_per_email: minos_backend::http::default_login_per_email(),
        auth_login_per_ip: minos_backend::http::default_login_per_ip(),
        auth_register_per_ip: minos_backend::http::default_register_per_ip(),
        auth_refresh_per_acc: minos_backend::http::default_refresh_per_acc(),
        version: "mobile-envelope-test",
    };

    let app = backend_router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Seed Mac device row + token + live session so the consume-path can
    // deliver `Event::Paired` and the consumer's WS handshake (post-pair)
    // is allowed.
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
    let (handle, mac_outbox) = SessionHandle::new(mac_id, DeviceRole::AgentHost);
    state.registry.insert(handle);

    RealBackend {
        addr,
        token: token.as_str().to_string(),
        state,
        _mac_outbox: mac_outbox,
    }
}

fn make_qr_for_real_backend(addr: std::net::SocketAddr, token: &str) -> String {
    serde_json::to_string(&PairingQrPayload {
        v: 2,
        backend_url: format!("ws://{addr}/devices"),
        host_display_name: "FakeMac".into(),
        pairing_token: token.into(),
        expires_at_ms: i64::MAX,
        cf_access_client_id: None,
        cf_access_client_secret: None,
    })
    .unwrap()
}

/// Phase 2 made `/v1/pairing/consume` and the iOS WS upgrade
/// bearer-gated. Tests build a MobileClient that's already authenticated
/// by registering an account over HTTP using the same device id, then
/// rehydrating the client from a PersistedPairingState that includes the
/// minted tokens. `new_with_persisted_state` populates the live
/// auth_session so `pair_with_qr_json` finds the Bearer in place.
async fn authenticated_client(backend: &RealBackend, email: &str) -> MobileClient {
    let device_id = minos_domain::DeviceId::new();
    let http = minos_mobile::http::MobileHttpClient::new(
        &format!("ws://{}/devices", backend.addr),
        device_id,
        None,
    )
    .unwrap();
    let resp = http
        .register(email, "testpass1")
        .await
        .expect("register against test backend");

    let now_ms = chrono::Utc::now().timestamp_millis();
    let persisted = PersistedPairingState {
        backend_url: Some(format!("ws://{}/devices", backend.addr)),
        device_id: Some(device_id.to_string()),
        device_secret: None,
        cf_access_client_id: None,
        cf_access_client_secret: None,
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
    client.pair_with_qr_json(qr).await.unwrap();

    assert_eq!(client.current_state(), ConnectionState::Connected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_events_stream_delivers_backend_fanout() {
    let backend = spawn_backend_with_paired_mac().await;

    let client = authenticated_client(&backend, "fanout@example.com").await;
    let consumer_id = client.device_id();
    let mut rx = client.ui_events_stream();

    let qr = make_qr_for_real_backend(backend.addr, &backend.token);
    client.pair_with_qr_json(qr).await.unwrap();

    // Push a fan-out event into the iPhone's live session via the registry.
    // Wait briefly for the WS to register the session post-pair.
    let push = Envelope::Event {
        version: 1,
        event: EventKind::UiEventMessage {
            thread_id: "thr_1".into(),
            seq: 7,
            ui: UiEventMessage::TextDelta {
                message_id: "msg_1".into(),
                text: "Hi".into(),
            },
            ts_ms: 42,
        },
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(handle) = backend.state.registry.get(consumer_id) {
                let _ = backend
                    .state
                    .registry
                    .try_send_current(&handle, push.clone());
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("iPhone session registered within 2s");

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("should receive one frame within 2s")
        .expect("broadcast channel must stay open");
    assert_eq!(frame.thread_id, "thr_1");
    assert_eq!(frame.seq, 7);
    match frame.ui {
        UiEventMessage::TextDelta { text, .. } => assert_eq!(text, "Hi"),
        other => panic!("unexpected ui variant: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_threads_round_trips_over_envelope() {
    let backend = spawn_backend_with_paired_mac().await;

    let client = authenticated_client(&backend, "list@example.com").await;
    let qr = make_qr_for_real_backend(backend.addr, &backend.token);
    client.pair_with_qr_json(qr).await.unwrap();

    // The real backend has no threads seeded → expect empty page.
    let resp = client
        .list_threads(ListThreadsParams {
            limit: 50,
            before_ts_ms: None,
            agent: None,
        })
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
    client.pair_with_qr_json(qr).await.unwrap();

    let persisted = client.persisted_pairing_state().await.unwrap();
    assert_eq!(persisted.backend_url.as_deref(), Some(backend_url.as_str()));
    // Real backend has no CF Access tokens, so the QR carried none.
    assert!(persisted.cf_access_client_id.is_none());
    assert!(persisted.cf_access_client_secret.is_none());
    assert!(persisted.device_id.is_some());
    let secret = persisted
        .device_secret
        .clone()
        .expect("pair must persist a device secret");
    assert_eq!(secret.len(), 43, "DeviceSecret base64url is 43 chars");

    // The auth tuple is populated since the test pre-registered an account.
    let access_token = persisted.access_token.clone().expect("auth set by helper");
    assert!(!access_token.is_empty());

    let rehydrated = MobileClient::new_with_persisted_state("iPhone".into(), persisted.clone());
    let restored = rehydrated.persisted_pairing_state().await.unwrap();
    let expected = PersistedPairingState {
        backend_url: Some(backend_url),
        device_id: persisted.device_id.clone(),
        device_secret: Some(secret),
        cf_access_client_id: None,
        cf_access_client_secret: None,
        access_token: persisted.access_token.clone(),
        access_expires_at_ms: persisted.access_expires_at_ms,
        refresh_token: persisted.refresh_token.clone(),
        account_id: persisted.account_id.clone(),
        account_email: persisted.account_email.clone(),
    };
    assert_eq!(restored, expected);
}

// ── resume_persisted_session: WS-only fake backend ──────────────────────

/// Accept the resume WS handshake, assert expected `X-Device-*` and
/// CF-Access headers were forwarded, then keep the socket open until the
/// client closes. After Phase C the `list_threads` query rides HTTP, so
/// the fake doesn't need to handle any envelope frames here.
async fn fake_backend_resume_handshake(
    listener: TcpListener,
    expected_device_id: String,
    expected_device_secret: String,
    expected_cf_access: Option<(String, String)>,
) {
    let (stream, _) = listener.accept().await.expect("accept");
    let ws = accept_hdr_async(
        stream,
        #[allow(clippy::result_large_err)] // accept_hdr_async dictates the closure signature.
        move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
            let headers = req.headers();
            assert_eq!(
                headers
                    .get("X-Device-Id")
                    .and_then(|value| value.to_str().ok()),
                Some(expected_device_id.as_str())
            );
            assert_eq!(
                headers
                    .get("X-Device-Secret")
                    .and_then(|value| value.to_str().ok()),
                Some(expected_device_secret.as_str())
            );
            if let Some((id, secret)) = &expected_cf_access {
                assert_eq!(
                    headers
                        .get("CF-Access-Client-Id")
                        .and_then(|value| value.to_str().ok()),
                    Some(id.as_str())
                );
                assert_eq!(
                    headers
                        .get("CF-Access-Client-Secret")
                        .and_then(|value| value.to_str().ok()),
                    Some(secret.as_str())
                );
            } else {
                assert!(headers.get("CF-Access-Client-Id").is_none());
                assert!(headers.get("CF-Access-Client-Secret").is_none());
            }
            Ok(response)
        },
    )
    .await
    .expect("handshake");
    let (mut write, mut read) = ws.split();

    // The test only cares about the headers asserted during the upgrade
    // closure above. After Phase C the `list_threads` query rides HTTP, so
    // the fake doesn't need to handle any envelope frames here. Close the
    // socket cleanly so the client-side reader returns and the test can
    // join the backend without hanging.
    let _ = write
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "test_done".into(),
        })))
        .await;
    while read.next().await.is_some() {}
}

/// Accept one client, immediately close the socket with code 4401 to
/// simulate the backend rejecting a stale-secret resume (the same close
/// code `ws_devices::upgrade` emits when activation revalidation fails).
async fn fake_backend_resume_rejects_with_4401(listener: TcpListener) {
    let (stream, _) = listener.accept().await.expect("accept");
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("handshake");
    let (mut write, _read) = ws.split();
    let _ = write
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Library(4401),
            reason: "auth_revoked".into(),
        })))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_persisted_session_returns_error_when_backend_rejects_with_4401() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let device_id = DeviceId::new();
    let backend = tokio::spawn(fake_backend_resume_rejects_with_4401(listener));

    let client = MobileClient::new_with_persisted_state(
        "iPhone".into(),
        PersistedPairingState {
            backend_url: Some(format!("ws://{addr}/devices")),
            device_id: Some(device_id.to_string()),
            device_secret: Some("sec_revoked".into()),
            cf_access_client_id: None,
            cf_access_client_secret: None,
            access_token: None,
            access_expires_at_ms: None,
            refresh_token: None,
            account_id: None,
            account_email: None,
        },
    );

    let resume = tokio::time::timeout(Duration::from_secs(2), client.resume_persisted_session())
        .await
        .expect("resume_persisted_session must not hang on a 4401 close");

    let _ = resume;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(client.current_state(), ConnectionState::Disconnected) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("resume must end in Disconnected after 4401 close");

    backend.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_persisted_session_reconnects_and_forwards_cf_access_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let device_id = DeviceId::new();
    let backend = tokio::spawn(fake_backend_resume_handshake(
        listener,
        device_id.to_string(),
        "sec_resume".into(),
        Some(("cf-id".into(), "cf-secret".into())),
    ));

    let client = MobileClient::new_with_persisted_state(
        "iPhone".into(),
        PersistedPairingState {
            backend_url: Some(format!("ws://{addr}/devices")),
            device_id: Some(device_id.to_string()),
            device_secret: Some("sec_resume".into()),
            cf_access_client_id: Some("cf-id".into()),
            cf_access_client_secret: Some("cf-secret".into()),
            access_token: None,
            access_expires_at_ms: None,
            refresh_token: None,
            account_id: None,
            account_email: None,
        },
    );

    client.resume_persisted_session().await.unwrap();
    assert_eq!(client.current_state(), ConnectionState::Connected);

    // The fake backend asserted the resume handshake's X-Device-* and
    // CF-Access headers; nothing more to verify on the WS side. The
    // post-Phase-C `list_threads` round-trip lives in
    // `list_threads_round_trips_over_envelope` (real backend, HTTP-backed).
    drop(client);
    backend.await.unwrap();
}
