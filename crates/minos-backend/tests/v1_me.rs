//! Integration tests asserting the retired `/v1/me/*` surface stays absent.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_domain::DeviceId;

mod common;

#[tokio::test]
async fn retired_me_hosts_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/me/hosts/query")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retired_me_peer_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/me/peer/query")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retired_me_peers_delete_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/v1/me/peers/{}", DeviceId::new()))
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
