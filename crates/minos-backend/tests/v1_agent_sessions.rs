use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::test_support::TEST_JWT_SECRET;
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::store::{device_installations::insert_device, host_links, social};
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

    let account = minos_backend::store::accounts::create(&state.store, email)
        .await
        .unwrap();
    // host account_id stays NULL (kind=host CHECK)
    minos_backend::store::device_installations::set_account_id(
        &state.store,
        &ios,
        &account.account_id,
    )
    .await
    .unwrap();
    host_links::insert_pair(&state.store, host, &account.account_id, ios, 0)
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
        None,
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
            "limit": 10
        }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(body["sessions"][0]["session_id"], "sess_route_list");

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

#[tokio::test]
async fn start_session_dispatches_host_command_and_persists_session() {
    let state = backend_state().await;
    let (host_id, ios_id, secret, account_id) =
        paired_pair_with_account(&state, "agent-session-start@example.com").await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");

    let conversation = social::create_group_conversation(
        &state.store,
        &account_id,
        "Start Session",
        &[account_id.clone()],
        1_000,
    )
    .await
    .unwrap();
    let agent = social::register_agent(
        &state.store,
        &account_id,
        "Codex",
        "assistant",
        "codex",
        "gpt-5.4",
        None,
        1_001,
    )
    .await
    .unwrap();
    let pool = state.store.sqlite_pool_cloned().unwrap();

    let mut app = router(state.clone());
    let req = authed_post(
        "/v1/agent-sessions/start",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "conversation_id": conversation.conversation_id,
            "agent_id": agent.agent_id,
            "workspace_path": "/Users/example/my-app",
            "initial_user_message": "hello from route",
            "client_request_id": "route-start-1"
        }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let session_id = body["session_id"].as_str().unwrap().to_string();
    let host_command_id = body["host_command_id"].as_str().unwrap().to_string();
    let initial_turn_id = body["initial_turn_id"].as_str().unwrap().to_string();
    assert!(!session_id.is_empty());
    assert_eq!(body["conversation_id"], conversation.conversation_id);
    assert_eq!(body["host_installation_id"], host_id.to_string());
    assert!(body["started_at_ms"].as_i64().unwrap() >= 1_000);
    assert_eq!(body["initial_turn_id"], initial_turn_id);

    let session = minos_backend::store::agent_sessions::get(&state.store, &session_id)
        .await
        .unwrap()
        .unwrap();
    let host_id_string = host_id.to_string();
    assert_eq!(session.conversation_id, conversation.conversation_id);
    assert_eq!(
        session.host_device_id.as_deref(),
        Some(host_id_string.as_str())
    );
    assert_eq!(session.agent_id.as_deref(), Some(agent.agent_id.as_str()));
    assert_eq!(session.status, "pending");

    let turns =
        minos_backend::store::agent_turns::list_for_session(&state.store, &session_id, None, 10)
            .await
            .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_seq, 1);
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[0].turn_id, initial_turn_id);
    assert_eq!(turns[0].summary_text.as_deref(), Some("hello from route"));

    let host_command = minos_backend::store::host_commands::get(&state.store, &host_command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(host_command.method, "agent_session.start");
    assert_eq!(
        host_command.agent_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(
        host_command.status,
        minos_backend::store::host_commands::HostCommandStatus::Pending
    );
    assert_eq!(host_command.params_json["session_id"], session_id);
    assert_eq!(host_command.params_json["runtime_agent"], "codex");
    assert_eq!(
        host_command.params_json["workspace"],
        "/Users/example/my-app"
    );
    assert_eq!(
        host_command.params_json["workspace_path"],
        "/Users/example/my-app"
    );
    assert_eq!(
        host_command.params_json["conversation_id"],
        conversation.conversation_id
    );
    assert_eq!(host_command.params_json["agent_id"], agent.agent_id);
    assert_eq!(
        host_command.params_json["initial_user_message"],
        "hello from route"
    );

    let session_events = minos_backend::store::durable_event_log::read_topic_after(
        &state.store,
        "agent_session",
        &format!("agent_session:{session_id}"),
        0,
        10,
    )
    .await
    .unwrap();
    assert_eq!(session_events.len(), 1);
    assert_eq!(
        session_events[0].payload_json["kind"],
        "agent_session_started"
    );

    let host_events = minos_backend::store::durable_event_log::read_topic_after(
        &state.store,
        "host",
        &format!("host:{host_id}"),
        0,
        10,
    )
    .await
    .unwrap();
    assert_eq!(host_events.len(), 1);
    assert_eq!(host_events[0].payload_json["kind"], "host_command_issued");
    assert_eq!(host_events[0].payload_json["command_id"], host_command_id);

    let outbox_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM outbox_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(outbox_count, 2);
}

#[tokio::test]
async fn send_input_dispatches_to_existing_session_and_appends_turn() {
    let state = backend_state().await;
    let (host_id, ios_id, secret, account_id) =
        paired_pair_with_account(&state, "agent-session-send@example.com").await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");

    let (_, session_id) = seed_session(&state, &account_id, host_id, "sess_route_send").await;
    let agent = social::register_agent(
        &state.store,
        &account_id,
        "Codex",
        "assistant",
        "codex",
        "gpt-5.4",
        None,
        1_001,
    )
    .await
    .unwrap();
    let pool = state.store.sqlite_pool_cloned().unwrap();
    sqlx::query("UPDATE agent_sessions SET agent_id = ? WHERE session_id = ?")
        .bind(&agent.agent_id)
        .bind(&session_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut app = router(state.clone());
    let req = authed_post(
        "/v1/agent-sessions/send-input",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "session_id": session_id,
            "text": "follow-up from route",
            "client_request_id": "route-send-1"
        }),
    );
    let (status, body) = common::send(&mut app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session_id"], session_id);
    let turn_id = body["turn_id"].as_str().unwrap().to_string();
    assert_eq!(body["turn_seq"], 1);
    let host_command_id = format!("cmd-agent-session-send-{turn_id}");

    let turns =
        minos_backend::store::agent_turns::list_for_session(&state.store, &session_id, None, 10)
            .await
            .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_seq, 1);
    assert_eq!(turns[0].turn_id, turn_id);
    assert_eq!(
        turns[0].summary_text.as_deref(),
        Some("follow-up from route")
    );

    let host_command = minos_backend::store::host_commands::get(&state.store, &host_command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(host_command.method, "agent_session.send_input");
    assert_eq!(
        host_command.agent_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(host_command.params_json["session_id"], session_id);
    assert_eq!(host_command.params_json["turn_id"], turn_id);
    assert_eq!(host_command.params_json["text"], "follow-up from route");
    assert_eq!(
        host_command.status,
        minos_backend::store::host_commands::HostCommandStatus::Pending
    );

    let session_events = minos_backend::store::durable_event_log::read_topic_after(
        &state.store,
        "agent_session",
        &format!("agent_session:{session_id}"),
        0,
        10,
    )
    .await
    .unwrap();
    assert_eq!(session_events.len(), 1);
    assert_eq!(
        session_events[0].payload_json["kind"],
        "agent_turn_appended"
    );
    assert_eq!(session_events[0].payload_json["turn_id"], turn_id);

    let host_events = minos_backend::store::durable_event_log::read_topic_after(
        &state.store,
        "host",
        &format!("host:{host_id}"),
        0,
        10,
    )
    .await
    .unwrap();
    assert_eq!(host_events.len(), 1);
    assert_eq!(host_events[0].payload_json["kind"], "host_command_issued");
    assert_eq!(host_events[0].payload_json["command_id"], host_command_id);

    let outbox_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM outbox_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(outbox_count, 2);
}

#[tokio::test]
async fn stop_session_dispatches_close_session_and_marks_session_stopped() {
    let state = backend_state().await;
    let (host_id, ios_id, secret, account_id) =
        paired_pair_with_account(&state, "agent-session-stop@example.com").await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");

    let (_, session_id) = seed_session(&state, &account_id, host_id, "sess_route_stop").await;
    let pool = state.store.sqlite_pool_cloned().unwrap();

    let mut app = router(state.clone());
    let req = authed_post(
        "/v1/agent-sessions/stop",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "session_id": session_id,
        }),
    );
    let (status, body) = common::send(&mut app, req).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, serde_json::Value::Null);

    let session = minos_backend::store::agent_sessions::get(&state.store, &session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.status, "stopping");
    assert!(session.ended_at_ms.is_none());

    let host_command_id = format!("cmd-agent-session-stop-{session_id}");
    let host_command = minos_backend::store::host_commands::get(&state.store, &host_command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(host_command.method, "agent_session.stop");
    assert_eq!(
        host_command.agent_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(host_command.params_json["session_id"], session_id);
    assert_eq!(
        host_command.status,
        minos_backend::store::host_commands::HostCommandStatus::Pending
    );

    let host_events = minos_backend::store::durable_event_log::read_topic_after(
        &state.store,
        "host",
        &format!("host:{host_id}"),
        0,
        10,
    )
    .await
    .unwrap();
    assert_eq!(host_events.len(), 1);
    assert_eq!(host_events[0].payload_json["kind"], "host_command_issued");
    assert_eq!(host_events[0].payload_json["command_id"], host_command_id);

    let outbox_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM outbox_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(outbox_count, 1);
}
