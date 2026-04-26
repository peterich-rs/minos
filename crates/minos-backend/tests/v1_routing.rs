use axum::http::{Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};

mod common;

#[tokio::test]
async fn unknown_v1_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/no-such-route")
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
