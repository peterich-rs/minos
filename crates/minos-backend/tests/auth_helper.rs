use axum::http::{HeaderMap, HeaderName, HeaderValue};
use minos_backend::http::auth::{authenticate, AuthError, AuthOutcome};
use minos_backend::store::test_support::insert_test_host;
use minos_backend::store::test_support::memory_pool;
use minos_domain::{DeviceId, DeviceRole};

fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        let name = HeaderName::from_bytes(k.as_bytes()).unwrap();
        h.insert(name, HeaderValue::from_str(v).unwrap());
    }
    h
}

#[tokio::test]
async fn first_connect_resolves_role_without_inserting() {
    // authenticate() must not insert: PG CHECK forbids null-account clients
    // and null-public_key hosts. Rows are created at session issuance / bootstrap.
    let pool = memory_pool().await;
    let id = DeviceId::new();
    let headers = header_map(&[
        ("x-device-id", &id.to_string()),
        ("x-device-role", "agent-host"),
        ("x-device-name", "Mac"),
    ]);

    let outcome = authenticate(&pool, &headers).await.unwrap();
    assert!(
        matches!(outcome, AuthOutcome { device_id, role: DeviceRole::AgentHost, .. } if device_id == id)
    );

    assert!(
        minos_backend::store::devices::get_device(&pool, id)
            .await
            .unwrap()
            .is_none(),
        "first-connect must not insert an installation row"
    );
}

#[tokio::test]
async fn missing_device_id_returns_unauthorized() {
    let pool = memory_pool().await;
    let headers = HeaderMap::new();
    let err = authenticate(&pool, &headers).await.unwrap_err();
    assert!(matches!(err, AuthError::Unauthorized(_)));
}

#[tokio::test]
async fn role_mismatch_against_existing_row_is_unauthorized() {
    let pool = memory_pool().await;
    let id = DeviceId::new();
    insert_test_host(&pool, id, "Mac", 0).await;
    let headers = header_map(&[
        ("x-device-id", &id.to_string()),
        ("x-device-role", "mobile-client"),
    ]);
    let err = authenticate(&pool, &headers).await.unwrap_err();
    assert!(matches!(err, AuthError::Unauthorized(_)));
}
