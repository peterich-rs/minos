//! Integration tests for `/v1/auth/{register,login,refresh,logout}`.
//!
//! Each test runs against a fresh in-memory SQLite via the
//! `test_support::backend_state` helper. The helper seeds a deterministic
//! `MINOS_JWT_SECRET` so token-binding assertions are stable.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http;
use minos_backend::http::test_support::{backend_state, TEST_JWT_SECRET};
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
async fn auth_realtime_ws_ticket_returns_short_lived_browser_upgrade_token() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &browser_headers(&device_id),
        json!({"email": "browser@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let access = body["access_token"].as_str().unwrap().to_string();
    let account_id = body["account"]["account_id"].as_str().unwrap().to_string();

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
    let state = backend_state().await;
    let mut app = http::router(state);
    let browser_device_id = uuid::Uuid::new_v4().to_string();
    let second_device_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &browser_headers(&browser_device_id),
        json!({"email": "browser-a@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&second_device_id),
        json!({"email": "browser-b@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let account_b = body["account"]["account_id"].as_str().unwrap().to_string();

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
            ("x-device-role", "mobile-client"),
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
    assert_eq!(body["error"]["code"], "weak_password");
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
    assert_eq!(body["error"]["code"], "email_taken");
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
    assert_eq!(body["error"]["code"], "invalid_credentials");
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
    assert_eq!(body["error"]["code"], "invalid_credentials");
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
    assert_eq!(body["error"]["code"], "invalid_refresh");
}

#[tokio::test]
async fn auth_login_keeps_other_iphone_ws_sessions_for_same_account() {
    // Multi-device mode: when device B logs into the same account that
    // device A is already logged into, A's live iOS WS session and refresh
    // token remain valid.
    use minos_backend::session::SessionHandle;
    use minos_domain::{DeviceId, DeviceRole};

    let state = backend_state().await;
    let device_a_uuid = uuid::Uuid::new_v4();
    let device_b_uuid = uuid::Uuid::new_v4();
    let device_a_str = device_a_uuid.to_string();
    let device_b_str = device_b_uuid.to_string();
    let device_a_id = DeviceId(device_a_uuid);

    let mut app = http::router(state.clone());

    // Device A registers + acquires a session. Register also seeds
    // accounts row + sets device_a.account_id.
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_a_str),
        json!({"email": "logout-displace@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let account_id = body["account"]["account_id"].as_str().unwrap().to_string();
    let device_a_refresh = body["refresh_token"].as_str().unwrap().to_string();

    // Simulate device A's live WS by directly inserting a SessionHandle
    // bound to the account. (The HTTP-only test app doesn't go through
    // /ws/client, so we model the post-upgrade state manually.)
    let (handle_a, mut rx_a) = SessionHandle::new(device_a_id, DeviceRole::MobileClient);
    handle_a.set_account_id(account_id.clone());
    state.registry.insert(handle_a.clone());
    let a_revoked = handle_a.subscribe_revocation();

    // Device B logs in with the same credentials. This should rotate only
    // device B's own tokens and leave device A connected.
    let (status, _body) = post_json(
        &mut app,
        "/v1/auth/login",
        &ios_headers(&device_b_str),
        json!({"email": "logout-displace@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        state.registry.get(device_a_id).is_some(),
        "device A's session must remain live after device B logs in",
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

    drop(handle_a);
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
            ("x-device-role", "mobile-client"),
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
    assert_eq!(body["error"]["code"], "invalid_refresh");
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
    assert_eq!(body["error"]["code"], "invalid_refresh");
}

#[tokio::test]
async fn auth_refresh_reuse_revokes_all_account_tokens_and_records_metric() {
    let state = backend_state().await;
    let mut app = http::router(state);
    let device_a = uuid::Uuid::new_v4().to_string();
    let device_b = uuid::Uuid::new_v4().to_string();

    let (_, register_body) = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_a),
        json!({"email": "reuse@example.com", "password": "testpass1"}),
    )
    .await;
    let original_refresh = register_body["refresh_token"].as_str().unwrap().to_string();

    let (_, login_body) = post_json(
        &mut app,
        "/v1/auth/login",
        &ios_headers(&device_b),
        json!({"email": "reuse@example.com", "password": "testpass1"}),
    )
    .await;
    let device_b_refresh = login_body["refresh_token"].as_str().unwrap().to_string();

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
            ("x-device-role", "mobile-client"),
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
async fn auth_rate_limit_login_returns_429_with_retry_after() {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    let state = backend_state().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();

    // Pre-register so the credentials check itself doesn't 401 fast.
    let _ = post_json(
        &mut app,
        "/v1/auth/register",
        &ios_headers(&device_id),
        json!({"email": "rl@example.com", "password": "testpass1"}),
    )
    .await;

    // Fire login 5 times: all should be allowed by the bucket. The 6th
    // hits the per-IP limit and must return 429 with Retry-After.
    let ip = "203.0.113.7";
    let mut last_status: Option<StatusCode> = None;
    let mut last_retry_after: Option<String> = None;
    for _ in 0..6 {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/login")
            .header("content-type", "application/json")
            .header("x-device-id", &device_id)
            .header("x-device-role", "mobile-client")
            .header("x-forwarded-for", ip)
            .body(json_body(json!({
                "email": "rl@example.com",
                "password": "testpass1"
            })))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        last_status = Some(resp.status());
        last_retry_after = resp
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string);
        // Drain body so the connection is reusable.
        let _ = resp.into_body().collect().await.unwrap().to_bytes();
    }

    assert_eq!(last_status, Some(StatusCode::TOO_MANY_REQUESTS));
    let retry: u32 = last_retry_after
        .expect("Retry-After header must be set on 429")
        .parse()
        .unwrap();
    assert!(retry >= 1, "retry-after must be >= 1");
}

// ── Supabase exchange ────────────────────────────────────────────────

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use minos_backend::http::test_support::{
    backend_state_with_supabase, TEST_SUPABASE_AUD, TEST_SUPABASE_HMAC, TEST_SUPABASE_ISS,
};

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
async fn auth_supabase_exchange_merges_verified_email_with_password_account() {
    let state = backend_state_with_supabase().await;
    let mut app = http::router(state);
    let device_id = uuid::Uuid::new_v4().to_string();

    let (status, reg) = post_json(
        &mut app,
        "/v1/auth/register",
        &browser_headers(&device_id),
        json!({"email": "merge-me@example.com", "password": "testpass1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={reg}");
    let password_account_id = reg["account"]["account_id"].as_str().unwrap().to_string();

    let other_device = uuid::Uuid::new_v4().to_string();
    let token = mint_supabase_token("sub-merge-1", Some("merge-me@example.com"), true, 3600);
    let (status, body) = post_json(
        &mut app,
        "/v1/auth/supabase",
        &browser_headers(&other_device),
        json!({ "access_token": token }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["account"]["account_id"], password_account_id);
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
