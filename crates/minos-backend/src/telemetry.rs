//! Production-grade Prometheus metrics for the backend.
//!
//! All metrics are prefixed `minos_backend_*` and exposed via the
//! `/metrics` HTTP endpoint. The catalog is intentionally exhaustive so
//! operators can answer "what is happening right now" without changing
//! code in production. Adding a new metric is a one-touch operation:
//! describe it once in [`prometheus_handle`], then push values via the
//! typed `record_*` / `set_*` helpers below.
//!
//! # Naming policy
//!
//! - Counters end in `_total`.
//! - Histograms end in `_seconds` (only seconds — no milliseconds).
//! - Gauges have no suffix.
//! - Labels are stable strings, not formatted via `format!`. New label
//!   values must be enumerated in [`metric_label::*`] so cardinality is
//!   bounded.
//!
//! # Outcome convention
//!
//! Every business-flow counter carries an `outcome` label whose values
//! come from [`OUTCOME_OK`] / [`OUTCOME_ERROR`] / [`OUTCOME_UNAUTHORIZED`]
//! etc. Free-form strings are forbidden so dashboards can rely on a
//! known cardinality.

use std::sync::OnceLock;
use std::time::Instant;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

// ── metric names (ALL prefixed minos_backend_) ────────────────────────

const SESSION_REGISTRY_SIZE: &str = "minos_backend_session_registry_size";
const WS_ACTIVE_SESSIONS: &str = "minos_backend_ws_active_sessions";
const WS_CONNECT_TOTAL: &str = "minos_backend_ws_connect_total";
const WS_CLOSE_TOTAL: &str = "minos_backend_ws_close_total";
const WS_OUTBOX_DEPTH: &str = "minos_backend_ws_outbox_depth";
const ENVELOPE_IN_TOTAL: &str = "minos_backend_envelope_in_total";
const ENVELOPE_OUT_TOTAL: &str = "minos_backend_envelope_out_total";
const INGEST_EVENTS_TOTAL: &str = "minos_backend_ingest_events_total";
const INGEST_OUTBOX_DROPPED_TOTAL: &str = "minos_backend_ingest_outbox_dropped_total";
const HTTP_REQUEST_DURATION_SECONDS: &str = "minos_backend_http_request_duration_seconds";
const FORWARD_RPC_LATENCY_SECONDS: &str = "minos_backend_forward_rpc_latency_seconds";
const AUTH_REGISTER_TOTAL: &str = "minos_backend_auth_register_total";
const AUTH_LOGIN_TOTAL: &str = "minos_backend_auth_login_total";
const AUTH_REFRESH_TOTAL: &str = "minos_backend_auth_refresh_total";
const AUTH_LOGOUT_TOTAL: &str = "minos_backend_auth_logout_total";
const AUTH_REFRESH_REUSE_TOTAL: &str = "minos_backend_auth_refresh_reuse_total";
const PAIRING_TOKEN_ISSUE_TOTAL: &str = "minos_backend_pairing_token_issue_total";
const PAIRING_CONSUME_TOTAL: &str = "minos_backend_pairing_consume_total";
const PAIRING_FORGET_TOTAL: &str = "minos_backend_pairing_forget_total";
const PENDING_APPROVALS_COUNT: &str = "minos_backend_pending_approvals_count";
const APPROVAL_DECISION_TOTAL: &str = "minos_backend_approval_decision_total";
const DB_QUERY_DURATION_SECONDS: &str = "minos_backend_db_query_duration_seconds";

// ── outcome label values (string constants for stable cardinality) ────

/// Successful business outcome (counter label `outcome="ok"`).
pub const OUTCOME_OK: &str = "ok";
/// Internal / unexpected error.
pub const OUTCOME_ERROR: &str = "error";
/// Authentication / authorization failure.
pub const OUTCOME_UNAUTHORIZED: &str = "unauthorized";
/// User input validation failure.
pub const OUTCOME_INVALID: &str = "invalid";
/// Conflict — uniqueness violation, state mismatch.
pub const OUTCOME_CONFLICT: &str = "conflict";
/// Rate limited.
pub const OUTCOME_RATE_LIMITED: &str = "rate_limited";
/// Forwarded RPC timed out before peer answered.
pub const OUTCOME_TIMEOUT: &str = "timeout";
/// Peer outbox was full or peer not connected.
pub const OUTCOME_PEER_OFFLINE: &str = "peer_offline";
/// Peer outbox was full (separate from offline so dashboards can split).
pub const OUTCOME_PEER_BACKPRESSURE: &str = "peer_backpressure";

// ── envelope kind labels ──────────────────────────────────────────────

pub const KIND_FORWARD: &str = "forward";
pub const KIND_FORWARDED: &str = "forwarded";
pub const KIND_EVENT: &str = "event";
pub const KIND_INGEST: &str = "ingest";

/// Singleton Prometheus handle. Initialised on first read; describes
/// every metric exactly once so the rendered output carries `# HELP` /
/// `# TYPE` lines.
fn prometheus_handle() -> &'static PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::set_global_recorder(recorder)
            .expect("prometheus recorder should install exactly once per process");

        // Gauges
        metrics::describe_gauge!(
            SESSION_REGISTRY_SIZE,
            "Current number of live websocket sessions in the in-process registry."
        );
        metrics::describe_gauge!(
            WS_ACTIVE_SESSIONS,
            "Currently live websocket sessions, partitioned by device role."
        );
        metrics::describe_gauge!(
            PENDING_APPROVALS_COUNT,
            "Pending (unresolved) approval requests awaiting user decision."
        );

        // Counters
        metrics::describe_counter!(
            WS_CONNECT_TOTAL,
            "Websocket upgrade attempts, labeled by role and outcome."
        );
        metrics::describe_counter!(
            WS_CLOSE_TOTAL,
            "Websocket close events, labeled by role and close-code reason."
        );
        metrics::describe_counter!(
            ENVELOPE_IN_TOTAL,
            "Envelopes received from clients, labeled by kind and protocol version."
        );
        metrics::describe_counter!(
            ENVELOPE_OUT_TOTAL,
            "Envelopes sent to clients, labeled by kind and protocol version."
        );
        metrics::describe_counter!(
            INGEST_EVENTS_TOTAL,
            "Raw ingest envelopes processed, labeled by agent and outcome."
        );
        metrics::describe_counter!(
            INGEST_OUTBOX_DROPPED_TOTAL,
            "Number of ingest fan-out frames dropped because the peer outbox rejected them."
        );
        metrics::describe_counter!(
            AUTH_REGISTER_TOTAL,
            "Account registration attempts, labeled by outcome."
        );
        metrics::describe_counter!(AUTH_LOGIN_TOTAL, "Login attempts, labeled by outcome.");
        metrics::describe_counter!(
            AUTH_REFRESH_TOTAL,
            "Refresh-token rotations, labeled by outcome."
        );
        metrics::describe_counter!(AUTH_LOGOUT_TOTAL, "Logout calls, labeled by outcome.");
        metrics::describe_counter!(
            AUTH_REFRESH_REUSE_TOTAL,
            "Detected refresh-token reuse incidents (security alert)."
        );
        metrics::describe_counter!(
            PAIRING_TOKEN_ISSUE_TOTAL,
            "Pairing token mints, labeled by outcome."
        );
        metrics::describe_counter!(
            PAIRING_CONSUME_TOTAL,
            "Pairing token consumes, labeled by outcome."
        );
        metrics::describe_counter!(
            PAIRING_FORGET_TOTAL,
            "Pairing tear-downs, labeled by outcome."
        );
        metrics::describe_counter!(
            APPROVAL_DECISION_TOTAL,
            "Approval-decision dispatches, labeled by outcome."
        );

        // Histograms
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
        metrics::describe_histogram!(
            DB_QUERY_DURATION_SECONDS,
            metrics::Unit::Seconds,
            "Repository operation duration in seconds, labeled by repo and op."
        );
        metrics::describe_histogram!(
            WS_OUTBOX_DEPTH,
            "Per-frame depth of websocket outbox at enqueue time, by role."
        );

        // Seed gauges so /metrics returns them on a fresh process.
        metrics::gauge!(SESSION_REGISTRY_SIZE).set(0.0);
        metrics::gauge!(PENDING_APPROVALS_COUNT).set(0.0);

        handle
    })
}

/// Idempotent initialiser. The first call to any `record_*`/`set_*`
/// helper installs the recorder; this helper exists so `main.rs` can
/// pre-warm during boot rather than on first request.
pub fn init() {
    let _ = prometheus_handle();
}

/// Render the current metric snapshot in the Prometheus 0.0.4 text
/// format. Cheap to call per scrape (held by an `Arc` internally).
#[must_use]
pub fn render() -> String {
    prometheus_handle().render()
}

// ── gauges ────────────────────────────────────────────────────────────

#[allow(clippy::cast_precision_loss)]
pub fn set_session_registry_size(size: usize) {
    init();
    metrics::gauge!(SESSION_REGISTRY_SIZE).set(size as f64);
}

#[allow(clippy::cast_precision_loss)]
pub fn set_pending_approvals_count(count: usize) {
    init();
    metrics::gauge!(PENDING_APPROVALS_COUNT).set(count as f64);
}

pub fn record_session_role_open(role: &str) {
    init();
    metrics::gauge!(WS_ACTIVE_SESSIONS, "role" => role.to_string()).increment(1.0);
}

pub fn record_session_role_close(role: &str) {
    init();
    metrics::gauge!(WS_ACTIVE_SESSIONS, "role" => role.to_string()).decrement(1.0);
}

// ── counters ──────────────────────────────────────────────────────────

pub fn record_ws_connect(role: &str, outcome: &str) {
    init();
    metrics::counter!(
        WS_CONNECT_TOTAL,
        "role" => role.to_string(),
        "outcome" => outcome.to_string(),
    )
    .increment(1);
}

/// Record a websocket close. `code_reason` should be a stable human
/// label (e.g. `"heartbeat_timeout"`, `"session_superseded"`,
/// `"auth_revoked"`, `"client_close"`) — not the numeric close code,
/// which is unbounded.
pub fn record_ws_close(role: &str, code_reason: &str) {
    init();
    metrics::counter!(
        WS_CLOSE_TOTAL,
        "role" => role.to_string(),
        "reason" => code_reason.to_string(),
    )
    .increment(1);
}

pub fn record_envelope_in(kind: &str, version: u8) {
    init();
    metrics::counter!(
        ENVELOPE_IN_TOTAL,
        "kind" => kind.to_string(),
        "version" => version.to_string(),
    )
    .increment(1);
}

pub fn record_envelope_out(kind: &str, version: u8) {
    init();
    metrics::counter!(
        ENVELOPE_OUT_TOTAL,
        "kind" => kind.to_string(),
        "version" => version.to_string(),
    )
    .increment(1);
}

pub fn record_ingest_event(agent: &str, outcome: &str) {
    init();
    metrics::counter!(
        INGEST_EVENTS_TOTAL,
        "agent" => agent.to_string(),
        "outcome" => outcome.to_string(),
    )
    .increment(1);
}

pub fn increment_ingest_outbox_dropped() {
    init();
    metrics::counter!(INGEST_OUTBOX_DROPPED_TOTAL).increment(1);
}

pub fn record_auth_register(outcome: &str) {
    init();
    metrics::counter!(AUTH_REGISTER_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

pub fn record_auth_login(outcome: &str) {
    init();
    metrics::counter!(AUTH_LOGIN_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

pub fn record_auth_refresh(outcome: &str) {
    init();
    metrics::counter!(AUTH_REFRESH_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

pub fn record_auth_logout(outcome: &str) {
    init();
    metrics::counter!(AUTH_LOGOUT_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

pub fn record_auth_refresh_reuse() {
    init();
    metrics::counter!(AUTH_REFRESH_REUSE_TOTAL).increment(1);
}

pub fn record_pairing_token_issue(outcome: &str) {
    init();
    metrics::counter!(PAIRING_TOKEN_ISSUE_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

pub fn record_pairing_consume(outcome: &str) {
    init();
    metrics::counter!(PAIRING_CONSUME_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

pub fn record_pairing_forget(outcome: &str) {
    init();
    metrics::counter!(PAIRING_FORGET_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

pub fn record_approval_decision(outcome: &str) {
    init();
    metrics::counter!(APPROVAL_DECISION_TOTAL, "outcome" => outcome.to_string()).increment(1);
}

// ── histograms ────────────────────────────────────────────────────────

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

pub fn record_db_query(repo: &str, op: &str, seconds: f64) {
    init();
    metrics::histogram!(
        DB_QUERY_DURATION_SECONDS,
        "repo" => repo.to_string(),
        "op" => op.to_string()
    )
    .record(seconds);
}

#[allow(clippy::cast_precision_loss)]
pub fn record_ws_outbox_depth(role: &str, depth: usize) {
    init();
    metrics::histogram!(WS_OUTBOX_DEPTH, "role" => role.to_string()).record(depth as f64);
}

// ── RAII timer ────────────────────────────────────────────────────────

/// Records a histogram observation on `Drop`.
///
/// Intended use is at the start of a fallible operation; the observation
/// fires regardless of whether the function returns `Ok` or `Err`.
/// Pair with the outcome counters to get both latency and success rate
/// from a single instrumentation point.
///
/// ```ignore
/// let _t = ForwardRpcTimer::new("minos_health");
/// let result = call_host(...).await;
/// telemetry::record_forward_rpc("minos_health", outcome_label(&result), _t.elapsed_secs());
/// ```
///
/// Most callers will prefer the explicit `record_*` helpers; the timer
/// is here mainly so we can layer per-repo instrumentation without
/// scattering `Instant::now()` calls.
#[must_use]
pub struct DbTimer {
    repo: &'static str,
    op: &'static str,
    started_at: Instant,
}

impl DbTimer {
    pub fn new(repo: &'static str, op: &'static str) -> Self {
        Self {
            repo,
            op,
            started_at: Instant::now(),
        }
    }
}

impl Drop for DbTimer {
    fn drop(&mut self) {
        record_db_query(self.repo, self.op, self.started_at.elapsed().as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_after_initial_describes_emits_metric_lines() {
        // Touch a few helpers so the snapshot has something concrete.
        record_envelope_in(KIND_FORWARD, 1);
        record_auth_login(OUTCOME_OK);
        set_session_registry_size(3);

        let body = render();
        assert!(
            body.contains("minos_backend_envelope_in_total"),
            "envelope_in_total must show up in /metrics: {body}"
        );
        assert!(
            body.contains("minos_backend_auth_login_total"),
            "auth_login_total must show up: {body}"
        );
        assert!(
            body.contains("minos_backend_session_registry_size"),
            "session_registry_size must show up: {body}"
        );
    }
}
