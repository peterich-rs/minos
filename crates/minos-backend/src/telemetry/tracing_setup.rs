//! OpenTelemetry tracer setup.
//!
//! Configures an OTLP/gRPC exporter for distributed tracing. The tracer
//! is optional -- if `MINOS_OTLP_ENDPOINT` is not set, OTel tracing is
//! disabled and only the existing `tracing` subscriber (fmt + xlog) is
//! active.
//!
//! # Configuration
//!
//! - `MINOS_OTLP_ENDPOINT` -- OTLP collector URL (e.g. `http://localhost:4317`)
//! - `MINOS_TRACE_SAMPLE_RATIO` -- sampling ratio 0.0..1.0 (default 1.0 in dev, 0.05 in prod)
//!
//! # Dependencies
//!
//! Requires `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`,
//! and `tracing-opentelemetry` crates in Cargo.toml.

use std::time::Duration;

/// Configuration for the OTel tracing subsystem.
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// OTLP collector endpoint. If `None`, OTel tracing is disabled.
    pub otlp_endpoint: Option<String>,
    /// Sampling ratio (0.0 to 1.0). Default 1.0 for dev.
    pub sample_ratio: f64,
}

impl TracingConfig {
    /// Build from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            otlp_endpoint: std::env::var("MINOS_OTLP_ENDPOINT").ok(),
            sample_ratio: std::env::var("MINOS_TRACE_SAMPLE_RATIO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
        }
    }
}

/// Guard that flushes and shuts down the OTel tracer on drop.
pub struct TracingGuard {
    _active: bool,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        if self._active {
            opentelemetry::global::shutdown_tracer_provider();
        }
    }
}

/// Initialize the OTel tracer. Returns a guard that must be held for the
/// lifetime of the process.
///
/// If `MINOS_OTLP_ENDPOINT` is not set, returns a no-op guard.
pub fn init(config: &TracingConfig) -> TracingGuard {
    let Some(ref endpoint) = config.otlp_endpoint else {
        tracing::info!(
            target: "minos_backend::telemetry",
            "OTel tracing disabled (MINOS_OTLP_ENDPOINT not set)"
        );
        return TracingGuard { _active: false };
    };

    match init_otlp(endpoint, config.sample_ratio) {
        Ok(()) => {
            tracing::info!(
                target: "minos_backend::telemetry",
                endpoint = %endpoint,
                sample_ratio = config.sample_ratio,
                "OTel tracing initialized"
            );
            TracingGuard { _active: true }
        }
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::telemetry",
                error = %error,
                "failed to initialize OTel tracing; continuing without distributed tracing"
            );
            TracingGuard { _active: false }
        }
    }
}

fn init_otlp(
    endpoint: &str,
    _sample_ratio: f64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use opentelemetry_otlp::WithExportConfig;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(5))
        .build()?;

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();

    opentelemetry::global::set_tracer_provider(provider);

    Ok(())
}
