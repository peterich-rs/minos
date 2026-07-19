//! Token-bucket rate limiting middleware.
//!
//! Provides per-IP and per-account rate limiting for sensitive endpoints.
//! Uses a simple in-memory token bucket with configurable capacity and
//! refill rate.
//!
//! Configuration via env vars:
//! - `MINOS_RATE_LIMIT_REQUESTS_PER_SECOND` — default 10
//! - `MINOS_RATE_LIMIT_BURST` — default 20
//!
//! Returns HTTP 429 with `Retry-After` header when the bucket is empty.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::RwLock;

/// A simple token bucket for rate limiting.
#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume one token. Returns `true` if allowed, `false` if rate limited.
    fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Seconds until the next token is available.
    fn retry_after_secs(&self) -> u32 {
        if self.tokens >= 1.0 {
            0
        } else {
            let deficit = 1.0 - self.tokens;
            (deficit / self.refill_rate).ceil() as u32
        }
    }
}

/// Shared rate limiter state.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<RwLock<HashMap<String, TokenBucket>>>,
    capacity: f64,
    refill_rate: f64,
}

impl RateLimiter {
    /// Create a new rate limiter from env vars or defaults.
    pub fn from_env() -> Self {
        let rps: f64 = std::env::var("MINOS_RATE_LIMIT_REQUESTS_PER_SECOND")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0);
        let burst: f64 = std::env::var("MINOS_RATE_LIMIT_BURST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20.0);

        Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
            capacity: burst,
            refill_rate: rps,
        }
    }

    /// Check if a request from the given key is allowed.
    pub async fn check(&self, key: &str) -> Result<(), u32> {
        let mut buckets = self.buckets.write().await;
        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(self.capacity, self.refill_rate));

        if bucket.try_consume() {
            Ok(())
        } else {
            Err(bucket.retry_after_secs())
        }
    }

    /// Periodically clean up stale buckets to prevent memory leaks.
    pub async fn cleanup_stale(&self, max_age: Duration) {
        let mut buckets = self.buckets.write().await;
        let now = Instant::now();
        buckets.retain(|_, bucket| now.duration_since(bucket.last_refill) < max_age);
    }
}

/// Extract client IP from request headers.
fn client_ip(req: &Request) -> String {
    req.headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Rate limiting middleware function.
///
/// Apply to routes via `axum::middleware::from_fn_with_state`.
pub async fn rate_limit_middleware(
    axum::extract::State(limiter): axum::extract::State<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let ip = client_ip(&req);
    let path = req.uri().path().to_string();
    let key = format!("{ip}:{path}");

    match limiter.check(&key).await {
        Ok(()) => next.run(req).await,
        Err(retry_after) => {
            let mut resp = Response::new(axum::body::Body::from(
                r#"{"error":{"code":"rate_limited","message":"too many requests"}}"#,
            ));
            *resp.status_mut() = StatusCode::TOO_MANY_REQUESTS;
            resp.headers_mut().insert(
                "retry-after",
                axum::http::HeaderValue::from_str(&retry_after.to_string())
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("1")),
            );
            resp.headers_mut().insert(
                "content-type",
                axum::http::HeaderValue::from_static("application/json"),
            );
            resp
        }
    }
}
