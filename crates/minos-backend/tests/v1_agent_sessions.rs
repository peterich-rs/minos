use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::test_support::TEST_JWT_SECRET;
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::store::{account_host_pairings, devices::insert_device};
use minos_domain::{DeviceId, DeviceRole};

mod common;

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

    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, host, &hash)
        .await
        .unwrap();

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

fn bearer_for(account_id: &str, device_id: DeviceId) -> String {
    jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        account_id,
        &device_id.to_string(),
    )
    .expect("test bearer signs cleanly")
}

fn authed_post(
    uri: &str,
    _ios_id: DeviceId,
    _secret: &minos_domain::DeviceSecret,
    auth_hdr: &str,
    body: serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", auth_hdr)
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn seed_session(
    state: &minos_backend::http::BackendState,
    account_id: &str,
    host_id: DeviceId,
    session_id: &str,
) -> (String, String) {
    let members = vec![account_id.to_string()];
    let conversation = minos_backend::store::social::create_group_conversation(
        &state.store,
        account_id,
        "Read Turns",
        &members,
        1_000,
    )
    .await
    .unwrap();
    let host_device_id = host_id.to_string();
    minos_backend::store::agent_sessions::create(
        &state.store,
        session_id,
        &conversation.conversation_id,
        None,
        Some(host_device_id.as_str()),
        Some("agent_codex"),
        "running",
        1_001,
        None,
    )
    .await
    .unwrap();

    (conversation.conversation_id, session_id.to_string())
}

#[tokio::test]
async fn read_turns_returns_paginated_turn_metadata() {
    let state = backend_state().await;
    let (host_id, ios_id, secret, account_id) =
        paired_pair_with_account(&state, "agent-turns-route@example.com").await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");

    seed_session(&state, &account_id, host_id, "sess_route_turns").await;
    for seq in 1..=3 {
        minos_backend::store::agent_turns::create(
            &state.store,
            &format!("turn_route_{seq}"),
            "sess_route_turns",
            seq,
            if seq == 1 { "user" } else { "assistant" },
            "completed",
            2_000 + seq,
            Some(2_100 + seq),
            Some(&format!("summary-{seq}")),
            None,
        )
        .await
        .unwrap();
    }

    let mut app = router(state);
    let req = authed_post(
        "/v1/agent-sessions/read-turns",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "session_id": "sess_route_turns",
            "after_turn_seq": 1,
            "limit": 2
        }),
    );
    let (status, body) = common::send(&mut app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session_id"], "sess_route_turns");
    assert_eq!(body["turn_id"], serde_json::Value::Null);
    assert_eq!(body["next_turn_seq"], 3);
    assert_eq!(body["turns"].as_array().unwrap().len(), 2);
    assert_eq!(body["turns"][0]["turn_seq"], 2);
    assert_eq!(body["turns"][1]["turn_seq"], 3);
    assert!(body["events"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn read_turns_returns_paginated_event_slice() {
    let state = backend_state().await;
    let (host_id, ios_id, secret, account_id) =
        paired_pair_with_account(&state, "agent-turn-events-route@example.com").await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");

    seed_session(&state, &account_id, host_id, "sess_route_events").await;
    minos_backend::store::agent_turns::create(
        &state.store,
        "turn_route_events",
        "sess_route_events",
        1,
        "assistant",
        "completed",
        2_000,
        Some(2_100),
        None,
        None,
    )
    .await
    .unwrap();
    for seq in 1..=3 {
        minos_backend::store::agent_turn_events::append(
            &state.store,
            "turn_route_events",
            seq,
            "agent_text_delta",
            &serde_json::json!({ "delta": format!("chunk-{seq}") }),
            3_000 + seq,
        )
        .await
        .unwrap();
    }

    let mut app = router(state);
    let req = authed_post(
        "/v1/agent-sessions/read-turns",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "turn_id": "turn_route_events",
            "after_event_seq": 1,
            "limit": 2
        }),
    );
    let (status, body) = common::send(&mut app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session_id"], "sess_route_events");
    assert_eq!(body["turn_id"], "turn_route_events");
    assert_eq!(body["next_event_seq"], 3);
    assert_eq!(body["events"].as_array().unwrap().len(), 2);
    assert_eq!(body["events"][0]["event_seq"], 2);
    assert_eq!(
        body["events"][0]["payload"],
        serde_json::json!({ "delta": "chunk-2" })
    );
    assert_eq!(body["events"][1]["event_seq"], 3);
    assert!(body["turns"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn read_turns_hides_sessions_from_non_members() {
    let state = backend_state().await;
    let (host_a, _ios_a, _secret_a, account_a) =
        paired_pair_with_account(&state, "agent-turns-owner@example.com").await;
    let (_host_b, ios_b, secret_b, account_b) =
        paired_pair_with_account(&state, "agent-turns-outsider@example.com").await;
    let bearer_b = bearer_for(&account_b, ios_b);
    let auth_hdr_b = format!("Bearer {bearer_b}");

    seed_session(&state, &account_a, host_a, "sess_hidden").await;

    let mut app = router(state);
    let req = authed_post(
        "/v1/agent-sessions/read-turns",
        ios_b,
        &secret_b,
        &auth_hdr_b,
        serde_json::json!({
            "session_id": "sess_hidden",
            "limit": 10
        }),
    );
    let (status, body) = common::send(&mut app, req).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "agent_session_not_found");
}

#[tokio::test]
async fn list_sessions_filters_by_conversation_and_project_scope() {
    let state = backend_state().await;
    let (host_id, ios_id, secret, account_id) =
        paired_pair_with_account(&state, "agent-sessions-list@example.com").await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");

    minos_backend::store::projects::create(
        &state.store,
        "proj-route-list",
        &account_id,
        "Project Route List",
        "project-route-list",
        999,
    )
    .await
    .unwrap();

    let (conversation_id, session_id) =
        seed_session(&state, &account_id, host_id, "sess_route_list").await;
    minos_backend::store::agent_sessions::assign_project_for_account(
        &state.store,
        &session_id,
        &account_id,
        Some("proj-route-list"),
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = authed_post(
        "/v1/agent-sessions/list",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "conversation_id": conversation_id,
            "limit": 10
        }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(body["sessions"][0]["session_id"], "sess_route_list");
    assert_eq!(body["sessions"][0]["project_id"], "proj-route-list");

    let req = authed_post(
        "/v1/agent-sessions/list",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "project_id": "proj-route-list",
            "limit": 10
        }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(body["sessions"][0]["session_id"], "sess_route_list");
}
