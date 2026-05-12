//! Integration tests for POST-first `/v1/me/*` query routes.
//!
//! `/v1/me/hosts` is bearer-only; iOS callers see every Mac paired to
//! their account. `/v1/me/peer` is secret-authenticated on the Mac host
//! rail and returns the most recently paired mobile device for that host.
//! `/v1/me/peers` is the host-side multi-device snapshot.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::store::{account_host_pairings, devices::insert_device};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret};

mod common;

async fn paired_host(
    state: &minos_backend::http::BackendState,
) -> (DeviceId, DeviceId, DeviceSecret) {
    let host = DeviceId::new();
    let mobile = DeviceId::new();
    insert_device(&state.store, host, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    insert_device(&state.store, mobile, "iPhone", DeviceRole::MobileClient, 0)
        .await
        .unwrap();

    let secret = DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, host, &hash)
        .await
        .unwrap();

    let account =
        minos_backend::store::accounts::create(&state.store, "me-test@example.com", "phc")
            .await
            .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &host, &account.account_id)
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &mobile, &account.account_id)
        .await
        .unwrap();
    account_host_pairings::insert_pair(&state.store, host, &account.account_id, mobile, 123)
        .await
        .unwrap();

    (host, mobile, secret)
}

fn peer_request(host: DeviceId, secret: &DeviceSecret) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/v1/me/peer/query")
        .header("content-type", "application/json")
        .header("x-device-id", host.to_string())
        .header("x-device-role", "agent-host")
        .header("x-device-secret", secret.as_str())
        .body(Body::from("{}"))
        .unwrap()
}

fn post_query_request(uri: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}

#[tokio::test]
async fn get_me_peer_returns_current_mobile_peer() {
    let state = backend_state().await;
    let (host, mobile, secret) = paired_host(&state).await;
    let mut app = router(state.clone());

    let req = peer_request(host, &secret);
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["peer_device_id"], mobile.to_string());
    assert_eq!(body["peer_name"], "iPhone");
    assert_eq!(body["paired_at_ms"], 123);
}

#[tokio::test]
async fn get_me_peer_without_pair_returns_not_paired() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let host = DeviceId::new();
    insert_device(&state.store, host, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    let secret = DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, host, &hash)
        .await
        .unwrap();

    let req = peer_request(host, &secret);
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_paired");
}

#[tokio::test]
async fn get_me_peer_route_rejects_legacy_get() {
    let state = backend_state().await;
    let (host, _mobile, secret) = paired_host(&state).await;
    let mut app = router(state.clone());

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .header("x-device-id", host.to_string())
        .header("x-device-role", "agent-host")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn get_me_hosts_without_bearer_returns_401() {
    let state = backend_state().await;
    let mut app = router(state);

    let req = post_query_request("/v1/me/hosts/query", Body::from("{}"));
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn post_me_hosts_query_returns_all_paired_hosts() {
    let state = backend_state().await;
    let (host, mobile, _secret) = paired_host(&state).await;
    let account =
        minos_backend::store::accounts::find_by_email(&state.store, "me-test@example.com")
            .await
            .unwrap()
            .unwrap();
    let token = minos_backend::auth::jwt::sign(
        minos_backend::http::test_support::TEST_JWT_SECRET.as_bytes(),
        &account.account_id,
        &mobile.to_string(),
    )
    .unwrap();
    let mut app = router(state.clone());

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/me/hosts/query")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-device-id", mobile.to_string())
        .body(Body::from("{}"))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hosts"].as_array().unwrap().len(), 1);
    assert_eq!(body["hosts"][0]["host_device_id"], host.to_string());
}

#[tokio::test]
async fn post_me_peers_query_returns_all_host_mobile_rows() {
    let state = backend_state().await;
    let (host, mobile, secret) = paired_host(&state).await;
    let mut app = router(state.clone());

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/me/peers/query")
        .header("content-type", "application/json")
        .header("x-device-id", host.to_string())
        .header("x-device-role", "agent-host")
        .header("x-device-secret", secret.as_str())
        .body(Body::from("{}"))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["peers"].as_array().unwrap().len(), 1);
    assert_eq!(body["peers"][0]["mobile_device_id"], mobile.to_string());
    assert_eq!(body["peers"][0]["mobile_device_name"], "iPhone");
    assert_eq!(body["peers"][0]["account_email"], "me-test@example.com");
    assert_eq!(body["peers"][0]["paired_at_ms"], 123);
    assert_eq!(body["peers"][0]["online"], false);
}

#[tokio::test]
async fn delete_me_peer_removes_only_targeted_mobile_row() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let host = DeviceId::new();
    let mobile_a = DeviceId::new();
    let mobile_b = DeviceId::new();
    insert_device(&state.store, host, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    insert_device(
        &state.store,
        mobile_a,
        "Alice iPhone",
        DeviceRole::MobileClient,
        0,
    )
    .await
    .unwrap();
    insert_device(
        &state.store,
        mobile_b,
        "Bob iPhone",
        DeviceRole::MobileClient,
        0,
    )
    .await
    .unwrap();

    let secret = DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, host, &hash)
        .await
        .unwrap();

    let account_a =
        minos_backend::store::accounts::create(&state.store, "alice@example.com", "phc")
            .await
            .unwrap();
    let account_b = minos_backend::store::accounts::create(&state.store, "bob@example.com", "phc")
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &host, &account_a.account_id)
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &mobile_a, &account_a.account_id)
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &mobile_b, &account_b.account_id)
        .await
        .unwrap();
    account_host_pairings::insert_pair(&state.store, host, &account_a.account_id, mobile_a, 100)
        .await
        .unwrap();
    account_host_pairings::insert_pair(&state.store, host, &account_b.account_id, mobile_b, 200)
        .await
        .unwrap();

    let req = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/v1/me/peers/{}", mobile_a))
        .header("x-device-id", host.to_string())
        .header("x-device-role", "agent-host")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let rows = account_host_pairings::list_accounts_for_host(&state.store, host)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].paired_via_device_id, mobile_b);
    assert_eq!(rows[0].mobile_account_id, account_b.account_id);
}
