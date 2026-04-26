use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_domain::DeviceId;
use minos_protocol::RequestPairingQrResponse;

mod common;

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
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
