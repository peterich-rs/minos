//! Integration tests for same-account Host Link (`/v1/hosts/*`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use minos_backend::auth::jwt;
use minos_backend::http;
use minos_backend::http::test_support::seed_live_connection;
use minos_backend::http::test_support::{backend_state, TEST_JWT_SECRET};
use minos_backend::http::v1::hosts::LINK_PATH;
use minos_backend::store::{device_installations, host_installation_tokens, host_links};
use minos_domain::{DeviceId, DeviceRole};
use serde_json::json;

mod common;

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public_key(signing_key: &SigningKey) -> String {
    format!(
        "ed25519:{}",
        URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes())
    )
}

fn signature(signing_key: &SigningKey, installation_id: &str, nonce: &str, path: &str) -> String {
    let payload = format!("{installation_id}:{nonce}:{path}");
    format!(
        "ed25519-sig:{}",
        URL_SAFE_NO_PAD.encode(signing_key.sign(payload.as_bytes()).to_bytes())
    )
}

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
        .header("x-request-id", "req_host_link");
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    common::send(app, builder.body(json_body(body)).unwrap()).await
}

async fn get_json(
    app: &mut axum::Router,
    path: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("GET")
        .uri(path)
        .header("x-request-id", "req_host_link");
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    common::send(app, builder.body(Body::empty()).unwrap()).await
}

async fn register_desktop(state: &http::BackendState, email: &str) -> (String, String, String) {
    let installation_id = uuid::Uuid::new_v4().to_string();
    let device_id = uuid::Uuid::parse_str(&installation_id)
        .map(DeviceId)
        .unwrap();
    let account = minos_backend::store::accounts::create(&state.store, email)
        .await
        .unwrap();
    let account_id = account.account_id.clone();
    device_installations::insert_client_for_account(
        &state.store,
        device_id,
        "desktop",
        DeviceRole::DesktopConsole,
        &account_id,
        0,
    )
    .await
    .unwrap();
    let access = jwt::sign(TEST_JWT_SECRET.as_bytes(), &account_id, &installation_id).unwrap();
    (access, account_id, installation_id)
}

async fn issue_nonce(app: &mut axum::Router, installation_id: &str) -> String {
    let (status, body) = post_json(
        app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    body["data"]["nonce"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn host_link_unlink_list_round_trip() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let (access, account_id, desktop_id) =
        register_desktop(&state, "host-link-roundtrip@example.com").await;
    let auth = format!("Bearer {access}");

    let host = DeviceId::new();
    let host_id = host.to_string();
    let key = signing_key(21);
    let pubkey = public_key(&key);
    let nonce = issue_nonce(&mut app, &host_id).await;
    let sig = signature(&key, &host_id, &nonce, LINK_PATH);

    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &auth)],
        json!({
            "installation_id": host_id,
            "nonce": nonce,
            "public_key": pubkey,
            "signature": sig,
            "host_display_name": "Studio Mac"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["host_installation_id"], host_id);
    let token = body["data"]["host_installation_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(token.starts_with("hit_"));
    assert_eq!(body["data"]["link"]["account_id"], account_id);
    assert_eq!(body["data"]["link"]["host_display_name"], "Studio Mac");
    assert!(body["data"]["link"]["linked_at_ms"].as_i64().unwrap() > 0);

    assert!(host_links::exists(&state.store, host, &account_id)
        .await
        .unwrap());
    let host_row = device_installations::get_device(&state.store, host)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(host_row.role, DeviceRole::AgentHost);
    assert_eq!(host_row.public_key.as_deref(), Some(pubkey.as_str()));
    // linked_via should be desktop installation
    let pairs = host_links::list_hosts_for_account(&state.store, &account_id)
        .await
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].paired_via_device_id.to_string(), desktop_id);

    // Seed online connection for list
    let (_conn, _rx) = seed_live_connection(&state, host, DeviceRole::AgentHost, None);

    let (status, body) = get_json(&mut app, "/v1/hosts", &[("authorization", &auth)]).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let hosts = body["data"]["hosts"].as_array().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0]["host_installation_id"], host_id);
    assert_eq!(hosts[0]["host_display_name"], "Studio Mac");
    assert_eq!(hosts[0]["online"], true);

    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/unlink",
        &[("authorization", &auth)],
        json!({"host_installation_id": host_id}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body={body}");
    assert!(!host_links::exists(&state.store, host, &account_id)
        .await
        .unwrap());
    assert!(state.registry.get(host).is_none());

    // Token must be revoked (verify_active_token returns None)
    let token_hash = minos_backend::host_link::sha256_hex(&token);
    let active = host_installation_tokens::verify_active_token(
        &state.store,
        &token_hash,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .unwrap();
    assert!(active.is_none());
}

#[tokio::test]
async fn multi_host_list_for_account() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let (access, account_id, _) = register_desktop(&state, "multi-host-list@example.com").await;
    let auth = format!("Bearer {access}");

    for (seed, name) in [(31_u8, "Mac A"), (32, "Mac B")] {
        let host = DeviceId::new();
        let host_id = host.to_string();
        let key = signing_key(seed);
        let nonce = issue_nonce(&mut app, &host_id).await;
        let (status, body) = post_json(
            &mut app,
            "/v1/hosts/link",
            &[("authorization", &auth)],
            json!({
                "installation_id": host_id,
                "nonce": nonce,
                "public_key": public_key(&key),
                "signature": signature(&key, &host_id, &nonce, LINK_PATH),
                "host_display_name": name,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={body}");
    }

    let pairs = host_links::list_hosts_for_account(&state.store, &account_id)
        .await
        .unwrap();
    assert_eq!(pairs.len(), 2);

    let (status, body) = get_json(&mut app, "/v1/hosts", &[("authorization", &auth)]).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["hosts"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn bad_proof_is_rejected() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let (access, _, _) = register_desktop(&state, "bad-proof@example.com").await;
    let auth = format!("Bearer {access}");

    let host_id = DeviceId::new().to_string();
    let key = signing_key(41);
    let nonce = issue_nonce(&mut app, &host_id).await;
    // Sign with wrong path → proof_invalid
    let bad_sig = signature(&key, &host_id, &nonce, "/v1/host/bootstrap/nonce");

    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &auth)],
        json!({
            "installation_id": host_id,
            "nonce": nonce,
            "public_key": public_key(&key),
            "signature": bad_sig,
            "host_display_name": "Bad"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "proof_invalid");
}

#[tokio::test]
async fn host_already_linked_elsewhere_returns_409() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let (access_a, _, _) = register_desktop(&state, "elsewhere-a@example.com").await;
    let (access_b, _, _) = register_desktop(&state, "elsewhere-b@example.com").await;
    let auth_a = format!("Bearer {access_a}");
    let auth_b = format!("Bearer {access_b}");

    let host = DeviceId::new();
    let host_id = host.to_string();
    let key = signing_key(51);
    let pubkey = public_key(&key);

    let nonce = issue_nonce(&mut app, &host_id).await;
    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &auth_a)],
        json!({
            "installation_id": host_id,
            "nonce": nonce,
            "public_key": pubkey,
            "signature": signature(&key, &host_id, &nonce, LINK_PATH),
            "host_display_name": "Shared Mac"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let nonce = issue_nonce(&mut app, &host_id).await;
    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &auth_b)],
        json!({
            "installation_id": host_id,
            "nonce": nonce,
            "public_key": pubkey,
            "signature": signature(&key, &host_id, &nonce, LINK_PATH),
            "host_display_name": "Shared Mac"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "host_linked_elsewhere");
}

#[tokio::test]
async fn same_account_re_link_rotates_token() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let (access, account_id, _) = register_desktop(&state, "re-link-rotate@example.com").await;
    let auth = format!("Bearer {access}");

    let host = DeviceId::new();
    let host_id = host.to_string();
    let key = signing_key(71);
    let pubkey = public_key(&key);

    let nonce = issue_nonce(&mut app, &host_id).await;
    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &auth)],
        json!({
            "installation_id": host_id,
            "nonce": nonce,
            "public_key": pubkey,
            "signature": signature(&key, &host_id, &nonce, LINK_PATH),
            "host_display_name": "Rotate Mac"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let token1 = body["data"]["host_installation_token"]
        .as_str()
        .unwrap()
        .to_string();

    let nonce = issue_nonce(&mut app, &host_id).await;
    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &auth)],
        json!({
            "installation_id": host_id,
            "nonce": nonce,
            "public_key": pubkey,
            "signature": signature(&key, &host_id, &nonce, LINK_PATH),
            "host_display_name": "Rotate Mac"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let token2 = body["data"]["host_installation_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(token1, token2);
    assert!(host_links::exists(&state.store, host, &account_id)
        .await
        .unwrap());

    let old_hash = minos_backend::host_link::sha256_hex(&token1);
    let active_old = host_installation_tokens::verify_active_token(
        &state.store,
        &old_hash,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .unwrap();
    assert!(
        active_old.is_none(),
        "prior token must be revoked on re-link"
    );

    let new_hash = minos_backend::host_link::sha256_hex(&token2);
    let active_new = host_installation_tokens::verify_active_token(
        &state.store,
        &new_hash,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .unwrap();
    assert!(active_new.is_some());
}

#[tokio::test]
async fn invalid_nonce_is_rejected() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let (access, _, _) = register_desktop(&state, "bad-nonce@example.com").await;
    let auth = format!("Bearer {access}");

    let host_id = DeviceId::new().to_string();
    let key = signing_key(61);
    let (status, body) = post_json(
        &mut app,
        "/v1/hosts/link",
        &[("authorization", &auth)],
        json!({
            "installation_id": host_id,
            "nonce": "nonce_notreal",
            "public_key": public_key(&key),
            "signature": signature(&key, &host_id, "nonce_notreal", LINK_PATH),
            "host_display_name": "Mac"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "bootstrap_nonce_invalid");
}
