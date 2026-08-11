//! Integration tests for `/v1/auth/{supabase,refresh,logout}`.
//!
//! Password register/login endpoints are retired (404). Account creation
//! goes through Supabase exchange with a synthetic HS256 JWT against
//! `backend_state_with_supabase`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use minos_backend::auth::jwt;
use minos_backend::http;
use minos_backend::http::test_support::{
    backend_state, backend_state_with_supabase, TEST_JWT_SECRET, TEST_SUPABASE_AUD,
    TEST_SUPABASE_HMAC, TEST_SUPABASE_ISS,
};
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
        ("x-device-role", "mobile-client"),
    ]
}

fn browser_headers(device_id: &str) -> Vec<(&str, &str)> {
    vec![
        ("x-device-id", device_id),
        ("x-device-role", "browser-admin"),
    ]
}

fn mint_supabase_token(
    sub: &str,
    email: Option<&str>,
    email_verified: bool,
    exp_offset_secs: i64,
) -> String {
    let now = chrono::Utc::now().timestamp();
    let mut claims = serde_json::json!({
        "sub": sub,
        "iss": TEST_SUPABASE_ISS,
        "aud": TEST_SUPABASE_AUD,
        "exp": now + exp_offset_secs,
        "iat": now,
        "email_verified": email_verified,
    });
    if let Some(email) = email {
        claims["email"] = serde_json::json!(email);
    }
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(TEST_SUPABASE_HMAC),
    )
    .unwrap()
}

/// Create a Minos session via Supabase exchange; returns (access, refresh, account_id, email).
async fn exchange_session(
    app: &mut axum::Router,
    headers: &[(&str, &str)],
    sub: &str,
    email: &str,
) -> (String, String, String, String) {
    let token = mint_supabase_token(sub, Some(email), true, 3600);
    let (status, body) = post_json(
        app,
        "/v1/auth/supabase",
        headers,
        json!({ "access_token": token }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    (
        body["access_token"].as_str().unwrap().to_string(),
        body["refresh_token"].as_str().unwrap().to_string(),
        body["account"]["account_id"].as_str().unwrap().to_string(),
        body["account"]["email"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn retired_password_register_and_login_return_404() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();

    for path in [
        "/v1/auth/register",
        "/v1/auth/login",
        "/v1/auth/change-password",
    ] {
        let (status, body) = post_json(
            &mut app,
            path,
            &ios_headers(&device_id),
            json!({"email": "a@example.com", "password": "testpass1"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "path={path} body={body}");
    }
}

#[tokio::test]
async fn auth_supabase_exchange_returns_access_and_refresh_tokens() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (access, refresh, _account_id, email) = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-ios-1",
        "alice@example.com",
    )
    .await;
    assert!(!access.is_empty());
    assert!(!refresh.is_empty());
    assert_eq!(email, "alice@example.com");
}

#[tokio::test]
async fn auth_realtime_ws_ticket_returns_short_lived_browser_upgrade_token() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();

    let (access, _refresh, account_id, _) = exchange_session(
        &mut app,
        &browser_headers(&device_id),
        "sub-browser-1",
        "browser@example.com",
    )
    .await;

    let auth_hdr = format!("Bearer {access}");
    let (status, body) = post_json(
        &mut app,
        "/v1/realtime/ws-ticket",
        &[("authorization", &auth_hdr)],
        json!({"installation_id": device_id.clone()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body["data"]["expires_at_ms"].as_i64().unwrap() > 0);
    let ticket = body["data"]["ticket"].as_str().unwrap();
    assert_eq!(
        body["data"]["gateway_url"],
        format!("/ws/client?ticket={ticket}")
    );
    let claims = jwt::verify_ws_ticket(TEST_JWT_SECRET.as_bytes(), ticket).unwrap();
    assert_eq!(claims.sub, account_id);
    assert_eq!(claims.did, device_id);
    assert_eq!(claims.role.to_string(), "browser-admin");
}

#[tokio::test]
async fn auth_realtime_ws_ticket_rejects_cross_account_device_rebind() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let browser_device_id = uuid::Uuid::new_v4().to_string();
    let second_device_id = uuid::Uuid::new_v4().to_string();

    let _ = exchange_session(
        &mut app,
        &browser_headers(&browser_device_id),
        "sub-browser-a",
        "browser-a@example.com",
    )
    .await;

    let (_access, _refresh, account_b, _) = exchange_session(
        &mut app,
        &ios_headers(&second_device_id),
        "sub-browser-b",
        "browser-b@example.com",
    )
    .await;

    let cross_account_token =
        jwt::sign(TEST_JWT_SECRET.as_bytes(), &account_b, &browser_device_id).unwrap();
    let auth_hdr = format!("Bearer {cross_account_token}");
    let (status, body) = post_json(
        &mut app,
        "/v1/realtime/ws-ticket",
        &[("authorization", &auth_hdr)],
        json!({"installation_id": browser_device_id}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn auth_exchange_refresh_logout_happy_path() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();

    let (_access, refresh, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-happy",
        "happy@example.com",
    )
    .await;

    // Re-exchange (login again) mints a fresh refresh for this device.
    let (_access2, new_refresh, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-happy",
        "happy@example.com",
    )
    .await;
    assert_ne!(
        new_refresh, refresh,
        "re-exchange mints a fresh refresh token"
    );

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
            ("x-device-role", "mobile-client"),
            ("authorization", &auth_hdr),
        ],
        json!({"refresh_token": final_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn auth_exchange_merges_verified_email_with_unbound_account() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state.clone());
    let device_id = uuid::Uuid::new_v4().to_string();

    let unbound = minos_backend::store::accounts::create(&state.store, "merge-me@example.com")
        .await
        .unwrap();

    let token = mint_supabase_token("sub-merge-1", Some("merge-me@example.com"), true, 3600);
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/supabase",
        &browser_headers(&device_id),
        json!({ "access_token": token }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["account"]["account_id"], unbound.account_id);
}

#[tokio::test]
async fn auth_exchange_reuses_existing_sub() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_a = uuid::Uuid::new_v4().to_string();
    let device_b = uuid::Uuid::new_v4().to_string();

    let (_, _, account_a, _) = exchange_session(
        &mut app,
        &ios_headers(&device_a),
        "sub-reuse",
        "reuse@example.com",
    )
    .await;
    let (_, _, account_b, _) = exchange_session(
        &mut app,
        &ios_headers(&device_b),
        "sub-reuse",
        "reuse@example.com",
    )
    .await;
    assert_eq!(account_a, account_b);
}

#[tokio::test]
async fn auth_exchange_revokes_existing_refresh_tokens_for_device() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (_, first_refresh, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-rev",
        "rev@example.com",
    )
    .await;

    let _ = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-rev",
        "rev@example.com",
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
    assert_eq!(body["error"]["code"], "invalid_refresh");
}

#[tokio::test]
async fn auth_exchange_keeps_other_iphone_ws_sessions_for_same_account() {
    use minos_backend::http::test_support::seed_live_connection;
    use minos_domain::{DeviceId, DeviceRole};

    let state = backend_state_with_supabase().await;
    let device_a_uuid = uuid::Uuid::new_v4();
    let device_b_uuid = uuid::Uuid::new_v4();
    let device_a_str = device_a_uuid.to_string();
    let device_b_str = device_b_uuid.to_string();
    let device_a_id = DeviceId(device_a_uuid);

    let mut app = http::router(state.clone());

    let (_, device_a_refresh, account_id, _) = exchange_session(
        &mut app,
        &ios_headers(&device_a_str),
        "sub-multi",
        "logout-displace@example.com",
    )
    .await;

    let (conn_a, mut rx_a) =
        seed_live_connection(&state, device_a_id, DeviceRole::MobileClient, Some(&account_id));
    let a_revoked = conn_a.subscribe_revocation();

    let (status, _body) = post_json(
        &mut app,
        "/v1/auth/supabase",
        &ios_headers(&device_b_str),
        json!({
            "access_token": mint_supabase_token(
                "sub-multi",
                Some("logout-displace@example.com"),
                true,
                3600,
            )
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        state.registry.get(device_a_id).is_some(),
        "device A's session must remain live after device B exchanges",
    );
    assert_eq!(*a_revoked.borrow(), None, "device A must not be revoked");
    assert!(
        rx_a.try_recv().is_err(),
        "device A should not receive a forced-close frame"
    );

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_a_str),
        json!({"refresh_token": device_a_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    drop(conn_a);
}

#[tokio::test]
async fn auth_refresh_with_revoked_token_returns_401() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (access, refresh, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-rev2",
        "rev2@example.com",
    )
    .await;

    let auth_hdr = format!("Bearer {access}");
    let _ = post_json(
        &mut app,
        "/v1/auth/logout",
        &[
            ("x-device-id", &device_id),
            ("x-device-role", "mobile-client"),
            ("authorization", &auth_hdr),
        ],
        json!({"refresh_token": refresh}),
    )
    .await;

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_refresh");
}

#[tokio::test]
async fn auth_refresh_rotation_old_token_invalidated() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (_, original_refresh, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-rot",
        "rot@example.com",
    )
    .await;

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": original_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": original_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_refresh");
}

#[tokio::test]
async fn auth_refresh_reuse_revokes_all_account_tokens_and_records_metric() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_a = uuid::Uuid::new_v4().to_string();
    let device_b = uuid::Uuid::new_v4().to_string();

    let (_, original_refresh, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_a),
        "sub-reuse-refresh",
        "reuse@example.com",
    )
    .await;

    let (_, device_b_refresh, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_b),
        "sub-reuse-refresh",
        "reuse@example.com",
    )
    .await;

    let (status, refresh_body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_a),
        json!({"refresh_token": original_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={refresh_body}");

    let (status, reuse_body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_a),
        json!({"refresh_token": original_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={reuse_body}");
    assert_eq!(reuse_body["error"]["code"], "invalid_refresh");

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_b),
        json!({"refresh_token": device_b_refresh}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "invalid_refresh");

    let metrics = minos_backend::telemetry::render();
    assert!(metrics.contains("minos_backend_auth_refresh_reuse_total"));
}

#[tokio::test]
async fn auth_logout_revokes_only_current_refresh_token() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (access, r1, _, _) = exchange_session(
        &mut app,
        &ios_headers(&device_id),
        "sub-logout",
        "logout@example.com",
    )
    .await;

    let (_, rot) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": r1}),
    )
    .await;
    let r2 = rot["refresh_token"].as_str().unwrap().to_string();

    let (status, _) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": r1}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let auth_hdr = format!("Bearer {access}");
    let (status, _body) = post_json(
        &mut app,
        "/v1/auth/logout",
        &[
            ("x-device-id", &device_id),
            ("x-device-role", "mobile-client"),
            ("authorization", &auth_hdr),
        ],
        json!({"refresh_token": r2}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/refresh",
        &ios_headers(&device_id),
        json!({"refresh_token": r2}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_refresh");
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
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn auth_rate_limit_exchange_returns_429_with_retry_after() {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    let state = backend_state_with_supabase().await;
    let app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let token = mint_supabase_token("sub-rl", Some("rl@example.com"), true, 3600);

    // exchange_per_ip default is 3 / hour — 4th should 429.
    let ip = "203.0.113.7";
    let mut last_status: Option<StatusCode> = None;
    let mut last_retry_after: Option<String> = None;
    for i in 0..4 {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/supabase")
            .header("content-type", "application/json")
            .header("x-device-id", &device_id)
            .header("x-device-role", "mobile-client")
            .header("x-forwarded-for", ip)
            .body(json_body(json!({ "access_token": token })))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        last_status = Some(resp.status());
        last_retry_after = resp
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string);
        let _ = resp.into_body().collect().await.unwrap().to_bytes();
        // First three should succeed (same sub reuses account).
        if i < 3 {
            assert_eq!(last_status, Some(StatusCode::OK), "attempt {i}");
        }
    }

    assert_eq!(last_status, Some(StatusCode::TOO_MANY_REQUESTS));
    let retry: u32 = last_retry_after
        .expect("Retry-After header must be set on 429")
        .parse()
        .unwrap();
    assert!(retry >= 1, "retry-after must be >= 1");
}

#[tokio::test]
async fn auth_supabase_exchange_creates_account_and_returns_minos_session() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let token = mint_supabase_token("sub-new-1", Some("oidc@example.com"), true, 3600);

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/supabase",
        &browser_headers(&device_id),
        json!({ "access_token": token }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["account"]["email"], "oidc@example.com");
    assert!(!body["access_token"].as_str().unwrap().is_empty());
    assert!(!body["refresh_token"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn auth_supabase_exchange_rejects_expired_token() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let token = mint_supabase_token("sub-exp", Some("exp@example.com"), true, -120);

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/supabase",
        &browser_headers(&device_id),
        json!({ "access_token": token }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "supabase_token_expired");
}

#[tokio::test]
async fn auth_supabase_exchange_without_config_returns_503() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/supabase",
        &browser_headers(&device_id),
        json!({ "access_token": "not-a-jwt" }),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={body}");
    assert_eq!(body["error"]["code"], "supabase_not_configured");
}
