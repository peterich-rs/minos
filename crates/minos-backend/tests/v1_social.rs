use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::{router, test_support::backend_state, test_support::TEST_JWT_SECRET};
use minos_backend::session::SessionHandle;
use minos_backend::store::{
    agent_sessions, device_installations, durable_event_log, host_commands, host_links, raw_events,
    sessions, social,
};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::Envelope;
use pretty_assertions::assert_eq;
use std::time::Duration;

mod common;

fn authed_request(
    method: Method,
    uri: &str,
    device_id: DeviceId,
    account_id: &str,
    body: Body,
) -> Request<Body> {
    let token = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        account_id,
        &device_id.to_string(),
    )
    .unwrap();
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("x-device-id", device_id.to_string())
        .body(body)
        .unwrap()
}

async fn seed_host_pair_for_account(
    state: &minos_backend::http::BackendState,
    account_id: &str,
    mobile_device_id: DeviceId,
) -> DeviceId {
    let host_device_id = DeviceId::new();
    device_installations::insert_device(
        &state.store,
        host_device_id,
        "Mac",
        DeviceRole::AgentHost,
        0,
    )
    .await
    .unwrap();
    device_installations::insert_device(
        &state.store,
        mobile_device_id,
        "iPhone",
        DeviceRole::MobileClient,
        0,
    )
    .await
    .unwrap();
    // host account_id stays NULL (kind=host CHECK)
    device_installations::set_account_id(&state.store, &mobile_device_id, account_id)
        .await
        .unwrap();
    host_links::insert_pair(
        &state.store,
        host_device_id,
        account_id,
        mobile_device_id,
        0,
    )
    .await
    .unwrap();
    host_device_id
}

fn seed_live_host_session(
    state: &minos_backend::http::BackendState,
    host_device_id: DeviceId,
    account_id: &str,
) -> tokio::sync::mpsc::Receiver<Envelope> {
    let (handle, outbox_rx) = SessionHandle::new(host_device_id, DeviceRole::AgentHost);
    handle.set_account_id(account_id.to_string());
    state.registry.insert(handle);
    outbox_rx
}

#[tokio::test]
async fn register_and_update_agent_persist_workspace_path() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/agents",
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "name": "Codex",
                    "description": "Assistant",
                    "runtime_agent": "codex",
                    "model": "gpt-5",
                    "workspace_path": "~/develop/minos"
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["workspace_path"], "~/develop/minos");
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!("/v1/agents/{agent_id}/update"),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "name": "Codex Writer",
                    "description": "Assistant",
                    "runtime_agent": "codex",
                    "model": "gpt-5.1",
                    "workspace_path": "/Users/example/minos"
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["workspace_path"], "/Users/example/minos");

    let row = social::get_agent(&state.store, &agent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.name, "Codex Writer");
    assert_eq!(row.workspace_path.as_deref(), Some("/Users/example/minos"));
}

fn deterministic_uuid(namespace: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};

    let mut value = namespace.to_string();
    for part in parts {
        value.push(':');
        value.push_str(part);
    }
    let digest = Sha256::digest(value.as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn expected_social_start_session_id(
    account_id: &str,
    origin_message_id: &str,
    agent_id: &str,
) -> String {
    // Must match multi-@ fan-out client_request_id: origin×agent.
    deterministic_uuid(
        "agent-session-start",
        &[
            account_id,
            &format!("social-start-{origin_message_id}:{agent_id}"),
        ],
    )
}

fn expected_social_send_turn_id(
    session_id: &str,
    origin_message_id: &str,
    agent_id: &str,
) -> String {
    deterministic_uuid(
        "agent-session-send-input",
        &[
            session_id,
            &format!("social-send-{origin_message_id}:{agent_id}"),
        ],
    )
}

async fn assert_agent_start_host_command(
    pool: &sqlx::SqlitePool,
    host_device_id: DeviceId,
    requester_account_id: &str,
    session_id: &str,
    _origin_message_id: &str,
    expected_text: &str,
    expected_conversation_id: &str,
    expected_agent_id: &str,
    expected_workspace_path: Option<&str>,
) {
    let row = host_commands::get(pool, &format!("cmd-agent-session-start-{session_id}"))
        .await
        .unwrap()
        .expect("host command should be recorded for social agent session start");

    assert_eq!(row.host_installation_id, host_device_id);
    assert_eq!(row.method, "agent_session.start");
    assert_eq!(row.agent_session_id.as_deref(), Some(session_id));
    assert_eq!(
        row.requested_by_account_id.as_deref(),
        Some(requester_account_id)
    );
    assert_eq!(row.status, host_commands::HostCommandStatus::Pending);
    assert_eq!(row.response_json, None);
    assert_eq!(row.params_json["session_id"], session_id);
    assert_eq!(row.params_json["agent_id"], expected_agent_id);
    assert_eq!(row.params_json["runtime_agent"], "codex");
    assert_eq!(row.params_json["conversation_id"], expected_conversation_id);
    assert_eq!(row.params_json["initial_user_message"], expected_text);
    // B4: origin_message_id must reach host so daemon pins agent-result suffix.
    assert_eq!(
        row.params_json["origin_message_id"], _origin_message_id,
        "agent_session.start must carry origin_message_id"
    );
    assert_eq!(
        row.params_json["workspace"],
        expected_workspace_path.unwrap_or_default()
    );
    assert_eq!(
        row.params_json["workspace_path"],
        expected_workspace_path
            .map(|path| serde_json::json!(path))
            .unwrap_or(serde_json::Value::Null)
    );
}

async fn assert_agent_send_host_command(
    pool: &sqlx::SqlitePool,
    host_device_id: DeviceId,
    requester_account_id: &str,
    session_id: &str,
    origin_message_id: &str,
    agent_id: &str,
    expected_text: &str,
) {
    let turn_id = expected_social_send_turn_id(session_id, origin_message_id, agent_id);
    let row = host_commands::get(pool, &format!("cmd-agent-session-send-{turn_id}"))
        .await
        .unwrap()
        .expect("host command should be recorded for social agent input");

    assert_eq!(row.host_installation_id, host_device_id);
    assert_eq!(row.method, "agent_session.send_input");
    assert_eq!(row.agent_session_id.as_deref(), Some(session_id));
    assert_eq!(
        row.requested_by_account_id.as_deref(),
        Some(requester_account_id)
    );
    assert_eq!(row.status, host_commands::HostCommandStatus::Pending);
    assert_eq!(row.response_json, None);
    assert_eq!(row.params_json["session_id"], session_id);
    assert_eq!(row.params_json["turn_id"], turn_id);
    assert_eq!(row.params_json["text"], expected_text);
    assert_eq!(row.params_json["mentions"], serde_json::json!([]));
    assert_eq!(
        row.params_json["origin_message_id"], origin_message_id,
        "agent_session.send_input must carry origin_message_id"
    );
}

async fn assert_formal_agent_session(
    state: &minos_backend::http::BackendState,
    session_id: &str,
    conversation_id: &str,
    host_device_id: DeviceId,
    agent_id: &str,
) {
    let session = agent_sessions::get(&state.store, session_id)
        .await
        .unwrap()
        .expect("social agent dispatch should create a formal agent session");
    let host_device_id = host_device_id.to_string();
    assert_eq!(session.conversation_id, conversation_id);
    assert_eq!(
        session.host_device_id.as_deref(),
        Some(host_device_id.as_str())
    );
    assert_eq!(session.agent_id.as_deref(), Some(agent_id));
    assert_eq!(session.status, "pending");
}

async fn wait_for_message_count(
    state: &minos_backend::http::BackendState,
    conversation_id: &str,
    expected: usize,
) -> Vec<social::ChatMessageRow> {
    // Must exceed GROUP_COMPLETION_SEQ_STABLE (2s) plus poll overhead.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rows = social::list_messages(&state.store, conversation_id, None, None, 50)
            .await
            .unwrap();
        if rows.len() >= expected {
            return rows;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for social reply"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn social_friend_and_chat_flow_round_trips() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let bob_device = DeviceId::new();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/profiles/self",
            alice_device,
            &alice.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "alice@example.com");
    assert_eq!(body["minos_id"], alice.minos_id);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/profiles/search",
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "minos_id": &bob.minos_id[..4] }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["users"].as_array().unwrap().len(), 1);
    assert_eq!(body["users"][0]["minos_id"], bob.minos_id);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/friend-requests",
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "target_minos_id": bob.minos_id }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let request_id = body["request_id"].as_str().unwrap().to_string();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/friend-requests/query",
            bob_device,
            &bob.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["incoming"].as_array().unwrap().len(), 1);

    let (status, _) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!("/v1/friend-requests/{request_id}/accept"),
            bob_device,
            &bob.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/friends/query",
            alice_device,
            &alice.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["friends"].as_array().unwrap().len(), 1);
    assert_eq!(body["friends"][0]["minos_id"], bob.minos_id);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/conversations/direct",
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "friend_account_id": bob.account_id }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let conversation_id = body["conversation_id"].as_str().unwrap();

    let (status, _) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!("/v1/conversations/{conversation_id}/messages"),
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "text": "hello bob" }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!("/v1/conversations/{conversation_id}/messages/query"),
            bob_device,
            &bob.account_id,
            Body::from(r#"{"limit":50}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["text"], "hello bob");
    assert_eq!(body["messages"][0]["sender"]["minos_id"], alice.minos_id);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/conversations/query",
            bob_device,
            &bob.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(body["conversations"][0]["conversation_id"], conversation_id);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!("/v1/conversations/{conversation_id}/members/query"),
            bob_device,
            &bob.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn conversation_command_aliases_list_and_send_message() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let conversation = social::create_group_conversation(
        &state.store,
        &alice.account_id,
        "Alias Conversation",
        &[],
        100,
    )
    .await
    .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/conversations/list",
            alice_device,
            &alice.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["conversations"][0]["conversation_id"],
        conversation.conversation_id
    );

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/conversations/send-message",
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "conversation_id": conversation.conversation_id,
                    "text": "hello via command route",
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["text"], "hello via command route");

    let rows = social::list_messages(&state.store, &conversation.conversation_id, None, None, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text, "hello via command route");
}

#[tokio::test]
async fn send_message_publishes_account_realtime_event_with_thin_digest() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let conversation = social::create_group_conversation(
        &state.store,
        &alice.account_id,
        "Realtime",
        std::slice::from_ref(&bob.account_id),
        100,
    )
    .await
    .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "text": "live hello" }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let message_id = body["message_id"].as_str().unwrap();

    for account_id in [&alice.account_id, &bob.account_id] {
        let topic = format!("account:{account_id}");
        let events = durable_event_log::read_topic_after(&state.store, "account", &topic, 0, 10)
            .await
            .unwrap();
        let event = events
            .iter()
            .find(|event| event.payload_json["message_id"] == message_id)
            .expect("account topic should receive the social message event");
        assert_eq!(
            event.payload_json["kind"],
            "account_conversation_message_appended"
        );
        assert_eq!(
            event.payload_json["conversation_id"],
            conversation.conversation_id
        );
        // R3: account topic carries thin digest only (no nested full message).
        assert_eq!(event.payload_json["message_id"], message_id);
        assert_eq!(event.payload_json["preview"], "live hello");
        assert!(event.payload_json.get("message").is_none());
        assert!(event.payload_json.get("sender_display_name").is_some());
    }
}

#[tokio::test]
async fn delete_conversation_stops_running_agent_session_and_hides_for_caller() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let host_device_id_string = host_device_id.to_string();
    let conversation =
        social::create_group_conversation(&state.store, &alice.account_id, "Delete Me", &[], 100)
            .await
            .unwrap();
    let agent = social::register_agent(
        &state.store,
        &alice.account_id,
        "Codex",
        "Assistant",
        "codex",
        "gpt-5",
        None,
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();
    agent_sessions::create(
        &state.store,
        "sess-delete-1",
        &conversation.conversation_id,
        None,
        Some(&host_device_id_string),
        Some(&agent.agent_id),
        "running",
        100,
        None,
    )
    .await
    .unwrap();

    let (status, _) = common::send(
        &mut app,
        authed_request(
            Method::DELETE,
            &format!("/v1/conversations/{}", conversation.conversation_id),
            alice_device,
            &alice.account_id,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let stop_command = host_commands::get(&state.store, "cmd-agent-session-stop-sess-delete-1")
        .await
        .unwrap()
        .expect("delete should enqueue stop for running agent session");
    assert_eq!(stop_command.host_installation_id, host_device_id);
    assert_eq!(stop_command.method, "agent_session.stop");
    assert_eq!(stop_command.params_json["session_id"], "sess-delete-1");
    assert!(
        social::get_conversation(&state.store, &conversation.conversation_id)
            .await
            .unwrap()
            .is_some()
    );
    let stopped_session = agent_sessions::get(&state.store, "sess-delete-1")
        .await
        .unwrap()
        .expect("session row should remain available for stop tracking");
    assert_eq!(stopped_session.status, "stopping");

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/conversations/query",
            alice_device,
            &alice.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["conversations"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn delete_direct_conversation_hides_only_for_requesting_account() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let bob_device = DeviceId::new();
    let conversation = social::ensure_direct_conversation(
        &state.store,
        &alice.account_id,
        &alice.account_id,
        &bob.account_id,
        100,
    )
    .await
    .unwrap();
    social::insert_message(
        &state.store,
        &conversation.conversation_id,
        &bob.account_id,
        "hello",
        101,
        None,
        &[],
    )
    .await
    .unwrap();

    let (status, _) = common::send(
        &mut app,
        authed_request(
            Method::DELETE,
            &format!("/v1/conversations/{}", conversation.conversation_id),
            alice_device,
            &alice.account_id,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/conversations/query",
            alice_device,
            &alice.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["conversations"].as_array().unwrap().is_empty());

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/conversations/query",
            bob_device,
            &bob.account_id,
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["conversations"][0]["conversation_id"],
        conversation.conversation_id
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn group_mentions_dispatch_to_host_and_post_completed_agent_reply() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let _host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

    let conversation = social::create_group_conversation(
        &state.store,
        &alice.account_id,
        "Group",
        &[bob.account_id.clone()],
        100,
    )
    .await
    .unwrap();
    let agent = social::register_agent(
        &state.store,
        &alice.account_id,
        "Codex",
        "Assistant",
        "codex",
        "gpt-5",
        Some("/Users/example/minos"),
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "text": format!("@{} please help", agent.agent_id)
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let user_message_id = body["message_id"].as_str().unwrap().to_string();
    // Dispatch is async via AgentDispatchQueue — drain worker path for host RPC.
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    let session_id =
        expected_social_start_session_id(&alice.account_id, &user_message_id, &agent.agent_id);
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(session_id.as_str())
    );
    assert_agent_start_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        &session_id,
        &user_message_id,
        // Full body kept so multi-@ co-mentions remain visible to each agent.
        &format!("@{} please help", agent.agent_id),
        &conversation.conversation_id,
        &agent.agent_id,
        Some("/Users/example/minos"),
    )
    .await;
    assert_eq!(
        social::lookup_latest_session_id_for_conversation_agent(
            &state.store,
            &conversation.conversation_id,
            &agent.agent_id
        )
        .await
        .unwrap()
        .as_deref(),
        Some(session_id.as_str())
    );
    assert_formal_agent_session(
        &state,
        &session_id,
        &conversation.conversation_id,
        host_device_id,
        &agent.agent_id,
    )
    .await;

    sessions::upsert(
        &state.store,
        &session_id,
        AgentName::Codex,
        &host_device_id.to_string(),
        199,
    )
    .await
    .unwrap();

    raw_events::insert_if_absent(
        &state.store,
        &session_id,
        1,
        AgentName::Codex,
        &serde_json::json!({
            "method": "item/started",
            "params": {
                "item": { "type": "agentMessage", "id": "agent-msg-1" },
                "sessionId": session_id,
                "turnId": "turn-1"
            }
        }),
        200,
    )
    .await
    .unwrap();
    raw_events::insert_if_absent(
        &state.store,
        &session_id,
        2,
        AgentName::Codex,
        &serde_json::json!({
            "method": "item/agentMessage/delta",
            "params": {
                "itemId": "agent-msg-1",
                "delta": "Done"
            }
        }),
        201,
    )
    .await
    .unwrap();
    raw_events::insert_if_absent(
        &state.store,
        &session_id,
        3,
        AgentName::Codex,
        &serde_json::json!({
            "method": "turn/completed",
            "params": {
                "finishedAtMs": 202
            }
        }),
        202,
    )
    .await
    .unwrap();

    // Completion is event-driven on host ingest; tests that seed raw_events
    // directly must trigger projection (no poller).
    minos_backend::http::v1::social::try_project_completion_for_session(&state, &session_id).await;
    // Stable-seq path may re-check after GROUP_COMPLETION_SEQ_STABLE (2s).
    tokio::time::sleep(Duration::from_millis(2200)).await;
    minos_backend::http::v1::social::try_project_completion_for_session(&state, &session_id).await;

    let rows = wait_for_message_count(&state, &conversation.conversation_id, 2).await;
    let agent_reply = rows
        .iter()
        .find(|row| row.sender_type == "agent")
        .expect("watcher should persist an agent-authored social reply");
    assert_eq!(
        agent_reply.reply_to_message_id.as_deref(),
        Some(user_message_id.as_str())
    );
    assert_eq!(agent_reply.text, format!("@{} Done", alice.minos_id));
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &agent_reply.message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(session_id.as_str())
    );
    // B4 frozen id: agent-result:{conv}:{session}:{origin_message_id}
    assert_eq!(
        agent_reply.message_id,
        format!(
            "agent-result:{}:{}:{}",
            conversation.conversation_id, session_id, user_message_id
        )
    );

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "text": format!("@{} one more thing", agent.agent_id)
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let followup_message_id = body["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &followup_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(session_id.as_str())
    );
    assert_agent_send_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        &session_id,
        &followup_message_id,
        &agent.agent_id,
        &format!("@{} one more thing", agent.agent_id),
    )
    .await;
}

#[tokio::test]
async fn group_member_can_be_removed_by_existing_member() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let conversation = social::create_group_conversation(
        &state.store,
        &alice.account_id,
        "Group",
        &[bob.account_id.clone()],
        100,
    )
    .await
    .unwrap();

    let (status, _) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/members/remove",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "member_account_id": bob.account_id
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let members = social::list_conversation_members(&state.store, &conversation.conversation_id)
        .await
        .unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0], alice.account_id);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn direct_agent_conversation_auto_routes_and_reuses_reply_session() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let _host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

    let conversation =
        social::create_group_conversation(&state.store, &alice.account_id, "Agent DM", &[], 100)
            .await
            .unwrap();
    let agent = social::register_agent(
        &state.store,
        &alice.account_id,
        "Codex",
        "Assistant",
        "codex",
        "gpt-5",
        None,
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "text": "hello agent" }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first_user_message_id = body["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    let session_id = expected_social_start_session_id(
        &alice.account_id,
        &first_user_message_id,
        &agent.agent_id,
    );
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &first_user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(session_id.as_str())
    );
    assert_formal_agent_session(
        &state,
        &session_id,
        &conversation.conversation_id,
        host_device_id,
        &agent.agent_id,
    )
    .await;

    let prior_agent_message = social::insert_agent_message(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        "previous reply",
        150,
        Some(&first_user_message_id),
        &[],
    )
    .await
    .unwrap();
    social::bind_session_to_message(&state.store, &prior_agent_message.message_id, &session_id)
        .await
        .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "text": "follow up",
                    "reply_to_message_id": prior_agent_message.message_id,
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second_user_message_id = body["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &second_user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(session_id.as_str())
    );
    assert_agent_start_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        &session_id,
        &first_user_message_id,
        "hello agent",
        &conversation.conversation_id,
        &agent.agent_id,
        None,
    )
    .await;
    assert_agent_send_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        &session_id,
        &second_user_message_id,
        &agent.agent_id,
        "follow up",
    )
    .await;
}

/// Tests group chat reply session reuse: when a user replies to an agent's
/// reply message in a group conversation, the server looks up the session_id
/// bound to the replied-to message and forwards the new message using that
/// existing session (no new session creation).
///
/// Validates: Requirements 11.3 (reply to agent message reuses session),
///            11.5 (session binding persists across messages)
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn group_reply_to_agent_message_reuses_session() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    // Setup: alice owns the host, bob is a group member
    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let _host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

    // Create group conversation with an agent
    let conversation = social::create_group_conversation(
        &state.store,
        &alice.account_id,
        "Project Group",
        &[bob.account_id.clone()],
        100,
    )
    .await
    .unwrap();
    let agent = social::register_agent(
        &state.store,
        &alice.account_id,
        "Codex",
        "Assistant",
        "codex",
        "gpt-5",
        None,
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();

    // Step 1: Send initial @mention to create a session
    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "text": format!("@{} summarize the PR", agent.agent_id)
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_message_id = body["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    let session_id =
        expected_social_start_session_id(&alice.account_id, &user_message_id, &agent.agent_id);

    // Verify session binding on user message
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(session_id.as_str())
    );
    assert_agent_start_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        &session_id,
        &user_message_id,
        &format!("@{} summarize the PR", agent.agent_id),
        &conversation.conversation_id,
        &agent.agent_id,
        None,
    )
    .await;

    // Step 2: Simulate agent completing and posting a reply message
    // (In production, the completion watcher does this; here we insert directly)
    let agent_reply = social::insert_agent_message(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &format!("@{} Here is the PR summary: ...", alice.minos_id),
        200,
        Some(&user_message_id),
        &[],
    )
    .await
    .unwrap();
    // Bind the same session to the agent's reply (as the completion watcher would)
    social::bind_session_to_message(&state.store, &agent_reply.message_id, &session_id)
        .await
        .unwrap();

    // Step 3: User replies to the agent's reply message — should reuse session
    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "text": "can you also include the test coverage?",
                    "reply_to_message_id": agent_reply.message_id,
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let followup_message_id = body["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();

    // Verify session binding on the follow-up message
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &followup_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(session_id.as_str())
    );
    assert_agent_send_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        &session_id,
        &followup_message_id,
        &agent.agent_id,
        "can you also include the test coverage?",
    )
    .await;
}

/// Desktop-started sessions are formalized by Host ingest into `agent_sessions`
/// without necessarily writing `chat_messages.agent_session_id`. Mobile `@agent`
/// must reuse that formal session (send_input) instead of `agent_session.start`.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn mobile_at_agent_reuses_desktop_formal_session_without_chat_bind() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let _host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

    let conversation = social::create_group_conversation(
        &state.store,
        &alice.account_id,
        "Desktop first",
        &[],
        100,
    )
    .await
    .unwrap();
    // Host-runtime agent (same path as Desktop attach / Host ingest).
    let agent = social::ensure_host_runtime_agent(
        &state.store,
        &alice.account_id,
        "codex",
        "Codex",
        "",
        Some("/Users/example/minos"),
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();

    // Simulate Desktop-local session registered only via Host ingest formal row
    // (no chat_messages.agent_session_id bind).
    let desktop_session_id = "desktop-local-sess-abcdef12";
    agent_sessions::create(
        &state.store,
        desktop_session_id,
        &conversation.conversation_id,
        None,
        Some(&host_device_id.to_string()),
        Some(&agent.agent_id),
        "running",
        150,
        None,
    )
    .await
    .unwrap();

    // No chat bind: lookup_latest_session_id_for_conversation_agent is empty.
    assert_eq!(
        social::lookup_latest_session_id_for_conversation_agent(
            &state.store,
            &conversation.conversation_id,
            &agent.agent_id
        )
        .await
        .unwrap(),
        None
    );

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "text": "@codex continue the refactor" }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_message_id = body["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();

    // Bound to the existing Desktop session — not a new social-start UUID.
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some(desktop_session_id)
    );
    assert_ne!(
        expected_social_start_session_id(&alice.account_id, &user_message_id, &agent.agent_id),
        desktop_session_id
    );

    // Must enqueue send_input against the reused session, not agent_session.start.
    assert_agent_send_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        desktop_session_id,
        &user_message_id,
        &agent.agent_id,
        "@codex continue the refactor",
    )
    .await;
    assert!(
        host_commands::get(
            &state.store,
            &format!(
                "cmd-agent-session-start-{}",
                expected_social_start_session_id(
                    &alice.account_id,
                    &user_message_id,
                    &agent.agent_id
                )
            )
        )
        .await
        .unwrap()
        .is_none(),
        "must not start a new formal session when Desktop formal session exists"
    );
}

/// B3: no live host → message HTTP 200 + queue row pending (no host command).
#[tokio::test]
async fn agent_dispatch_queues_when_host_offline() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    // Pair host but do not seed a live registry session.
    let _host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;

    let conversation =
        social::create_group_conversation(&state.store, &alice.account_id, "Agent DM", &[], 100)
            .await
            .unwrap();
    let agent = social::register_agent(
        &state.store,
        &alice.account_id,
        "Codex",
        "Assistant",
        "codex",
        "gpt-5",
        None,
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "text": "hello agent" }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let origin = body["message_id"].as_str().unwrap().to_string();

    let row = minos_backend::store::agent_dispatch_queue::get_by_origin(&state.store, &origin)
        .await
        .unwrap()
        .expect("dispatch row should be pending after send");
    assert_eq!(
        row.status,
        minos_backend::store::agent_dispatch_queue::STATUS_PENDING
    );

    // Worker drain without live host requeues (still pending, not terminal yet).
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    let after = minos_backend::store::agent_dispatch_queue::get_by_origin(&state.store, &origin)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.status,
        minos_backend::store::agent_dispatch_queue::STATUS_PENDING
    );
    assert!(after.attempts >= 1);
    assert!(after.next_attempt_at_ms >= after.updated_at_ms);

    // No formal session bound until host is live.
    assert!(social::lookup_session_id_for_message(&state.store, &origin)
        .await
        .unwrap()
        .is_none());
}

/// B3: pending dispatch drains when host becomes online.
#[tokio::test]
async fn agent_dispatch_drains_when_host_comes_online() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;

    let conversation =
        social::create_group_conversation(&state.store, &alice.account_id, "Agent DM", &[], 100)
            .await
            .unwrap();
    let agent = social::register_agent(
        &state.store,
        &alice.account_id,
        "Codex",
        "Assistant",
        "codex",
        "gpt-5",
        None,
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(serde_json::json!({ "text": "hello agent" }).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let origin = body["message_id"].as_str().unwrap().to_string();

    // Offline drain → pending with future next_attempt (backoff).
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    let pending = minos_backend::store::agent_dispatch_queue::get_by_origin(&state.store, &origin)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pending.status,
        minos_backend::store::agent_dispatch_queue::STATUS_PENDING
    );
    assert!(
        pending.next_attempt_at_ms > pending.updated_at_ms,
        "backoff should push next_attempt into the future"
    );

    // Production host-online edge: force due for linked accounts (no fake requeue).
    let _host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);
    let forced = minos_backend::http::v1::social::on_host_online_force_agent_dispatch(
        &state,
        host_device_id,
    )
    .await
    .unwrap();
    assert!(forced >= 1, "host online must force-due pending dispatches");

    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();

    let done = minos_backend::store::agent_dispatch_queue::get_by_origin(&state.store, &origin)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        done.status,
        minos_backend::store::agent_dispatch_queue::STATUS_SUCCEEDED
    );
    let session_id = expected_social_start_session_id(&alice.account_id, &origin, &agent.agent_id);
    assert_agent_start_host_command(
        &state.store,
        host_device_id,
        &alice.account_id,
        &session_id,
        &origin,
        "hello agent",
        &conversation.conversation_id,
        &agent.agent_id,
        None,
    )
    .await;
}

/// B4: two rapid dispatches on same session → two agent bubbles with distinct ids.
#[tokio::test]
async fn two_rapid_dispatches_project_two_agent_bubbles() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let _host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

    let conversation = social::create_group_conversation(
        &state.store,
        &alice.account_id,
        "Group",
        &[bob.account_id.clone()],
        100,
    )
    .await
    .unwrap();
    let agent = social::register_agent(
        &state.store,
        &alice.account_id,
        "Codex",
        "Assistant",
        "codex",
        "gpt-5",
        None,
        100,
    )
    .await
    .unwrap();
    social::add_agent_to_conversation(
        &state.store,
        &conversation.conversation_id,
        &agent.agent_id,
        &alice.account_id,
        100,
    )
    .await
    .unwrap();

    // First mention → start session.
    let (status, body1) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "text": format!("@{} first", agent.agent_id)
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let origin1 = body1["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();
    let session_id = expected_social_start_session_id(&alice.account_id, &origin1, &agent.agent_id);

    // Second mention (reuse session via lookup) before first completion.
    let (status, body2) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            &format!(
                "/v1/conversations/{}/messages",
                conversation.conversation_id
            ),
            alice_device,
            &alice.account_id,
            Body::from(
                serde_json::json!({
                    "text": format!("@{} second", agent.agent_id)
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let origin2 = body2["message_id"].as_str().unwrap().to_string();
    minos_backend::http::v1::social::process_agent_dispatch_batch(&state)
        .await
        .unwrap();

    // Two watches on the same session.
    assert_eq!(
        state.completion_watches.list_for_session(&session_id).len(),
        2
    );

    sessions::upsert(
        &state.store,
        &session_id,
        AgentName::Codex,
        &host_device_id.to_string(),
        199,
    )
    .await
    .unwrap();

    // Turn 1 events (seq 1-3), turn 2 events (seq 4-6).
    for (seq, item_id, text, finished) in [
        (1u64, "m1", "answer-one", 200i64),
        (2, "m1", "answer-one", 201),
        (3, "m1", "answer-one", 202),
    ] {
        let payload = if seq == 1 {
            serde_json::json!({
                "method": "item/started",
                "params": {
                    "item": { "type": "agentMessage", "id": item_id },
                    "sessionId": session_id,
                    "turnId": "t1"
                }
            })
        } else if seq == 2 {
            serde_json::json!({
                "method": "item/agentMessage/delta",
                "params": { "itemId": item_id, "delta": text }
            })
        } else {
            serde_json::json!({
                "method": "turn/completed",
                "params": { "finishedAtMs": finished }
            })
        };
        raw_events::insert_if_absent(
            &state.store,
            &session_id,
            seq,
            AgentName::Codex,
            &payload,
            finished,
        )
        .await
        .unwrap();
        let _ = (item_id, text);
    }

    minos_backend::http::v1::social::try_project_completion_for_session(&state, &session_id).await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    minos_backend::http::v1::social::try_project_completion_for_session(&state, &session_id).await;

    // After turn1: second watch may still be pending (higher floor).
    // Feed turn2 events.
    for (seq, item_id, text, finished) in [
        (4u64, "m2", "answer-two", 300i64),
        (5, "m2", "answer-two", 301),
        (6, "m2", "answer-two", 302),
    ] {
        let payload = if seq == 4 {
            serde_json::json!({
                "method": "item/started",
                "params": {
                    "item": { "type": "agentMessage", "id": item_id },
                    "sessionId": session_id,
                    "turnId": "t2"
                }
            })
        } else if seq == 5 {
            serde_json::json!({
                "method": "item/agentMessage/delta",
                "params": { "itemId": item_id, "delta": text }
            })
        } else {
            serde_json::json!({
                "method": "turn/completed",
                "params": { "finishedAtMs": finished }
            })
        };
        raw_events::insert_if_absent(
            &state.store,
            &session_id,
            seq,
            AgentName::Codex,
            &payload,
            finished,
        )
        .await
        .unwrap();
        let _ = (item_id, text);
    }

    minos_backend::http::v1::social::try_project_completion_for_session(&state, &session_id).await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    minos_backend::http::v1::social::try_project_completion_for_session(&state, &session_id).await;

    let rows = wait_for_message_count(&state, &conversation.conversation_id, 4).await;
    let agent_rows: Vec<_> = rows.iter().filter(|r| r.sender_type == "agent").collect();
    assert_eq!(agent_rows.len(), 2, "expected two agent bubbles");
    let ids: std::collections::HashSet<_> =
        agent_rows.iter().map(|r| r.message_id.as_str()).collect();
    let expected1 = format!(
        "agent-result:{}:{}:{}",
        conversation.conversation_id, session_id, origin1
    );
    let expected2 = format!(
        "agent-result:{}:{}:{}",
        conversation.conversation_id, session_id, origin2
    );
    assert!(ids.contains(expected1.as_str()), "missing {expected1}");
    assert!(ids.contains(expected2.as_str()), "missing {expected2}");

    // Re-project is idempotent (no third bubble).
    minos_backend::http::v1::social::try_project_completion_for_session(&state, &session_id).await;
    let rows2 = social::list_messages(&state.store, &conversation.conversation_id, None, None, 50)
        .await
        .unwrap();
    assert_eq!(rows2.iter().filter(|r| r.sender_type == "agent").count(), 2);
}
