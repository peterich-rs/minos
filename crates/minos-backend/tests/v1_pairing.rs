use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::test_support::TEST_JWT_SECRET;
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::pairing::PairingService;
use minos_backend::store::devices::insert_device;
use minos_domain::{DeviceId, DeviceRole, PairingToken};
use minos_protocol::{PairConsumeRequest, PairResponse, RequestPairingQrResponse};
use std::time::Duration as StdDuration;

mod common;

/// Helper: sign a bearer JWT bound to `device_id` for the test JWT
/// secret. Use a shared `account_id` across both sides of a pair when the
/// test asserts post-consume account propagation.
fn sign_bearer(device_id: DeviceId, account_id: &str) -> String {
    jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        account_id,
        &device_id.to_string(),
    )
    .expect("test bearer signs cleanly")
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
}

fn seed_live_session(
    state: &minos_backend::http::BackendState,
    device_id: DeviceId,
    role: DeviceRole,
) -> tokio::sync::mpsc::Receiver<minos_protocol::Envelope> {
    use minos_backend::session::SessionHandle;
    let (handle, outbox_rx) = SessionHandle::new(device_id, role);
    state.registry.insert(handle);
    outbox_rx
}

#[tokio::test]
async fn post_pairing_tokens_mints_qr_payload_for_agent_host() {
    let state = backend_state().await;
    let mut app = router(state);
    let device_id = DeviceId::new();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/tokens")
        .header("x-device-id", device_id.to_string())
        .header("x-device-role", "agent-host")
        .header("x-device-name", "Mac")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(
            serde_json::json!({ "host_display_name": "Fan's Mac" }),
        ))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);

    let resp: RequestPairingQrResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.qr_payload.v, 2);
    assert_eq!(resp.qr_payload.host_display_name, "Fan's Mac");
    assert!(!resp.qr_payload.pairing_token.is_empty());
    assert!(resp.qr_payload.expires_at_ms > 0);
}

#[tokio::test]
async fn post_pairing_tokens_rejects_ios_client() {
    let state = backend_state().await;
    let mut app = router(state);
    let device_id = DeviceId::new();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/tokens")
        .header("x-device-id", device_id.to_string())
        .header("x-device-role", "mobile-client")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::json!({ "host_display_name": "x" })))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn post_pairing_tokens_rejects_missing_device_id() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/tokens")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::json!({ "host_display_name": "x" })))
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn post_pairing_consume_happy_path_pairs_account_and_mac() {
    let state = backend_state().await;

    // Pre-seed a Mac issuer + token (mirrors what /v1/pairing/tokens does).
    let mac_id = DeviceId::new();
    insert_device(&state.store, mac_id, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    let svc = PairingService::new(state.store.clone());
    let (token, _expires) = svc
        .request_token(mac_id, StdDuration::from_mins(5))
        .await
        .unwrap();

    let mut mac_outbox = seed_live_session(&state, mac_id, DeviceRole::AgentHost);

    let mut app = router(state.clone());
    let consumer_id = DeviceId::new();
    // Pre-create the consumer's account row + device row so the bearer
    // can be issued and `set_account_id` (called by /pairing/consume)
    // satisfies the FK on `accounts(account_id)`.
    let account = minos_backend::store::accounts::create(&state.store, "ios@example.com", "phc")
        .await
        .unwrap();
    let bearer = sign_bearer(consumer_id, &account.account_id);
    let auth_hdr = format!("Bearer {bearer}");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", consumer_id.to_string())
        .header("x-device-role", "mobile-client")
        .header("authorization", &auth_hdr)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(
            serde_json::to_value(PairConsumeRequest {
                token: token.clone(),
                device_name: "iPhone".into(),
            })
            .unwrap(),
        ))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);

    let resp: PairResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.peer_device_id, mac_id);
    assert_eq!(resp.peer_name, "Mac");

    // Pair row committed in account_mac_pairings.
    assert!(minos_backend::store::account_mac_pairings::exists(
        &state.store,
        mac_id,
        &account.account_id,
    )
    .await
    .unwrap());

    // Issuer received Event::Paired with Some(secret).
    let frame = mac_outbox
        .recv()
        .await
        .expect("issuer receives Event::Paired");
    match frame {
        minos_protocol::Envelope::Event {
            event:
                minos_protocol::EventKind::Paired {
                    peer_device_id,
                    your_device_secret,
                    ..
                },
            ..
        } => {
            assert_eq!(peer_device_id, consumer_id);
            let secret = your_device_secret.expect("Mac side gets a secret");
            assert_eq!(secret.as_str().len(), 43);
        }
        other => panic!("expected Event::Paired, got {other:?}"),
    }
}

#[tokio::test]
async fn pairing_consume_ios_writes_account_id_to_pairing_record() {
    // Phase 2 Task 2.3: a successful /v1/pairing/consume must propagate
    // the iOS bearer's `account_id` to BOTH device rows so subsequent
    // Mac→iOS routing (Task 2.4) can scope by account.
    let state = backend_state().await;
    let mac_id = DeviceId::new();
    insert_device(&state.store, mac_id, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    let svc = PairingService::new(state.store.clone());
    let (token, _expires) = svc
        .request_token(mac_id, StdDuration::from_mins(5))
        .await
        .unwrap();

    let _mac_outbox = seed_live_session(&state, mac_id, DeviceRole::AgentHost);

    let mut app = router(state.clone());
    let consumer_id = DeviceId::new();
    let account = minos_backend::store::accounts::create(&state.store, "scoped@example.com", "phc")
        .await
        .unwrap();
    let bearer = sign_bearer(consumer_id, &account.account_id);
    let auth_hdr = format!("Bearer {bearer}");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", consumer_id.to_string())
        .header("x-device-role", "mobile-client")
        .header("authorization", &auth_hdr)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(
            serde_json::to_value(PairConsumeRequest {
                token: token.clone(),
                device_name: "iPhone".into(),
            })
            .unwrap(),
        ))
        .unwrap();
    let (status, _body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);

    let consumer_row = minos_backend::store::devices::get_device(&state.store, consumer_id)
        .await
        .unwrap()
        .expect("consumer device row");
    let issuer_row = minos_backend::store::devices::get_device(&state.store, mac_id)
        .await
        .unwrap()
        .expect("issuer device row");
    assert_eq!(
        consumer_row.account_id.as_deref(),
        Some(account.account_id.as_str()),
        "consumer (iOS) row should carry the bearer's account_id",
    );
    assert_eq!(
        issuer_row.account_id.as_deref(),
        Some(account.account_id.as_str()),
        "issuer (Mac) row should inherit the iOS bearer's account_id",
    );

    // Live Mac handle should also see the account_id so routing in
    // Task 2.4 can scope without waiting for a Mac reconnect.
    let live_mac = state.registry.get(mac_id).expect("Mac session live");
    assert_eq!(
        live_mac.account_id().as_deref(),
        Some(account.account_id.as_str()),
        "live Mac handle should have account_id seeded after consume",
    );
}

#[tokio::test]
async fn post_pairing_consume_invalid_token_returns_409() {
    let state = backend_state().await;
    let consumer_id = DeviceId::new();
    let account = minos_backend::store::accounts::create(&state.store, "ios2@example.com", "phc")
        .await
        .unwrap();
    let bearer = sign_bearer(consumer_id, &account.account_id);
    let auth_hdr = format!("Bearer {bearer}");
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", consumer_id.to_string())
        .header("x-device-role", "mobile-client")
        .header("authorization", &auth_hdr)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(
            serde_json::to_value(PairConsumeRequest {
                token: PairingToken::generate(),
                device_name: "iPhone".into(),
            })
            .unwrap(),
        ))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "pairing_token_invalid");
}

#[tokio::test]
async fn post_pairing_consume_rejects_agent_host_role() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", DeviceId::new().to_string())
        .header("x-device-role", "agent-host")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(
            serde_json::to_value(PairConsumeRequest {
                token: PairingToken::generate(),
                device_name: "iPhone".into(),
            })
            .unwrap(),
        ))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn delete_pairing_legacy_route_returns_410_gone() {
    // Phase E2: legacy `DELETE /v1/pairing` is replaced by
    // `DELETE /v1/pairings/{mac_device_id}` (bearer-authenticated). The
    // legacy route now returns 410 Gone with a directive message.
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/v1/pairing")
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["error"]["code"], "replaced");
}
