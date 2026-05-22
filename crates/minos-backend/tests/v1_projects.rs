use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::test_support::TEST_JWT_SECRET;
use minos_backend::http::{router, test_support::backend_state};
use minos_domain::{AgentName, DeviceId, DeviceRole};

mod common;

fn authed_post(
    uri: &str,
    device_id: DeviceId,
    account_id: &str,
    body: serde_json::Value,
) -> Request<Body> {
    let token = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        account_id,
        &device_id.to_string(),
    )
    .unwrap();
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("x-device-id", device_id.to_string())
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn formal_project_routes_expose_canonical_conversation_and_agent_session_flow() {
    let state = backend_state().await;
    let mut app = router(state.clone());

    let account =
        minos_backend::store::accounts::create(&state.store, "projects-formal@example.com", "phc")
            .await
            .unwrap();
    let device_id = DeviceId::new();

    let conversation = minos_backend::store::social::create_group_conversation(
        &state.store,
        &account.account_id,
        "Project Conversation",
        &[],
        1_000,
    )
    .await
    .unwrap();
    minos_backend::store::agent_sessions::create(
        &state.store,
        "sess-project-link-1",
        &conversation.conversation_id,
        None,
        None,
        Some("agent_codex"),
        "running",
        1_001,
        None,
    )
    .await
    .unwrap();
    let host_device_id = DeviceId::new();
    minos_backend::store::devices::insert_device(
        &state.store,
        host_device_id,
        "Mac",
        DeviceRole::AgentHost,
        999,
    )
    .await
    .unwrap();
    minos_backend::store::devices::set_account_id(
        &state.store,
        &host_device_id,
        &account.account_id,
    )
    .await
    .unwrap();
    minos_backend::store::threads::upsert(
        &state.store,
        "sess-project-link-1",
        AgentName::Codex,
        &host_device_id.to_string(),
        1_005,
    )
    .await
    .unwrap();
    minos_backend::store::threads::update_title(
        &state.store,
        "sess-project-link-1",
        "Project Session",
    )
    .await
    .unwrap();
    minos_backend::store::threads::increment_message_count(&state.store, "sess-project-link-1")
        .await
        .unwrap();

    let (status, body) = common::send(
        &mut app,
        authed_post(
            "/v1/projects/create",
            device_id,
            &account.account_id,
            serde_json::json!({
                "name": "Workspace",
                "workspace_slug": "workspace",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let project_id = body["project"]["project_id"].as_str().unwrap().to_string();

    let (status, _) = common::send(
        &mut app,
        authed_post(
            "/v1/projects/rename",
            device_id,
            &account.account_id,
            serde_json::json!({
                "project_id": project_id,
                "name": "Renamed Workspace",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = common::send(
        &mut app,
        authed_post(
            "/v1/projects/conversations/link",
            device_id,
            &account.account_id,
            serde_json::json!({
                "project_id": project_id,
                "conversation_id": conversation.conversation_id,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = common::send(
        &mut app,
        authed_post(
            "/v1/projects/list",
            device_id,
            &account.account_id,
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["projects"].as_array().unwrap().len(), 1);
    assert_eq!(body["projects"][0]["name"], "Renamed Workspace");
    assert_eq!(body["projects"][0]["thread_count"], 1);

    let (status, body) = common::send(
        &mut app,
        authed_post(
            "/v1/projects/agent-sessions/query",
            device_id,
            &account.account_id,
            serde_json::json!({
                "project_id": project_id,
                "limit": 10,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(body["sessions"][0]["session_id"], "sess-project-link-1");
    assert_eq!(
        body["sessions"][0]["conversation_id"],
        conversation.conversation_id
    );
    assert_eq!(body["sessions"][0]["project_id"], project_id);
    assert_eq!(body["sessions"][0]["title"], "Project Session");
    assert_eq!(body["sessions"][0]["message_count"], 1);
    assert_eq!(body["sessions"][0]["last_activity_at_ms"], 1005);

    let pool = state
        .store
        .sqlite_pool()
        .expect("project compatibility checks run only against sqlite test state");

    let compat_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM project_threads WHERE project_id = ?")
            .bind(&project_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        compat_rows, 0,
        "project_threads compatibility storage is retired"
    );

    let linked_project_id: Option<String> =
        sqlx::query_scalar("SELECT project_id FROM agent_sessions WHERE session_id = ?")
            .bind("sess-project-link-1")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(linked_project_id.as_deref(), Some(project_id.as_str()));

    let (status, _) = common::send(
        &mut app,
        authed_post(
            "/v1/projects/threads/query",
            device_id,
            &account.account_id,
            serde_json::json!({
                "project_id": project_id,
                "limit": 10,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
