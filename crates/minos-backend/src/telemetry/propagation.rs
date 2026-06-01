//! W3C Trace Context propagation utilities.
//!
//! Provides helpers for extracting and injecting W3C `traceparent` and
//! `tracestate` headers from HTTP requests. This enables distributed
//! tracing across service boundaries.
//!
//! # Usage
//!
//! In HTTP middleware or handlers, extract the trace context from incoming
//! headers and use it to create child spans:
//!
//! ```ignore
//! let parent_ctx = propagation::extract_from_headers(request.headers());
//! let span = tracing::info_span!("handle_request", parent_context = ?parent_ctx);
//! ```

use opentelemetry::propagation::Extractor;

/// A wrapper around `axum::http::HeaderMap` that implements the OTel
/// `Extractor` trait for trace context extraction.
pub struct HeaderExtractor<'a> {
    headers: &'a axum::http::HeaderMap,
}

impl<'a> HeaderExtractor<'a> {
    #[must_use]
    pub fn new(headers: &'a axum::http::HeaderMap) -> Self {
        Self { headers }
    }
}

impl<'a> Extractor for HeaderExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.headers
            .keys()
            .map(|k| k.as_str())
            .collect()
    }
}

/// Extract the remote trace context from HTTP headers.
///
/// Returns the extracted `opentelemetry::Context` which can be used as
/// the parent context for new spans.
#[must_use]
pub fn extract_from_headers(headers: &axum::http::HeaderMap) -> opentelemetry::Context {
    let extractor = HeaderExtractor::new(headers);
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&extractor)
    })
}

/// Inject the current trace context into HTTP headers (for outgoing requests).
pub fn inject_into_headers(context: &opentelemetry::Context, headers: &mut axum::http::HeaderMap) {
    struct HeaderInjector<'a> {
        headers: &'a mut axum::http::HeaderMap,
    }

    impl<'a> opentelemetry::propagation::Injector for HeaderInjector<'a> {
        fn set(&mut self, key: &str, value: String) {
            if let Ok(name) = axum::http::HeaderName::from_bytes(key.as_bytes()) {
                if let Ok(val) = axum::http::HeaderValue::from_str(&value) {
                    self.headers.insert(name, val);
                }
            }
        }
    }

    let mut injector = HeaderInjector { headers };
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(context, &mut injector);
    });
}

/// Initialize the global W3C trace context propagator.
///
/// Should be called once during application startup, before any spans
/// are created. Uses W3C TraceContext as the primary propagator.
pub fn init_propagator() {
    let propagator = opentelemetry_sdk::propagation::TraceContextPropagator::new();
    opentelemetry::global::set_text_map_propagator(propagator);
}
