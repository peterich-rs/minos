use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

pub async fn get() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        crate::telemetry::render(),
    )
}
