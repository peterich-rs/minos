use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::{router, test_support::backend_state, test_support::TEST_JWT_SECRET};
use minos_domain::DeviceId;

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

#[tokio::test]
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
            Method::GET,
            "/v1/me/profile",
            alice_device,
            &alice.account_id,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "alice@example.com");
    assert_eq!(body["minos_id"], alice.minos_id);

    let search_uri = format!("/v1/users/search?minos_id={}", &bob.minos_id[..4]);
    let (status, body) = common::send(
        &mut app,
        authed_request(
            Method::GET,
            &search_uri,
            alice_device,
            &alice.account_id,
            Body::empty(),
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
            Method::GET,
            "/v1/friend-requests",
            bob_device,
            &bob.account_id,
            Body::empty(),
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
            Method::GET,
            "/v1/friends",
            alice_device,
            &alice.account_id,
            Body::empty(),
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
            Method::GET,
            &format!("/v1/conversations/{conversation_id}/messages?limit=50"),
            bob_device,
            &bob.account_id,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["text"], "hello bob");
    assert_eq!(body["messages"][0]["sender"]["minos_id"], alice.minos_id);
}
