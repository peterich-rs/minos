use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use minos_backend::auth::jwt;
use minos_backend::http;
use minos_backend::http::test_support::{backend_state, TEST_JWT_SECRET};
use minos_backend::session::SessionHandle;
use minos_backend::store::{device_installations, host_links};
use minos_domain::{DeviceId, DeviceRole};
use serde_json::json;

mod common;

const REQUEST_CODE_PATH: &str = "/v1/host/pairing/request-code";
const REDEEM_PATH: &str = "/v1/host/pairing/redeem";

struct HostFixture {
    host: DeviceId,
    mobile: DeviceId,
    account_id: String,
    token: String,
    account_auth_header: String,
}

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public_key(signing_key: &SigningKey) -> String {
    format!(
        "ed25519:{}",
        URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes())
    )
}

fn signature(signing_key: &SigningKey, installation_id: &str, nonce: &str) -> String {
    signature_for_path(signing_key, installation_id, nonce, REQUEST_CODE_PATH)
}

fn signature_for_path(
    signing_key: &SigningKey,
    installation_id: &str,
    nonce: &str,
    path: &str,
) -> String {
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
        .header("x-request-id", "req_host_contract");
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    common::send(app, builder.body(json_body(body)).unwrap()).await
}

#[tokio::test]
async fn host_pairing_request_code_requires_ed25519_bootstrap_proof() {
    let state = backend_state().await;
    let installation_id = DeviceId::new().to_string();
    let mut app = http::router(state.clone());

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["meta"]["request_id"], "req_host_contract");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let signing_key = signing_key(11);
    let public_key = public_key(&signing_key);
    let signature = signature(&signing_key, &installation_id, &nonce);

    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": public_key,
            "signature": signature,
            "host_display_name": "Formal Host"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["meta"]["request_id"], "req_host_contract");
    assert_eq!(
        body["data"]["qr_payload"]["host_display_name"],
        "Formal Host"
    );
    assert!(!body["data"]["qr_payload"]["pairing_token"]
        .as_str()
        .unwrap()
        .is_empty());
    let stored = device_installations::get_device(
        &state.store,
        uuid::Uuid::parse_str(&installation_id)
            .map(DeviceId)
            .unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(stored.public_key.as_deref(), Some(public_key.as_str()));
}

#[tokio::test]
async fn host_pairing_request_code_rejects_nonce_reuse_and_key_mismatch() {
    let state = backend_state().await;
    let installation_id = DeviceId::new().to_string();
    let mut app = http::router(state.clone());
    let host_signing_key = signing_key(12);
    let host_public_key = public_key(&host_signing_key);

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature(&host_signing_key, &installation_id, &nonce);

    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": host_public_key.clone(),
            "signature": sig,
            "host_display_name": "Formal Host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let replay_sig = signature(&host_signing_key, &installation_id, &nonce);
    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": host_public_key,
            "signature": replay_sig,
            "host_display_name": "Formal Host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "host_bootstrap_nonce_invalid");

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let different_key = signing_key(13);
    let different_public_key = public_key(&different_key);
    let different_sig = signature(&different_key, &installation_id, &nonce);

    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": different_public_key,
            "signature": different_sig,
            "host_display_name": "Formal Host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "host_bootstrap_proof_invalid");
}

#[tokio::test]
async fn host_pairing_request_code_allows_omitted_public_key_after_tofu() {
    let state = backend_state().await;
    let installation_id = DeviceId::new().to_string();
    let mut app = http::router(state);
    let signing_key = signing_key(14);
    let public_key = public_key(&signing_key);

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature(&signing_key, &installation_id, &nonce);
    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": public_key,
            "signature": sig,
            "host_display_name": "Formal Host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature(&signing_key, &installation_id, &nonce);

    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "signature": sig,
            "host_display_name": "Formal Host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
}

#[tokio::test]
async fn host_pairing_redeem_reports_not_confirmed_while_mobile_is_pending() {
    let state = backend_state().await;
    let installation_id = DeviceId::new().to_string();
    let mut app = http::router(state);
    let signing_key = signing_key(15);
    let host_public_key = public_key(&signing_key);

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature(&signing_key, &installation_id, &nonce);
    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": host_public_key,
            "signature": sig,
            "host_display_name": "Formal Host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let pairing_code = body["data"]["qr_payload"]["pairing_token"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature_for_path(&signing_key, &installation_id, &nonce, REDEEM_PATH);

    let (status, body) = post_json(
        &mut app,
        REDEEM_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": host_public_key,
            "signature": sig,
            "pairing_code": pairing_code,
            "client_request_id": "redeem-before-confirm"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "pairing_not_confirmed");
}

async fn formally_paired_host(
    state: &minos_backend::http::BackendState,
    app: &mut axum::Router,
) -> HostFixture {
    let account =
        minos_backend::store::accounts::create(&state.store, "formal-host-self@example.com", "phc")
            .await
            .unwrap();
    let mobile = DeviceId::new();
    let host = DeviceId::new();
    let installation_id = host.to_string();
    let signing_key = signing_key(21);
    let host_public_key = public_key(&signing_key);

    device_installations::insert_device(
        &state.store,
        mobile,
        "Owner Phone",
        DeviceRole::MobileClient,
        100,
    )
    .await
    .unwrap();
    device_installations::set_account_id(&state.store, &mobile, &account.account_id)
        .await
        .unwrap();

    let (status, body) = post_json(
        app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature(&signing_key, &installation_id, &nonce);
    let (status, body) = post_json(
        app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": host_public_key,
            "signature": sig,
            "host_display_name": "Linux Workstation"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let pairing_code = body["data"]["qr_payload"]["pairing_token"]
        .as_str()
        .unwrap()
        .to_string();

    let bearer = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        &account.account_id,
        &mobile.to_string(),
    )
    .unwrap();
    let auth_header = format!("Bearer {bearer}");
    let (status, body) = post_json(
        app,
        "/v1/pairing/confirm",
        &[("authorization", &auth_header)],
        json!({
            "pairing_code": pairing_code,
            "client_request_id": "confirm-formal-host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["host_installation_id"], host.to_string());
    assert_eq!(body["data"]["status"], "confirmed");
    assert_eq!(body["data"]["already_confirmed"], false);

    let (status, body) = post_json(
        app,
        "/v1/pairing/confirm",
        &[("authorization", &auth_header)],
        json!({
            "pairing_code": pairing_code,
            "client_request_id": "confirm-formal-host-retry"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["already_confirmed"], true);

    let (status, body) = post_json(
        app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature_for_path(&signing_key, &installation_id, &nonce, REDEEM_PATH);
    let (status, body) = post_json(
        app,
        REDEEM_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "signature": sig,
            "pairing_code": pairing_code,
            "client_request_id": "redeem-formal-host"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let token = body["data"]["host_installation_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(token.starts_with("hit_"));

    assert!(host_links::exists(&state.store, host, &account.account_id,)
        .await
        .unwrap());
    device_installations::touch_last_seen(&state.store, &mobile, 100)
        .await
        .unwrap();

    HostFixture {
        host,
        mobile,
        account_id: account.account_id,
        token,
        account_auth_header: auth_header,
    }
}

fn host_headers(fixture: &HostFixture) -> Vec<(&'static str, String)> {
    vec![("authorization", format!("Bearer {}", fixture.token))]
}

#[tokio::test]
async fn host_installations_self_returns_host_view_without_account_pii() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let fixture = formally_paired_host(&state, &mut app).await;
    let (mobile_session, _mobile_outbox) =
        SessionHandle::new(fixture.mobile, DeviceRole::MobileClient);
    mobile_session.set_account_id(fixture.account_id.clone());
    state.registry.insert(mobile_session);
    let headers = host_headers(&fixture);
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();

    let (status, body) = post_json(
        &mut app,
        "/v1/host/installations/self",
        &header_refs,
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["meta"]["request_id"], "req_host_contract");
    assert_eq!(
        body["data"]["host_installation_id"],
        fixture.host.to_string()
    );
    assert_eq!(body["data"]["display_name"], "Linux Workstation");
    assert_eq!(body["data"]["link_count"], 1);
    assert_eq!(
        body["data"]["links"][0]["linked_via_installation_id"],
        fixture.mobile.to_string()
    );
    assert_eq!(body["data"]["links"][0]["link_display_name"], "Owner Phone");
    assert_eq!(body["data"]["links"][0]["last_active_at_ms"], 100);
    assert_eq!(body["data"]["links"][0]["online"], true);
    let body_text = serde_json::to_string(&body).unwrap();
    assert!(!body_text.contains("formal-host-self@example.com"));
    assert!(body["data"]["links"][0].get("account_email").is_none());
}

#[tokio::test]
async fn host_installations_self_uses_account_presence_not_paired_via_device_presence() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let fixture = formally_paired_host(&state, &mut app).await;
    let current_mobile = DeviceId::new();
    device_installations::insert_device(
        &state.store,
        current_mobile,
        "Current iPhone",
        DeviceRole::MobileClient,
        200,
    )
    .await
    .unwrap();
    device_installations::set_account_id(&state.store, &current_mobile, &fixture.account_id)
        .await
        .unwrap();
    let (mobile_session, _mobile_outbox) =
        SessionHandle::new(current_mobile, DeviceRole::MobileClient);
    mobile_session.set_account_id(fixture.account_id.clone());
    state.registry.insert(mobile_session);

    let headers = host_headers(&fixture);
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();

    let (status, body) = post_json(
        &mut app,
        "/v1/host/installations/self",
        &header_refs,
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["links"][0]["linked_via_installation_id"],
        fixture.mobile.to_string()
    );
    assert_eq!(body["data"]["links"][0]["online"], true);
    assert_eq!(body["data"]["links"][0]["last_active_at_ms"], 200);
}

#[tokio::test]
async fn host_installations_self_does_not_count_browser_admin_as_mobile_online() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let fixture = formally_paired_host(&state, &mut app).await;
    let browser_id = DeviceId::new();
    let (browser_session, _browser_outbox) =
        SessionHandle::new(browser_id, DeviceRole::BrowserAdmin);
    browser_session.set_account_id(fixture.account_id.clone());
    state.registry.insert(browser_session);

    let headers = host_headers(&fixture);
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();

    let (status, body) = post_json(
        &mut app,
        "/v1/host/installations/self",
        &header_refs,
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["links"][0]["online"], false);
}

#[tokio::test]
async fn host_realtime_ws_ticket_binds_ticket_to_host_installation() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let fixture = formally_paired_host(&state, &mut app).await;
    let headers = host_headers(&fixture);
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();

    let (status, body) = post_json(
        &mut app,
        "/v1/host/realtime/ws-ticket",
        &header_refs,
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["meta"]["request_id"], "req_host_contract");
    assert!(body["data"]["gateway_url"]
        .as_str()
        .unwrap()
        .starts_with("/ws/host?ticket="));
    let ticket = body["data"]["ticket"].as_str().unwrap();
    let claims = jwt::verify_ws_ticket(TEST_JWT_SECRET.as_bytes(), ticket).unwrap();
    assert_eq!(claims.sub, fixture.host.to_string());
    assert_eq!(claims.did, fixture.host.to_string());
    assert_eq!(claims.role, DeviceRole::AgentHost);
}

#[tokio::test]
async fn host_realtime_ws_ticket_rejects_legacy_device_secret() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let fixture = formally_paired_host(&state, &mut app).await;
    let host_id = fixture.host.to_string();

    let (status, body) = post_json(
        &mut app,
        "/v1/host/realtime/ws-ticket",
        &[("x-device-id", &host_id), ("x-device-role", "agent-host")],
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn pairing_revoke_revokes_last_host_installation_token() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let fixture = formally_paired_host(&state, &mut app).await;

    let (status, body) = post_json(
        &mut app,
        "/v1/pairing/revoke",
        &[("authorization", fixture.account_auth_header.as_str())],
        json!({
            "host_installation_id": fixture.host.to_string(),
            "client_request_id": "revoke-formal-host"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["meta"]["request_id"], "req_host_contract");
    assert_eq!(
        body["data"]["host_installation_id"],
        fixture.host.to_string()
    );
    assert_eq!(body["data"]["revoked"], true);
    assert_eq!(body["data"]["remaining_link_count"], 0);
    assert_eq!(body["data"]["host_installation_token_revoked"], true);

    let headers = host_headers(&fixture);
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    let (status, body) = post_json(
        &mut app,
        "/v1/host/installations/self",
        &header_refs,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn pairing_revoke_keeps_host_token_when_other_account_link_remains() {
    let state = backend_state().await;
    let mut app = http::router(state.clone());
    let fixture = formally_paired_host(&state, &mut app).await;

    let second_account = minos_backend::store::accounts::create(
        &state.store,
        "formal-host-second-owner@example.com",
        "phc",
    )
    .await
    .unwrap();
    let second_mobile = DeviceId::new();
    device_installations::insert_device(
        &state.store,
        second_mobile,
        "Second Owner Phone",
        DeviceRole::MobileClient,
        200,
    )
    .await
    .unwrap();
    device_installations::set_account_id(&state.store, &second_mobile, &second_account.account_id)
        .await
        .unwrap();

    let installation_id = fixture.host.to_string();
    let signing_key = signing_key(21);
    let host_public_key = public_key(&signing_key);

    let (status, body) = post_json(
        &mut app,
        "/v1/host/bootstrap/nonce",
        &[],
        json!({"installation_id": installation_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let nonce = body["data"]["nonce"].as_str().unwrap().to_string();
    let sig = signature(&signing_key, &installation_id, &nonce);
    let (status, body) = post_json(
        &mut app,
        REQUEST_CODE_PATH,
        &[],
        json!({
            "installation_id": installation_id,
            "nonce": nonce,
            "public_key": host_public_key,
            "signature": sig,
            "host_display_name": "Linux Workstation"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let second_pairing_code = body["data"]["qr_payload"]["pairing_token"]
        .as_str()
        .unwrap()
        .to_string();

    let second_bearer = jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        &second_account.account_id,
        &second_mobile.to_string(),
    )
    .unwrap();
    let second_auth_header = format!("Bearer {second_bearer}");
    let (status, body) = post_json(
        &mut app,
        "/v1/pairing/confirm",
        &[("authorization", &second_auth_header)],
        json!({
            "pairing_code": second_pairing_code,
            "client_request_id": "confirm-formal-host-second-owner"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["host_installation_id"],
        fixture.host.to_string()
    );
    assert_eq!(body["data"]["already_confirmed"], false);

    let (status, body) = post_json(
        &mut app,
        "/v1/pairing/revoke",
        &[("authorization", fixture.account_auth_header.as_str())],
        json!({
            "host_installation_id": fixture.host.to_string(),
            "client_request_id": "revoke-formal-host-first-owner"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["remaining_link_count"], 1);
    assert_eq!(body["data"]["host_installation_token_revoked"], false);

    let headers = host_headers(&fixture);
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    let (status, body) = post_json(
        &mut app,
        "/v1/host/installations/self",
        &header_refs,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["link_count"], 1);
}
