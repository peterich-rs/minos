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
async fn get_thread_events_paginates() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret) = paired_pair(&state).await;
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a",
        AgentName::Codex,
        &mac_id.to_string(),
        100,
    )
    .await
    .unwrap();
    // Seed a `thread/started` event — the codex translator yields a
    // `ThreadOpened` UI event for this without prerequisite state, so the
    // assertion below can confirm the helper actually translates.
    minos_backend::store::raw_events::insert_if_absent(
        &state.store,
        "thr_a",
        1,
        AgentName::Codex,
        &serde_json::json!({
            "method":"thread/started",
            "params":{"threadId":"thr_a","createdAtMs":100}
        }),
        100,
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads/thr_a/events?limit=10")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::ReadThreadResponse = serde_json::from_value(body).unwrap();
    assert!(!resp.ui_events.is_empty());
}

#[tokio::test]
async fn get_thread_last_seq_returns_max() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret) = paired_pair(&state).await;
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a",
        AgentName::Codex,
        &mac_id.to_string(),
        100,
    )
    .await
    .unwrap();
    minos_backend::store::raw_events::insert_if_absent(
        &state.store,
        "thr_a",
        7,
        AgentName::Codex,
        &serde_json::json!({"method":"x"}),
        100,
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads/thr_a/last_seq")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::GetThreadLastSeqResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.last_seq, 7);
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
