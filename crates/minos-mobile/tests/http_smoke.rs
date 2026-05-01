//! Round-trip tests for [`minos_mobile::http::MobileHttpClient`] against an
//! in-process `minos-backend` axum router.

// MSRV portability: prefer `Duration::from_secs(N * 60)` over
// `Duration::from_mins(N)` (which was only stabilized in Rust 1.84). See
// the matching crate-level allow in `src/lib.rs`.
#![allow(clippy::duration_suboptimal_units)]

use minos_backend::http::{router, test_support::backend_state};
use minos_domain::{DeviceId, DeviceRole, MinosError};
use minos_mobile::http::MobileHttpClient;
use minos_protocol::PairConsumeRequest;

#[tokio::test(flavor = "multi_thread")]
async fn pair_consume_round_trips_against_real_backend() {
    let state = backend_state().await;
    let mac_id = DeviceId::new();
    minos_backend::store::devices::insert_device(
        &state.store,
        mac_id,
        "Mac",
        DeviceRole::AgentHost,
        0,
    )
    .await
    .unwrap();
    let svc = minos_backend::pairing::PairingService::new(state.store.clone());
    let (token, _) = svc
        .request_token(mac_id, std::time::Duration::from_secs(300))
        .await
        .unwrap();

    // Seed a live Mac session so consume can deliver Event::Paired.
    let (handle, mut mac_outbox) =
        minos_backend::session::SessionHandle::new(mac_id, DeviceRole::AgentHost);
    state.registry.insert(handle);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let consumer_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), consumer_id, None).unwrap();

    // ADR-0020: bearer-only iOS rail. Register an account bound to this
    // device id so we can stamp the Bearer.
    let auth = client
        .register("pairsmoke@example.com", "testpass1")
        .await
        .unwrap();

    let resp = client
        .pair_consume(
            PairConsumeRequest {
                token,
                device_name: "iPhone".into(),
            },
            &auth.access_token,
        )
        .await
        .unwrap();

    assert_eq!(resp.peer_device_id, mac_id);
    assert_eq!(resp.peer_name, "Mac");
    let _ = mac_outbox.recv().await.unwrap(); // Event::Paired delivered
}

/// Spawn the test backend and return its bound address.
async fn spawn_backend() -> std::net::SocketAddr {
    let state = backend_state().await;
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_register_round_trips_against_real_backend() {
    let addr = spawn_backend().await;
    let device_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), device_id, None).unwrap();

    let resp = client
        .register("smoke@example.com", "testpass1")
        .await
        .expect("register should succeed");
    assert_eq!(resp.account.email, "smoke@example.com");
    assert!(!resp.access_token.is_empty());
    assert!(!resp.refresh_token.is_empty());
    assert!(resp.expires_in > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_register_then_login_then_refresh_roundtrip() {
    let addr = spawn_backend().await;
    let device_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), device_id, None).unwrap();

    let registered = client
        .register("flow@example.com", "testpass1")
        .await
        .unwrap();
    let logged_in = client
        .login("flow@example.com", "testpass1")
        .await
        .unwrap();
    assert_ne!(
        logged_in.refresh_token, registered.refresh_token,
        "login mints a fresh refresh token"
    );

    let refreshed = client
        .refresh(&logged_in.refresh_token)
        .await
        .expect("refresh should succeed against the live backend");
    assert!(!refreshed.access_token.is_empty());
    assert!(!refreshed.refresh_token.is_empty());
    assert_ne!(refreshed.refresh_token, logged_in.refresh_token);
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_register_with_weak_password_maps_to_weak_password_variant() {
    let addr = spawn_backend().await;
    let device_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), device_id, None).unwrap();

    let err = client
        .register("weak@example.com", "short")
        .await
        .expect_err("8-char minimum should reject `short`");
    assert!(
        matches!(err, MinosError::WeakPassword),
        "unexpected error: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_login_with_wrong_password_maps_to_invalid_credentials() {
    let addr = spawn_backend().await;
    let device_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), device_id, None).unwrap();
    let _ = client
        .register("wrong@example.com", "testpass1")
        .await
        .unwrap();
    let err = client
        .login("wrong@example.com", "incorrect")
        .await
        .expect_err("wrong password must be rejected");
    assert!(
        matches!(err, MinosError::InvalidCredentials),
        "unexpected error: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_register_duplicate_email_maps_to_email_taken() {
    let addr = spawn_backend().await;
    let device_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), device_id, None).unwrap();
    let _ = client
        .register("dup@example.com", "testpass1")
        .await
        .unwrap();
    let err = client
        .register("dup@example.com", "testpass2")
        .await
        .expect_err("duplicate email must be rejected");
    assert!(
        matches!(err, MinosError::EmailTaken),
        "unexpected error: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_logout_revokes_the_named_refresh_token() {
    let addr = spawn_backend().await;
    let device_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), device_id, None).unwrap();
    let resp = client
        .register("logout@example.com", "testpass1")
        .await
        .unwrap();

    client
        .logout(&resp.access_token, &resp.refresh_token)
        .await
        .expect("logout should return 204");

    // After logout, the same refresh token must no longer be accepted.
    let err = client
        .refresh(&resp.refresh_token)
        .await
        .expect_err("revoked refresh token must be rejected");
    assert!(
        matches!(
            err,
            MinosError::AuthRefreshFailed { .. } | MinosError::Unauthorized { .. }
        ),
        "unexpected error: {err:?}"
    );
}
