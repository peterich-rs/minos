//! Host rail contract after QR pairing removal.
//!
//! Host binding is Host Link (`tests/v1_hosts_link.rs`). This file covers
//! remaining host endpoints and asserts retired QR routes are gone.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use minos_backend::http;
use minos_backend::http::test_support::backend_state;
use minos_domain::DeviceId;
use serde_json::json;

mod common;

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
        .header("x-request-id", "req_host_contract");
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    common::send(app, builder.body(json_body(body)).unwrap()).await
}

#[tokio::test]
async fn host_bootstrap_nonce_issues_for_valid_installation_id() {
    let state = backend_state().await;
    let installation_id = DeviceId::new().to_string();
    let mut app = http::router(state);

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body["data"]["nonce"].as_str().unwrap().starts_with("nonce_"));
    assert!(body["data"]["expires_at_ms"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn host_bootstrap_nonce_rejects_invalid_installation_id() {
    let state = backend_state().await;
    let mut app = http::router(state);

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": "not-a-uuid"}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

#[tokio::test]
async fn retired_qr_pairing_routes_return_404() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let installation_id = DeviceId::new().to_string();

    for path in [
        "/v1/host/pairing/request-code",
        "/v1/host/pairing/redeem",
        "/v1/pairing/confirm",
        "/v1/pairing/revoke",
        "/v1/pairing/list-hosts",
    ] {
        let (status, _body) = post_json(
            &mut app,
            path,
            &[],
            json!({
                "installation_id": installation_id,
                "pairing_code": "dead",
                "nonce": "nonce_x",
                "signature": "ed25519-sig:x",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "path={path}");
    }
}
