//! Integration tests for `/v1/auth/{register,login,refresh,logout}`.
//!
//! Each test runs against a fresh in-memory SQLite via the
//! `test_support::backend_state` helper. The helper seeds a deterministic
//! `MINOS_JWT_SECRET` so token-binding assertions are stable.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use minos_backend::http;
use minos_backend::http::test_support::backend_state;
use serde_json::json;

mod common;

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
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
        .header("content-type", "application/json");
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let req = builder.body(json_body(body)).unwrap();
    common::send(app, req).await
}

fn ios_headers(device_id: &str) -> Vec<(&str, &str)> {
    vec![
        ("x-device-id", device_id),
        ("x-device-role", "ios-client"),
    ]
}

#[tokio::test]
async fn auth_register_returns_access_and_refresh_tokens() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "alice@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(!body["access_token"].as_str().unwrap().is_empty());
    assert!(!body["refresh_token"].as_str().unwrap().is_empty());
    assert_eq!(body["account"]["email"], "alice@example.com");
    assert!(body["expires_in"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn auth_register_login_refresh_logout_happy_path() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "happy@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let refresh = body["refresh_token"].as_str().unwrap().to_string();
    assert!(!refresh.is_empty());

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/login",
        &ios_headers(&device_id),
        json!({"email": "happy@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let new_refresh = body["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(new_refresh, refresh, "login mints a fresh refresh token");

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": new_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let new_access = body["access_token"].as_str().unwrap().to_string();
    let final_refresh = body["refresh_token"].as_str().unwrap().to_string();

    let auth_hdr = format!("Bearer {new_access}");
    let (status, _body) = post_json(
        &mut app,
        "/v1/auth/logout",
        &[
            ("x-device-id", &device_id),
            ("x-device-role", "ios-client"),
            ("authorization", &auth_hdr),
        ],
        json!({"refresh_token": final_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn auth_register_weak_password_returns_400() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "bob@example.com", "password": "short"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["kind"], "weak_password");
}

#[tokio::test]
async fn auth_register_duplicate_email_returns_409() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let _ = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "dup@example.com", "password": "testpass1"}),
    )
    .await;
    let device_id_b = uuid::Uuid::new_v4().to_string();
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id_b),
        json!({"email": "DUP@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["kind"], "email_taken");
}

#[tokio::test]
async fn auth_login_wrong_password_returns_401() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let _ = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "wrong@example.com", "password": "testpass1"}),
    )
    .await;
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/login",
        &ios_headers(&device_id),
        json!({"email": "wrong@example.com", "password": "different"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "invalid_credentials");
}

#[tokio::test]
async fn auth_login_unknown_email_returns_401() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/login",
        &ios_headers(&device_id),
        json!({"email": "ghost@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "invalid_credentials");
}

#[tokio::test]
async fn auth_login_revokes_existing_refresh_tokens() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (_, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "rev@example.com", "password": "testpass1"}),
    )
    .await;
    let first_refresh = body["refresh_token"].as_str().unwrap().to_string();

    let _ = post_json(
        &mut app,
        "/v1/auth/login",
        &ios_headers(&device_id),
        json!({"email": "rev@example.com", "password": "testpass1"}),
    )
    .await;

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": first_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "invalid_refresh");
}

#[tokio::test]
async fn auth_refresh_with_revoked_token_returns_401() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (_, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "rev2@example.com", "password": "testpass1"}),
    )
    .await;
    let access = body["access_token"].as_str().unwrap().to_string();
    let refresh = body["refresh_token"].as_str().unwrap().to_string();

    // Logout revokes the refresh token.
    let auth_hdr = format!("Bearer {access}");
    let _ = post_json(
        &mut app,
        "/v1/auth/logout",
        &[
            ("x-device-id", &device_id),
            ("x-device-role", "ios-client"),
            ("authorization", &auth_hdr),
        ],
        json!({"refresh_token": refresh}),
    )
    .await;

    // Subsequent refresh must fail.
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "invalid_refresh");
}

#[tokio::test]
async fn auth_refresh_rotation_old_token_invalidated() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (_, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "rot@example.com", "password": "testpass1"}),
    )
    .await;
    let original_refresh = body["refresh_token"].as_str().unwrap().to_string();

    // Rotate.
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": original_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let _new_refresh = body["refresh_token"].as_str().unwrap().to_string();

    // Reusing the original token must fail.
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": original_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "invalid_refresh");
}

#[tokio::test]
async fn auth_logout_revokes_only_current_refresh_token() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (_, reg) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "logout@example.com", "password": "testpass1"}),
    )
    .await;
    let r1 = reg["refresh_token"].as_str().unwrap().to_string();
    let access = reg["access_token"].as_str().unwrap().to_string();

    // Rotate to get a second active refresh token.
    let (_, rot) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": r1}),
    )
    .await;
    let r2 = rot["refresh_token"].as_str().unwrap().to_string();

    // Refresh r1 again to confirm rotation already revoked it.
    let (status, _) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": r1}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Logout revokes the current (r2) — but does not touch other accounts.
    let auth_hdr = format!("Bearer {access}");
    let (status, _body) = post_json(
        &mut app,
        "/v1/auth/logout",
        &[
            ("x-device-id", &device_id),
            ("x-device-role", "ios-client"),
            ("authorization", &auth_hdr),
        ],
        json!({"refresh_token": r2}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // r2 must now also be invalid.
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": r2}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "invalid_refresh");
}

#[tokio::test]
async fn auth_logout_without_bearer_returns_401() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/logout",
        &ios_headers(&device_id),
        json!({"refresh_token": "any"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "unauthorized");
}
