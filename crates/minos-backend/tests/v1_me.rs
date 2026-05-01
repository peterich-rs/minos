//! Integration tests for `GET /v1/me/macs` and the legacy
//! `/v1/me/peer` 410 redirect (see ADR-0020).
//!
//! `/v1/me/macs` is bearer-only; iOS callers see every Mac paired to
//! their account. The legacy `/v1/me/peer` always returns
//! `410 Gone` with an `error.code == "replaced"` body so older Mac
//! daemons get a clear migration signal.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};

mod common;

#[tokio::test]
async fn get_me_peer_returns_410_gone() {
    let state = backend_state().await;
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/peer")
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["error"]["code"], "replaced");
}

#[tokio::test]
async fn get_me_macs_without_bearer_returns_401() {
    let state = backend_state().await;
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/me/macs")
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}
