use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::test_support::TEST_JWT_SECRET;
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::store::{account_host_pairings, devices::insert_device};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::ListThreadsResponse;

mod common;

/// Seed an account and a paired (Mac, iOS) where both device rows are
/// linked to the new account_id. Returns
/// `(mac_id, ios_id, ios_secret, account_id)`.
async fn paired_pair_with_account(
    state: &minos_backend::http::BackendState,
    email: &str,
) -> (DeviceId, DeviceId, minos_domain::DeviceSecret, String) {
    let host = DeviceId::new();
    let ios = DeviceId::new();
    insert_device(&state.store, host, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    insert_device(&state.store, ios, "iPhone", DeviceRole::MobileClient, 0)
        .await
        .unwrap();

    // After ADR-0020 the iOS rail is bearer-only and `secret_hash` stays
    // NULL; we no longer mint an iOS device secret. The Mac side is still
    // secret-bound, so we generate a Mac secret to keep the legacy
    // assertions and signature-compat callers happy.
    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, host, &hash)
        .await
        .unwrap();

    // Phase 2 Task 2.6 / ADR-0020: link both device rows to a real
    // account_id, then record the pair via the account_host_pairings table
    // (the legacy device-keyed `pairings` module has been retired).
    let account = minos_backend::store::accounts::create(&state.store, email, "phc")
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &host, &account.account_id)
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &ios, &account.account_id)
        .await
        .unwrap();
    account_host_pairings::insert_pair(&state.store, host, &account.account_id, ios, 0)
        .await
        .unwrap();

    (host, ios, secret, account.account_id)
}

/// Convenience: signed bearer JWT bound to the given (account_id, device_id).
fn bearer_for(account_id: &str, device_id: DeviceId) -> String {
    jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        account_id,
        &device_id.to_string(),
    )
    .expect("test bearer signs cleanly")
}

/// Backwards-compat shim used by the tests that don't care about
/// account scoping; they still need a paired pair + bearer to satisfy
/// the new threads-route requirements.
async fn paired_pair(
    state: &minos_backend::http::BackendState,
) -> (DeviceId, DeviceId, minos_domain::DeviceSecret, String) {
    paired_pair_with_account(state, "threads-test@example.com").await
}

#[tokio::test]
async fn get_threads_returns_owner_scoped_list() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
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
        .header("x-device-role", "mobile-client")
        .header("x-device-secret", secret.as_str())
        .header("authorization", &auth_hdr)
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
    let (mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
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
        .header("x-device-role", "mobile-client")
        .header("x-device-secret", secret.as_str())
        .header("authorization", &auth_hdr)
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
    let (mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
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
        .header("x-device-role", "mobile-client")
        .header("x-device-secret", secret.as_str())
        .header("authorization", &auth_hdr)
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::GetThreadLastSeqResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.last_seq, 7);
}

#[tokio::test]
async fn routing_threads_filtered_by_account() {
    // Phase 2 Task 2.6: list_threads must scope by the bearer's
    // account_id. With two paired pairs on two distinct accounts, an iOS
    // bearer for account A only sees threads owned by A's Mac.
    let state = backend_state().await;
    let (mac_a, ios_a, secret_a, account_a) =
        paired_pair_with_account(&state, "alice@example.com").await;
    let (mac_b, _ios_b, _secret_b, _account_b) =
        paired_pair_with_account(&state, "bob@example.com").await;
    // Seed threads for both Macs.
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a1",
        AgentName::Codex,
        &mac_a.to_string(),
        100,
    )
    .await
    .unwrap();
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a2",
        AgentName::Claude,
        &mac_a.to_string(),
        300,
    )
    .await
    .unwrap();
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_b1",
        AgentName::Codex,
        &mac_b.to_string(),
        500,
    )
    .await
    .unwrap();

    let bearer_a = bearer_for(&account_a, ios_a);
    let auth_hdr = format!("Bearer {bearer_a}");
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads?limit=50")
        .header("x-device-id", ios_a.to_string())
        .header("x-device-role", "mobile-client")
        .header("x-device-secret", secret_a.as_str())
        .header("authorization", &auth_hdr)
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: ListThreadsResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.threads.len(), 2, "iOS A must see only A's threads");
    let ids: Vec<&str> = resp.threads.iter().map(|t| t.thread_id.as_str()).collect();
    assert!(ids.contains(&"thr_a1"));
    assert!(ids.contains(&"thr_a2"));
    assert!(
        !ids.contains(&"thr_b1"),
        "B's thread must not leak across accounts"
    );
}

#[tokio::test]
async fn get_threads_without_bearer_returns_401() {
    // After ADR-0020 the iOS rail is bearer-only. A request without an
    // Authorization header is rejected with 401 regardless of any
    // x-device-secret presented (which is no longer consulted on the iOS
    // path).
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::MobileClient, 0)
        .await
        .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads?limit=10")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "mobile-client")
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}
