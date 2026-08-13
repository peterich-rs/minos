//! Retired QR pairing HTTP surface must not be mounted.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use minos_backend::http;
use minos_backend::http::test_support::backend_state;
use minos_domain::DeviceId;
use serde_json::json;

mod common;

async fn post(app: &mut axum::Router, path: &str, body: serde_json::Value) -> StatusCode {
    let (status, _) = common::send(
        app,
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await;
    status
}

#[tokio::test]
async fn retired_pairing_routes_return_404() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let host_id = DeviceId::new();

    assert_eq!(
        post(&mut app, "/v1/pairing/tokens", json!({})).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            &mut app,
            "/v1/pairing/consume",
            json!({"pairing_token": "x"}),
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            &mut app,
            "/v1/pairing/confirm",
            json!({"pairing_code": "x"}),
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            &mut app,
            "/v1/pairing/revoke",
            json!({"host_device_id": host_id.to_string()}),
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(&mut app, "/v1/pairing/list-hosts", json!({})).await,
        StatusCode::NOT_FOUND
    );

    let (status, _) = common::send(
        &mut app,
        Request::builder()
            .method("DELETE")
            .uri(format!("/v1/pairings/{host_id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
