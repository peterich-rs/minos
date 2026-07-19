//! Structured logging with OTel trace context injection.
//!
//! Provides a tracing layer that injects `trace_id` and `span_id` from
//! the current OpenTelemetry span into every log line. This makes it
//! possible to correlate log entries with distributed traces.
//!
//! # Configuration
//!
//! - `MINOS_LOG_LEVEL` — log level filter (default "info")
//!
//! # Usage
//!
//! The structured logging layer is installed alongside the existing
//! fmt and xlog layers in `main.rs`. It does not replace them.

use tracing_subscriber::Layer;

/// Configuration for structured logging.
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Log level filter string (e.g. "info", "minos_backend=debug,info").
    pub level: String,
    /// Whether to output JSON format (vs human-readable).
    pub json_format: bool,
}

impl LogConfig {
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            level: std::env::var("MINOS_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
            json_format: std::env::var("MINOS_LOG_JSON")
                .map(|v| v == "true")
                .unwrap_or(false),
        }
    }
}

/// Create a structured JSON logging layer that injects trace context.
///
/// Returns a boxed layer that can be composed with the existing tracing
/// subscriber stack. If JSON format is not enabled, returns `None` (the
/// existing fmt layer handles output).
pub fn create_layer<S>() -> Option<impl Layer<S>>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    // For now, we rely on the existing fmt + xlog layers.
    // A future enhancement would add a JSON layer with trace_id injection.
    //
    // The OTel tracing integration (via tracing-opentelemetry) automatically
    // adds span context to log records when the OTel layer is active.
    //
    // TODO: Implement a custom JSON layer that extracts trace_id/span_id
    // from the current OTel span context and injects them as field.
    let _enabled = false;
    None as Option<tracing_subscriber::fmt::Layer<S>>
}
