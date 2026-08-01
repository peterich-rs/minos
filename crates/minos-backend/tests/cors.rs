use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use minos_backend::http;
use minos_backend::http::test_support::backend_state;
use tower::ServiceExt as _;

#[tokio::test]
async fn preflight_allows_browser_admin_headers() {
    let state = backend_state().await;
    let app = http::router(state);

    let request = Request::builder()
        .method("OPTIONS")
        .uri("/v1/auth/supabase")
        .header("origin", "http://127.0.0.1:4173")
        .header("access-control-request-method", "POST")
        .header(
            "access-control-request-headers",
            "authorization,content-type,x-device-id,x-device-role",
        )
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let headers = response.headers();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
    let allow_headers = headers
        .get("access-control-allow-headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(allow_headers.contains("authorization"));
    assert!(allow_headers.contains("x-device-id"));
    assert!(allow_headers.contains("x-device-role"));

    let _ = response.into_body().collect().await.unwrap();
}
