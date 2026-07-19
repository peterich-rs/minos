//! Telemetry subsystem: Prometheus metrics, OTel tracing, structured
//! logging, and W3C trace context propagation.
//!
//! Re-exports the public API from the previous single-file module so
//! existing callers (`crate::telemetry::*`) continue to work unchanged.

pub mod logs;
mod metrics;
pub mod propagation;
pub mod tracing_setup;

// Re-export everything from metrics so existing callers don't break.
pub use metrics::*;
