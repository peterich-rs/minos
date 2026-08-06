use axum::body::Body;
use axum::http::{Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http;
use minos_backend::http::test_support::{backend_state, TEST_JWT_SECRET};
use minos_backend::store::test_support::{insert_test_client, insert_test_host};
use minos_backend::store::{device_installations, host_links};
use minos_domain::{DeviceId, DeviceRole};
use serde_json::json;

mod common;

fn json_body(value: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&value).unwrap())
}

async fn post_json(
    app: &mut axum::Router,
    path: &str,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("x-request-id", "req_contract_test");
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    common::send(app, builder.body(json_body(body)).unwrap()).await
}

#[tokio::test]
async fn formal_realtime_ws_ticket_uses_account_bearer_without_device_headers() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let installation_id = uuid::Uuid::new_v4().to_string();
    let device_id = uuid::Uuid::parse_str(&installation_id)
        .map(DeviceId)
        .unwrap();

    let account = minos_backend::store::accounts::create(&state.store, "formal-ticket@example.com")
        .await
        .unwrap();
    let account_id = account.account_id.clone();
    device_installations::insert_client_for_account(
        &state.store,
        device_id,
        "browser",
        DeviceRole::BrowserAdmin,
        &account_id,
        0,
    )
    .await
    .unwrap();
    device_installations::touch_last_seen(&state.store, &device_id, 100)
        .await
        .unwrap();
    let access = jwt::sign(TEST_JWT_SECRET.as_bytes(), &account_id, &installation_id).unwrap();

    let auth_header = format!("Bearer {access}");
    let (status, body) = post_json(
        &mut app,
        "/v1/realtime/ws-ticket",
        &[("authorization", &auth_header)],
        json!({"installation_id": installation_id.clone()}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["meta"]["request_id"], "req_contract_test");
    assert!(body["data"]["gateway_url"]
        .as_str()
        .unwrap()
        .starts_with("/ws/client?ticket="));
    assert!(body["data"]["expires_at_ms"].as_i64().unwrap() > 0);
    let ticket = body["data"]["ticket"].as_str().unwrap();
    let claims = jwt::verify_ws_ticket(TEST_JWT_SECRET.as_bytes(), ticket).unwrap();
    assert_eq!(claims.sub, account_id);
    assert_eq!(claims.did, installation_id);
    let row = device_installations::get_device(&state.store, device_id)
        .await
        .unwrap()
        .unwrap();
    assert!(row.last_seen_at > 100);
}

#[tokio::test]
async fn formal_hosts_list_uses_account_bearer_without_device_headers() {
    let state = backend_state().await;
    let account = minos_backend::store::accounts::create(&state.store, "formal-hosts@example.com")
        .await
        .unwrap();
    let host = DeviceId::new();
    let mobile = DeviceId::new();

    insert_test_host(&state.store, host, "Mac Studio", 100,).await;
    { let _acct = minos_backend::store::accounts::create(&state.store, &format!("fixture-{}@localhost", mobile)).await.unwrap(); insert_test_client(&state.store, mobile, DeviceRole::MobileClient, &_acct.account_id, "iPhone", 100,).await; };
    device_installations::set_account_id(&state.store, &mobile, &account.account_id)
        .await
        .unwrap();
    host_links::insert_pair(&state.store, host, &account.account_id, mobile, 123)
        .await
        .unwrap();

    let token = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        &account.account_id,
        &mobile.to_string(),
    )
    .unwrap();
    let auth_header = format!("Bearer {token}");
    let mut app = http::router(state);

    let (status, body) = common::send(
        &mut app,
        Request::builder()
            .method("GET")
            .uri("/v1/hosts")
            .header("authorization", &auth_header)
            .header("x-request-id", "req_contract_test")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["meta"]["request_id"], "req_contract_test");
    let hosts = body["data"]["hosts"].as_array().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0]["host_installation_id"], host.to_string());
    assert_eq!(hosts[0]["host_display_name"], "Mac Studio");
    assert_eq!(hosts[0]["linked_at_ms"], 123);
    assert_eq!(hosts[0]["online"], false);
}
