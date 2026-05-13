use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::{router, test_support::backend_state, test_support::TEST_JWT_SECRET};
use minos_backend::session::SessionHandle;
use minos_backend::store::{account_host_pairings, devices, raw_events, social, threads};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::{AgentDispatchRequest, Envelope};
use std::sync::Arc;
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
    devices::insert_device(
        &state.store,
        host_device_id,
        "Mac",
        DeviceRole::AgentHost,
        0,
    )
    .await
    .unwrap();
    devices::insert_device(
        &state.store,
        mobile_device_id,
        "iPhone",
        DeviceRole::MobileClient,
        0,
    )
    .await
    .unwrap();
    devices::set_account_id(&state.store, &host_device_id, account_id)
        .await
        .unwrap();
    devices::set_account_id(&state.store, &mobile_device_id, account_id)
        .await
        .unwrap();
    account_host_pairings::insert_pair(
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

fn spawn_agent_dispatch_responder(
    registry: Arc<minos_backend::session::SessionRegistry>,
    host_device_id: DeviceId,
    mut host_rx: tokio::sync::mpsc::Receiver<Envelope>,
    response_session_id: &str,
) -> tokio::task::JoinHandle<AgentDispatchRequest> {
    let response_session_id = response_session_id.to_string();
    tokio::spawn(async move {
        let frame = tokio::time::timeout(Duration::from_secs(1), host_rx.recv())
            .await
            .expect("host dispatch should arrive before timeout")
            .expect("host session should receive a forwarded rpc");
        let Envelope::Forwarded { from, payload, .. } = frame else {
            panic!("expected forwarded rpc envelope");
        };
        assert_eq!(payload["method"], "minos_agent_dispatch");
        let request: AgentDispatchRequest =
            serde_json::from_value(payload["params"].clone()).expect("dispatch params decode");

        registry
            .route(
                host_device_id,
                from,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": payload["id"].clone(),
                    "result": { "session_id": response_session_id }
                }),
            )
            .await
            .expect("host rpc response routes back to requester");
        request
    })
}

async fn wait_for_message_count(
    state: &minos_backend::http::BackendState,
    conversation_id: &str,
    expected: usize,
) -> Vec<social::ChatMessageRow> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let rows = social::list_messages(&state.store, conversation_id, None, 50)
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

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com", "phc")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com", "phc")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let bob_device = DeviceId::new();

    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::POST,
            "/v1/me/profile/query",
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
            "/v1/users/search/query",
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
#[allow(clippy::too_many_lines)]
async fn group_mentions_dispatch_to_host_and_post_completed_agent_reply() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com", "phc")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com", "phc")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

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

    let dispatch = spawn_agent_dispatch_responder(
        Arc::clone(&state.registry),
        host_device_id,
        host_rx,
        "sess-group-1",
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
                    "text": format!("@{} please help", agent.agent_id)
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let dispatch = dispatch.await.unwrap();
    let user_message_id = body["message_id"].as_str().unwrap().to_string();
    assert_eq!(dispatch.agent, AgentName::Codex);
    assert_eq!(dispatch.session_id, None);
    assert_eq!(dispatch.text, "please help");
    assert_eq!(
        dispatch.conversation_id.as_deref(),
        Some(conversation.conversation_id.as_str())
    );
    assert_eq!(
        dispatch.origin_message_id.as_deref(),
        Some(user_message_id.as_str())
    );
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some("sess-group-1")
    );

    threads::upsert(
        &state.store,
        "sess-group-1",
        AgentName::Codex,
        &host_device_id.to_string(),
        199,
    )
    .await
    .unwrap();

    raw_events::insert_if_absent(
        &state.store,
        "sess-group-1",
        1,
        AgentName::Codex,
        &serde_json::json!({
            "method": "item/started",
            "params": {
                "item": { "type": "agentMessage", "id": "agent-msg-1" },
                "threadId": "sess-group-1",
                "turnId": "turn-1"
            }
        }),
        200,
    )
    .await
    .unwrap();
    raw_events::insert_if_absent(
        &state.store,
        "sess-group-1",
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
        "sess-group-1",
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
        Some("sess-group-1")
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn direct_agent_conversation_auto_routes_and_reuses_reply_session() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com", "phc")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let mut host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

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

    let registry = Arc::clone(&state.registry);
    let host_task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for session_id in ["sess-direct-1", "sess-direct-1"] {
            let frame = tokio::time::timeout(Duration::from_secs(1), host_rx.recv())
                .await
                .expect("agent dispatch should arrive before timeout")
                .expect("host should receive forwarded rpc");
            let Envelope::Forwarded { from, payload, .. } = frame else {
                panic!("expected forwarded rpc envelope");
            };
            assert_eq!(payload["method"], "minos_agent_dispatch");
            let request: AgentDispatchRequest =
                serde_json::from_value(payload["params"].clone()).expect("dispatch params decode");
            registry
                .route(
                    host_device_id,
                    from,
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": payload["id"].clone(),
                        "result": { "session_id": session_id }
                    }),
                )
                .await
                .expect("host rpc response routes back to requester");
            requests.push(request);
        }
        requests
    });

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
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &first_user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some("sess-direct-1")
    );

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
    social::bind_session_to_message(
        &state.store,
        &prior_agent_message.message_id,
        "sess-direct-1",
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
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &second_user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some("sess-direct-1")
    );

    let requests = host_task.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].session_id, None);
    assert_eq!(requests[0].text, "hello agent");
    assert_eq!(
        requests[0].origin_message_id.as_deref(),
        Some(first_user_message_id.as_str())
    );
    assert_eq!(requests[1].session_id.as_deref(), Some("sess-direct-1"));
    assert_eq!(requests[1].text, "follow up");
    assert_eq!(
        requests[1].origin_message_id.as_deref(),
        Some(second_user_message_id.as_str())
    );
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
    let alice = minos_backend::store::accounts::create(&state.store, "alice@example.com", "phc")
        .await
        .unwrap();
    let bob = minos_backend::store::accounts::create(&state.store, "bob@example.com", "phc")
        .await
        .unwrap();
    let alice_device = DeviceId::new();
    let host_device_id = seed_host_pair_for_account(&state, &alice.account_id, alice_device).await;
    let host_rx = seed_live_host_session(&state, host_device_id, &alice.account_id);

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
    let dispatch = spawn_agent_dispatch_responder(
        Arc::clone(&state.registry),
        host_device_id,
        host_rx,
        "sess-group-reply-1",
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
                    "text": format!("@{} summarize the PR", agent.agent_id)
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_message_id = body["message_id"].as_str().unwrap().to_string();

    // Wait for dispatch to complete
    let initial_dispatch = dispatch.await.unwrap();
    assert_eq!(initial_dispatch.agent, AgentName::Codex);
    assert_eq!(initial_dispatch.session_id, None);
    assert_eq!(initial_dispatch.text, "summarize the PR");

    // Verify session binding on user message
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &user_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some("sess-group-reply-1")
    );

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
    social::bind_session_to_message(&state.store, &agent_reply.message_id, "sess-group-reply-1")
        .await
        .unwrap();

    // Step 3: User replies to the agent's reply message — should reuse session
    // Need a fresh host session receiver for the second dispatch
    let host_rx2 = seed_live_host_session(&state, host_device_id, &alice.account_id);
    let dispatch2 = spawn_agent_dispatch_responder(
        Arc::clone(&state.registry),
        host_device_id,
        host_rx2,
        "sess-group-reply-1",
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

    // Verify the follow-up dispatch reuses the existing session
    let reuse_dispatch = dispatch2.await.unwrap();
    assert_eq!(
        reuse_dispatch.session_id.as_deref(),
        Some("sess-group-reply-1")
    );
    assert_eq!(
        reuse_dispatch.text,
        "can you also include the test coverage?"
    );
    assert_eq!(
        reuse_dispatch.origin_message_id.as_deref(),
        Some(followup_message_id.as_str())
    );

    // Verify session binding on the follow-up message
    assert_eq!(
        social::lookup_session_id_for_message(&state.store, &followup_message_id)
            .await
            .unwrap()
            .as_deref(),
        Some("sess-group-reply-1")
    );
}
