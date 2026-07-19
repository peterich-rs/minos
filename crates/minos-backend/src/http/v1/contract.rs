use axum::http::HeaderMap;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ResponseEnvelope<T> {
    pub data: T,
    pub meta: ResponseMeta,
}

#[derive(Debug, Serialize)]
pub struct ResponseMeta {
    pub request_id: String,
}

impl<T> ResponseEnvelope<T> {
    pub fn new(data: T, request_id: String) -> Self {
        Self {
            data,
            meta: ResponseMeta { request_id },
        }
    }
}

pub fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}
