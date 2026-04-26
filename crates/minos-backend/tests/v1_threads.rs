use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::store::{devices::insert_device, pairings::insert_pairing};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::ListThreadsResponse;

mod common;

async fn paired_pair(
    state: &minos_backend::http::BackendState,
) -> (DeviceId, DeviceId, minos_domain::DeviceSecret) {
    let mac = DeviceId::new();
    let ios = DeviceId::new();
    insert_device(&state.store, mac, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    insert_device(&state.store, ios, "iPhone", DeviceRole::IosClient, 0)
        .await
        .unwrap();

    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, ios, &hash)
        .await
        .unwrap();
    insert_pairing(&state.store, mac, ios, 0).await.unwrap();
    (mac, ios, secret)
}

#[tokio::test]
async fn get_threads_returns_owner_scoped_list() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret) = paired_pair(&state).await;
    // Seed two threads owned by the Mac.
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a",
        AgentName::Codex,
        &mac_id.to_string(),
        100,
    )
    .await
    .unwrap();
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_b",
        AgentName::Claude,
        &mac_id.to_string(),
        300,
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads?limit=50")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: ListThreadsResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.threads.len(), 2);
}

#[tokio::test]
async fn get_threads_unpaired_returns_401() {
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::IosClient, 0)
        .await
        .unwrap();
    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, id, &hash)
        .await
        .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads?limit=10")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}
