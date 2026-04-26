use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::pairing::PairingService;
use minos_backend::store::devices::insert_device;
use minos_domain::{DeviceId, DeviceRole, PairingToken};
use minos_protocol::{PairConsumeRequest, PairResponse, RequestPairingQrResponse};
use std::time::Duration as StdDuration;

mod common;

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
        .header("x-device-role", "ios-client")
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
async fn post_pairing_consume_happy_path_returns_secret_and_pairs() {
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
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", consumer_id.to_string())
        .header("x-device-role", "ios-client")
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
    assert_eq!(resp.your_device_secret.as_str().len(), 43);

    // Pairing committed
    let pair = minos_backend::store::pairings::get_pair(&state.store, mac_id)
        .await
        .unwrap();
    assert_eq!(pair, Some(consumer_id));

    // Issuer received Event::Paired
    let frame = mac_outbox
        .recv()
        .await
        .expect("issuer receives Event::Paired");
    match frame {
        minos_protocol::Envelope::Event {
            event: minos_protocol::EventKind::Paired { peer_device_id, .. },
            ..
        } => {
            assert_eq!(peer_device_id, consumer_id);
        }
        other => panic!("expected Event::Paired, got {other:?}"),
    }
}

#[tokio::test]
async fn post_pairing_consume_invalid_token_returns_409() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", DeviceId::new().to_string())
        .header("x-device-role", "ios-client")
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
