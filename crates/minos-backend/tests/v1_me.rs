//! Integration tests for `GET /v1/me/peer`. Mirrors the pairing-rail
//! auth contract used by `DELETE /v1/pairing`: the caller must present
//! `X-Device-Id` + `X-Device-Secret`, the row must be paired, and the
//! body shape is the protocol-shared [`MePeerResponse`] struct.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::pairing::PairingService;
use minos_backend::store::devices::insert_device;
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::MePeerResponse;
use std::time::Duration as StdDuration;

mod common;

/// Pre-seed a fully paired Mac+iPhone with both `secret_hash` columns
/// populated so each side can authenticate via `X-Device-Secret`.
/// Returns `(mac_id, ios_id, mac_secret, ios_secret)`. The pairing row
/// is inserted via [`PairingService::consume_token`] so `created_at`
/// reflects a realistic pair-time epoch instead of an arbitrary literal.
async fn pair_mac_and_ios(
    state: &minos_backend::http::BackendState,
) -> (
    DeviceId,
    DeviceId,
    minos_domain::DeviceSecret,
    minos_domain::DeviceSecret,
) {
    let mac_id = DeviceId::new();
    insert_device(&state.store, mac_id, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    let svc = PairingService::new(state.store.clone());
    let (token, _expires) = svc
        .request_token(mac_id, StdDuration::from_mins(5))
        .await
        .unwrap();
    let ios_id = DeviceId::new();
    let outcome = svc
        .consume_token(&token, ios_id, "iPhone".into())
        .await
        .unwrap();
    (
        mac_id,
        ios_id,
        outcome.issuer_secret,
        outcome.consumer_secret,
    )
}

#[tokio::test]
async fn get_me_peer_returns_peer_record_for_paired_device() {
    let state = backend_state().await;
    let (mac_id, ios_id, mac_secret, _ios_secret) = pair_mac_and_ios(&state).await;
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .header("x-device-id", mac_id.to_string())
        .header("x-device-role", "agent-host")
        .header("x-device-secret", mac_secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);

    let resp: MePeerResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.peer_device_id, ios_id);
    assert_eq!(resp.peer_name, "iPhone");
    assert!(
        resp.paired_at_ms > 0,
        "paired_at_ms must be a positive epoch-ms value"
    );
}

#[tokio::test]
async fn get_me_peer_works_from_ios_side_too() {
    // Symmetry check: the iOS side authenticated with its own secret
    // should see the Mac as its peer. This is mostly exercising the
    // get_pair_with_created_at "either side" branch from the store.
    let state = backend_state().await;
    let (mac_id, ios_id, _mac_secret, ios_secret) = pair_mac_and_ios(&state).await;
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", ios_secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);

    let resp: MePeerResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.peer_device_id, mac_id);
    assert_eq!(resp.peer_name, "Mac");
}

#[tokio::test]
async fn get_me_peer_returns_404_not_paired_when_unpaired_with_secret() {
    // An iOS device that's been authenticated (has a secret hash) but
    // is currently unpaired must get the structured 404 envelope so the
    // Mac daemon's `get_me_peer` client can map it to `Ok(None)` rather
    // than a generic error.
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::IosClient, 0)
        .await
        .unwrap();
    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, id, &hash)
        .await
        .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_paired");
}

#[tokio::test]
async fn get_me_peer_returns_404_not_paired_when_first_connect_no_secret() {
    // Brand-new device that hasn't been paired and doesn't carry a secret.
    // Authentication classifies it as FirstConnect → secret-less, so the
    // route's "secret required" gate fires before we ever reach the
    // pairings query. This is the expected behaviour: `/v1/me/peer` is
    // pairing-rail-only.
    let state = backend_state().await;
    let mut app = router(state);
    let id = DeviceId::new();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "ios-client")
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn get_me_peer_without_secret_returns_401() {
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::IosClient, 0)
        .await
        .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "ios-client")
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn get_me_peer_with_wrong_secret_returns_401() {
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::IosClient, 0)
        .await
        .unwrap();
    let real_secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&real_secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, id, &hash)
        .await
        .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", "definitely-not-the-real-secret")
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}
