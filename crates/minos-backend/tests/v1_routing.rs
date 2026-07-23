use axum::body::Body;
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

#[tokio::test]
async fn legacy_auth_ws_ticket_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/auth/ws-ticket")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retired_v1_me_routes_return_404() {
    let state = backend_state().await;
    let mut app = router(state);

    for path in [
        "/v1/me/hosts/query",
        "/v1/me/peer/query",
        "/v1/me/peers/query",
        "/v1/me/profile/query",
        "/v1/me/profile/minos-id",
        "/v1/me/profile/display-name",
        "/v1/users/search/query",
    ] {
        let req = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let (status, _) = common::send(&mut app, req).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "path={path}");
    }

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/v1/me/peers/00000000-0000-0000-0000-000000000000")
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retired_v1_thread_routes_return_404() {
    let state = backend_state().await;
    let mut app = router(state);

    for path in [
        "/v1/sessions",
        "/v1/sessions/query",
        "/v1/sessions/read",
        "/v1/sessions/last-seq",
        "/v1/sessions/thread_probe/events",
        "/v1/sessions/thread_probe/last_seq",
        "/v1/sessions/approval-decision",
    ] {
        let req = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let (status, _) = common::send(&mut app, req).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "path={path}");
    }
}
