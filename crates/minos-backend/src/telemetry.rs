use std::sync::OnceLock;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

const SESSION_REGISTRY_SIZE: &str = "minos_backend_session_registry_size";
const INGEST_OUTBOX_DROPPED_TOTAL: &str = "minos_backend_ingest_outbox_dropped_total";
const HTTP_REQUEST_DURATION_SECONDS: &str = "minos_backend_http_request_duration_seconds";
const FORWARD_RPC_LATENCY_SECONDS: &str = "minos_backend_forward_rpc_latency_seconds";

fn prometheus_handle() -> &'static PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::set_global_recorder(recorder)
            .expect("prometheus recorder should install exactly once per process");

        metrics::describe_gauge!(
            SESSION_REGISTRY_SIZE,
            "Current number of live websocket sessions in the in-process registry."
        );
        metrics::describe_counter!(
            INGEST_OUTBOX_DROPPED_TOTAL,
            "Number of ingest fan-out frames dropped because the peer outbox rejected them."
        );
        metrics::describe_histogram!(
            HTTP_REQUEST_DURATION_SECONDS,
            metrics::Unit::Seconds,
            "HTTP request latency in seconds, labeled by route, method, and status."
        );
        metrics::describe_histogram!(
            FORWARD_RPC_LATENCY_SECONDS,
            metrics::Unit::Seconds,
            "Forwarded RPC round-trip latency in seconds, labeled by method and outcome."
        );
        metrics::gauge!(SESSION_REGISTRY_SIZE).set(0.0);
        handle
    })
}

pub fn init() {
    let _ = prometheus_handle();
}

#[must_use]
pub fn render() -> String {
    prometheus_handle().render()
}

pub fn set_session_registry_size(size: usize) {
    init();
    metrics::gauge!(SESSION_REGISTRY_SIZE).set(size as f64);
}

pub fn increment_ingest_outbox_dropped() {
    init();
    metrics::counter!(INGEST_OUTBOX_DROPPED_TOTAL).increment(1);
}

pub fn record_http_request(route: &str, method: &str, status: u16, seconds: f64) {
    init();
    metrics::histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        "route" => route.to_string(),
        "method" => method.to_string(),
        "status" => status.to_string()
    )
    .record(seconds);
}

pub fn record_forward_rpc(method: &str, outcome: &str, seconds: f64) {
    init();
    metrics::histogram!(
        FORWARD_RPC_LATENCY_SECONDS,
        "method" => method.to_string(),
        "outcome" => outcome.to_string()
    )
    .record(seconds);
}
