//! Integration tests for the supported public pairing surface.
//!
//! The formal request-code, confirm, redeem, list-hosts, and revoke flows
//! now live in `formal_host_contract.rs` and `formal_account_contract.rs`.
//! This suite keeps coverage for the retained delete route and verifies the
//! retired legacy bootstrap routes stay unmounted.

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::test_support::TEST_JWT_SECRET;
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::store::device_installations::insert_device;
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::{Envelope, EventKind};

mod common;

fn sign_bearer(device_id: DeviceId, account_id: &str) -> String {
    jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        account_id,
        &device_id.to_string(),
    )
    .expect("test bearer signs cleanly")
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
async fn legacy_pairing_tokens_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);
    let device_id = DeviceId::new();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/tokens")
        .header("x-device-id", device_id.to_string())
        .header("x-device-role", "agent-host")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"host_display_name":"Fan's Mac"}"#))
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn legacy_pairing_consume_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);
    let consumer_id = DeviceId::new();
    let bearer = sign_bearer(consumer_id, "legacy-pairing-account");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", consumer_id.to_string())
        .header("x-device-role", "mobile-client")
        .header("authorization", format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"token":"legacy","device_name":"iPhone"}"#))
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_pairing_route_removes_host_pairing() {
    let state = backend_state().await;
    let host_id = DeviceId::new();
    let mobile_id = DeviceId::new();
    let account =
        minos_backend::store::accounts::create(&state.store, "delete-pair@example.com", "phc")
            .await
            .unwrap();

    insert_device(&state.store, host_id, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    insert_device(
        &state.store,
        mobile_id,
        "iPhone",
        DeviceRole::MobileClient,
        0,
    )
    .await
    .unwrap();
    minos_backend::store::device_installations::set_account_id(
        &state.store,
        &mobile_id,
        &account.account_id,
    )
    .await
    .unwrap();
    minos_backend::store::host_links::insert_pair(
        &state.store,
        host_id,
        &account.account_id,
        mobile_id,
        0,
    )
    .await
    .unwrap();

    let mut host_outbox = seed_live_session(&state, host_id, DeviceRole::AgentHost);
    let bearer = sign_bearer(mobile_id, &account.account_id);
    let mut app = router(state.clone());
    let req = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/v1/pairings/{host_id}"))
        .header("x-device-id", mobile_id.to_string())
        .header("authorization", format!("Bearer {bearer}"))
        .body(Body::empty())
        .unwrap();

    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body={body}");
    assert_eq!(body, serde_json::Value::Null);
    assert!(
        !minos_backend::store::host_links::exists(&state.store, host_id, &account.account_id)
            .await
            .unwrap()
    );

    let frame = host_outbox
        .recv()
        .await
        .expect("host receives unpaired event");
    match frame {
        Envelope::Event {
            event: EventKind::Unpaired,
            ..
        } => {}
        other => panic!("expected Event::Unpaired, got {other:?}"),
    }
}
