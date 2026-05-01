//! Dart-visible frb surface over `minos_mobile::MobileClient`.
//!
//! This file is the entire frb input: `flutter_rust_bridge_codegen` walks this
//! module (and its siblings under `crate::api`) to emit Dart bindings and the
//! matching `wire_*` handlers in `crate::frb_generated`. Anything added here
//! becomes visible from Dart; internal helpers live outside `crate::api`.
//!
//! The opaque wrapper [`MobileClient`] holds the real
//! `minos_mobile::MobileClient` behind a `RustOpaque` handle — Dart never
//! marshals its fields, only invokes methods on it. Domain enums/structs are
//! mirrored (see the `#[frb(mirror(...))]` blocks below) so pattern-matching
//! works on the Dart side without duplicating the localization table.

use std::path::Path;
use std::sync::OnceLock;

use flutter_rust_bridge::frb;
use minos_mobile::log_capture::{LogLevel as CoreLogLevel, LogRecord as CoreLogRecord};
use minos_mobile::request_trace::{
    RequestTraceRecord as CoreRequestTraceRecord, RequestTraceStatus as CoreRequestTraceStatus,
    RequestTransport as CoreRequestTransport,
};
use minos_mobile::UiEventFrame as MobileUiEventFrame;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::watch;

// `StreamSink` is defined by the `frb_generated_boilerplate!` macro expanded
// inside `crate::frb_generated`, not at the flutter_rust_bridge crate root.
// We re-route the name through the generated module so unqualified
// `StreamSink<T>` resolves both pre- and post-codegen.
use crate::frb_generated::StreamSink;

// Re-exported `pub use` so `crate::api::minos::TypeName` resolves for the
// generated wire code in `frb_generated.rs`. Mirror declarations below still
// provide the shape metadata the codegen needs.
pub use minos_domain::{
    AgentDescriptor, AgentName, AgentStatus, ConnectionState, ErrorKind, Lang, MinosError,
    PairingState,
};
pub use minos_protocol::{
    AuthSummary, HostSummary, ListThreadsParams, ListThreadsResponse, ReadThreadParams,
    ReadThreadResponse, StartAgentResponse, ThreadSummary,
};
pub use minos_ui_protocol::{MessageRole, ThreadEndReason, UiEventMessage};

// ───────────────────────────── opaque client ─────────────────────────────

/// Opaque Dart handle around `minos_mobile::MobileClient`.
///
/// The inner type is not exposed to Dart — all interactions go through the
/// `impl` below. This keeps `Arc<dyn MobilePairingStore>` (and any other
/// non-FFI-safe internals) Rust-side.
#[frb(opaque)]
pub struct MobileClient(minos_mobile::MobileClient);

fn frb_runtime() -> &'static Runtime {
    static FRB_RUNTIME: OnceLock<Runtime> = OnceLock::new();
    FRB_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("minos-frb")
            .build()
            .expect("failed to build minos-ffi-frb tokio runtime")
    })
}

fn spawn_state_forwarder<F>(mut rx: watch::Receiver<ConnectionState>, mut emit: F)
where
    F: FnMut(ConnectionState) -> Result<(), ()> + Send + 'static,
{
    frb_runtime().spawn(async move {
        // Emit the snapshot visible at subscribe time so late subscribers
        // aren't stuck on whatever they last rendered.
        if emit(*rx.borrow_and_update()).is_err() {
            return;
        }
        while rx.changed().await.is_ok() {
            if emit(*rx.borrow()).is_err() {
                break;
            }
        }
    });
}

/// Dart-visible shape of `minos_mobile::UiEventFrame`. Held as a separate
/// type (rather than mirrored) so the `ui` field lands as the mirrored
/// `UiEventMessage` variant on the Dart side.
pub struct UiEventFrame {
    pub thread_id: String,
    pub seq: u64,
    pub ui: UiEventMessage,
    pub ts_ms: i64,
}

/// Durable mobile pairing snapshot mirrored into the iOS keychain.
///
/// Phase 4 added the five auth fields (access/refresh tokens + bound
/// account identity) so the Dart-side secure store can rehydrate the full
/// session on cold launch. All five auth fields are persisted as a tuple —
/// either every one is present or all are `None`.
///
/// ADR-0020 dropped the device_secret from this snapshot — the iOS rail
/// is bearer-only.
///
/// Backend URL and CF Access service-token headers were dropped from the
/// snapshot when pairing transitioned to compile-time `build_config` — the
/// transport-edge values never round-trip through durable storage now.
pub struct PersistedPairingState {
    pub device_id: Option<String>,
    pub access_token: Option<String>,
    pub access_expires_at_ms: Option<i64>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
}

impl From<minos_mobile::PersistedPairingState> for PersistedPairingState {
    fn from(state: minos_mobile::PersistedPairingState) -> Self {
        Self {
            device_id: state.device_id,
            access_token: state.access_token,
            access_expires_at_ms: state.access_expires_at_ms,
            refresh_token: state.refresh_token,
            account_id: state.account_id,
            account_email: state.account_email,
        }
    }
}

impl From<PersistedPairingState> for minos_mobile::PersistedPairingState {
    fn from(state: PersistedPairingState) -> Self {
        Self {
            device_id: state.device_id,
            access_token: state.access_token,
            access_expires_at_ms: state.access_expires_at_ms,
            refresh_token: state.refresh_token,
            account_id: state.account_id,
            account_email: state.account_email,
        }
    }
}

/// Dart-visible mirror of `minos_protocol::HostSummary`. One row in
/// `/v1/me/hosts`.
pub struct HostSummaryDto {
    pub host_device_id: String,
    pub host_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: String,
}

impl From<HostSummary> for HostSummaryDto {
    fn from(s: HostSummary) -> Self {
        Self {
            host_device_id: s.host_device_id.to_string(),
            host_display_name: s.host_display_name,
            paired_at_ms: s.paired_at_ms,
            paired_via_device_id: s.paired_via_device_id.to_string(),
        }
    }
}

impl From<MobileUiEventFrame> for UiEventFrame {
    fn from(f: MobileUiEventFrame) -> Self {
        Self {
            thread_id: f.thread_id,
            seq: f.seq,
            ui: f.ui,
            ts_ms: f.ts_ms,
        }
    }
}

/// Dart-visible auth state frame.
///
/// Defined fresh here rather than mirrored from `minos_mobile::auth` because
/// the inner `RefreshFailed` payload is `Arc<MinosError>` for cheap watch-
/// channel cloning — frb's `#[frb(mirror)]` codegen would have to round-trip
/// the Arc, which is awkward. The `From` impl below unwraps the Arc and
/// clones the inner `MinosError` (cheap, since `MinosError` derives `Clone`)
/// so the Dart side sees a plain typed-error variant.
#[derive(Debug, Clone)]
pub enum AuthStateFrame {
    Unauthenticated,
    Authenticated { account: AuthSummary },
    Refreshing,
    RefreshFailed { error: MinosError },
}

impl From<minos_mobile::auth::AuthStateFrame> for AuthStateFrame {
    fn from(f: minos_mobile::auth::AuthStateFrame) -> Self {
        use minos_mobile::auth::AuthStateFrame as M;
        match f {
            M::Unauthenticated => Self::Unauthenticated,
            M::Authenticated { account } => Self::Authenticated { account },
            M::Refreshing => Self::Refreshing,
            M::RefreshFailed { error } => Self::RefreshFailed {
                error: (*error).clone(),
            },
        }
    }
}

impl MobileClient {
    /// Construct a client backed by the built-in in-memory pairing store.
    /// Synchronous — no I/O happens until a pairing method is called.
    #[frb(sync)]
    #[must_use]
    pub fn new(self_name: String) -> Self {
        Self(minos_mobile::MobileClient::new_with_in_memory_store(
            self_name,
        ))
    }

    /// Construct a client preloaded with a durable pairing snapshot from the
    /// Dart-side secure store.
    #[frb(sync)]
    #[must_use]
    pub fn new_with_persisted_state(self_name: String, state: PersistedPairingState) -> Self {
        Self(minos_mobile::MobileClient::new_with_persisted_state(
            self_name,
            state.into(),
        ))
    }

    /// Pair using the raw JSON payload extracted from the scanned QR v2
    /// code. Delegates to `MobileClient::pair_with_qr_json`.
    pub async fn pair_with_qr_json(&self, qr_json: String) -> Result<(), MinosError> {
        self.0.pair_with_qr_json(qr_json).await
    }

    /// Reconnect using the durable pairing snapshot already loaded from the
    /// Dart-side secure store.
    pub async fn resume_persisted_session(&self) -> Result<(), MinosError> {
        self.0.resume_persisted_session().await
    }

    /// Forget a specific paired Mac. The path-bound `host_device_id` is
    /// the Mac to forget. Idempotent. ADR-0020 supersedes the old
    /// `forget_peer` (single-peer) call.
    pub async fn forget_host(&self, host_device_id: String) -> Result<(), MinosError> {
        let host = parse_device_id(&host_device_id)?;
        self.0.forget_host(host).await
    }

    /// List every Mac paired to the caller's account.
    pub async fn list_paired_hosts(&self) -> Result<Vec<HostSummaryDto>, MinosError> {
        let hosts = self.0.list_paired_hosts().await?;
        Ok(hosts.into_iter().map(HostSummaryDto::from).collect())
    }

    /// Override the active Mac the next forward-RPC routes to.
    pub async fn set_active_host(&self, host_device_id: String) -> Result<(), MinosError> {
        let host = parse_device_id(&host_device_id)?;
        self.0.set_active_host(host).await
    }

    /// Read the current active Mac id, or `None` if no pair has been
    /// completed yet.
    pub async fn active_host(&self) -> Result<Option<String>, MinosError> {
        Ok(self.0.active_host().await?.map(|id| id.to_string()))
    }

    /// Request a page of thread summaries.
    pub async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        self.0.list_threads(req).await
    }

    /// Read a window of translated UI events for one thread.
    pub async fn read_thread(
        &self,
        req: ReadThreadParams,
    ) -> Result<ReadThreadResponse, MinosError> {
        self.0.read_thread(req).await
    }

    /// Export the current pairing snapshot so Dart can mirror it into secure
    /// storage after pairing succeeds.
    pub async fn persisted_pairing_state(&self) -> Result<PersistedPairingState, MinosError> {
        self.0
            .persisted_pairing_state()
            .await
            .map(PersistedPairingState::from)
    }

    /// Current connection state, read from the watch-channel cache. Cheap and
    /// synchronous.
    #[frb(sync)]
    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        self.0.current_state()
    }

    /// Subscribe to connection-state transitions. Emits the current value
    /// immediately, then every subsequent change. The spawned task exits once
    /// the Dart side drops the stream (detected via `sink.add(...).is_err()`).
    pub fn subscribe_state(&self, sink: StreamSink<ConnectionState>) {
        spawn_state_forwarder(self.0.events_stream(), move |state| {
            sink.add(state).map_err(|_| ())
        });
    }

    /// Subscribe to live `UiEventFrame`s fanned out from the backend.
    /// Every frb stream sink gets its own broadcast receiver; lagging
    /// subscribers lose old frames rather than blocking the producer.
    pub fn subscribe_ui_events(&self, sink: StreamSink<UiEventFrame>) {
        let mut rx = self.0.ui_events_stream();
        frb_runtime().spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(frame) => {
                        if sink.add(UiEventFrame::from(frame)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "ui_events_stream lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // ─────────────────────────── account auth ──────────────────────────────

    /// Register a new account on the backend. On success the bearer +
    /// refresh tokens are held in memory and surfaced via the auth-state
    /// stream; the reconnect loop then drives the WS back to `Connected`.
    pub async fn register(
        &self,
        email: String,
        password: String,
    ) -> Result<AuthSummary, MinosError> {
        self.0.register(email, password).await
    }

    /// Log into an existing account on the backend. Same shape as
    /// `register` modulo the create-vs-find behaviour on the server.
    pub async fn login(&self, email: String, password: String) -> Result<AuthSummary, MinosError> {
        self.0.login(email, password).await
    }

    /// Rotate the bearer + refresh tokens. Surfaces `Refreshing` /
    /// `Authenticated` / `RefreshFailed` transitions on the auth-state
    /// stream.
    pub async fn refresh_session(&self) -> Result<(), MinosError> {
        self.0.refresh_session().await
    }

    /// Log out of the current session. Best-effort `stop_agent`, then
    /// revoke the refresh token server-side, then wipe local state.
    pub async fn logout(&self) -> Result<(), MinosError> {
        self.0.logout().await
    }

    // ─────────────────────────── agent dispatch ────────────────────────────

    /// Detect the CLI agents available on the paired runtime.
    pub async fn list_clis(&self) -> Result<Vec<AgentDescriptor>, MinosError> {
        self.0.list_clis().await
    }

    /// Start a new agent session and return the daemon-issued `session_id`
    /// (a.k.a. `thread_id`) plus the resolved workspace path. The caller is
    /// responsible for sending the first user message separately.
    pub async fn start_agent(
        &self,
        agent: AgentName,
        prompt: String,
    ) -> Result<StartAgentResponse, MinosError> {
        self.0.start_agent(agent, prompt).await
    }

    /// Send a follow-up user message to an existing agent session.
    pub async fn send_user_message(
        &self,
        session_id: String,
        text: String,
    ) -> Result<(), MinosError> {
        self.0.send_user_message(session_id, text).await
    }

    /// Pause an in-flight turn on the given thread. Best-effort. The thread
    /// transitions to `Suspended { UserInterrupt }` regardless of whether the
    /// codex side acknowledges in time.
    pub async fn interrupt_thread(&self, thread_id: String) -> Result<(), MinosError> {
        self.0.interrupt_thread(thread_id).await
    }

    /// Permanently close the given thread. Idempotent.
    pub async fn close_thread(&self, thread_id: String) -> Result<(), MinosError> {
        self.0.close_thread(thread_id).await
    }

    // ─────────────────────────── lifecycle hooks ───────────────────────────

    /// Mark the app as foregrounded. Resets the reconnect backoff so the
    /// next connect attempt happens promptly.
    #[frb(sync)]
    pub fn notify_foregrounded(&self) {
        self.0.notify_foregrounded();
    }

    /// Mark the app as backgrounded. Pauses the reconnect loop so we
    /// don't poke the backend while the OS is freezing the process.
    #[frb(sync)]
    pub fn notify_backgrounded(&self) {
        self.0.notify_backgrounded();
    }

    // ─────────────────────────── auth subscription ─────────────────────────

    /// Subscribe to auth-state transitions. Emits the current cached frame
    /// immediately, then every subsequent change. The spawned task exits
    /// once Dart drops the stream (detected via `sink.add(...).is_err()`).
    pub fn subscribe_auth_state(&self, sink: StreamSink<AuthStateFrame>) {
        let mut rx = self.0.subscribe_auth_state();
        frb_runtime().spawn(async move {
            // Emit the snapshot visible at subscribe time so late subscribers
            // aren't stuck on whatever they last rendered.
            let snapshot = AuthStateFrame::from(rx.borrow_and_update().clone());
            if sink.add(snapshot).is_err() {
                return;
            }
            while rx.changed().await.is_ok() {
                let frame = AuthStateFrame::from(rx.borrow().clone());
                if sink.add(frame).is_err() {
                    break;
                }
            }
        });
    }
}

/// Parse a UUID-shaped device id string emitted from Dart back into a
/// `minos_domain::DeviceId`. Surfaces `MinosError::StoreCorrupt` on
/// malformed input — the Dart side is expected to round-trip the same
/// strings it received from `HostSummaryDto.host_device_id`, so this is a
/// best-effort guard rather than a user-facing error path.
fn parse_device_id(s: &str) -> Result<minos_domain::DeviceId, MinosError> {
    uuid::Uuid::parse_str(s)
        .map(minos_domain::DeviceId)
        .map_err(|e| MinosError::StoreCorrupt {
            path: "device_id".into(),
            message: format!("invalid uuid '{s}': {e}"),
        })
}

// ────────────────────────────── free functions ──────────────────────────────

/// Initialize mobile-side Rust logging with the given directory (supplied by
/// Dart, typically `<Documents>/Minos/Logs`). Idempotent — safe to call once
/// per launch.
pub fn init_logging(log_dir: String) -> Result<(), MinosError> {
    minos_mobile::logging::init(Path::new(&log_dir))
}

/// Localize an `ErrorKind` into user-facing copy. Mirrors the UniFFI adapter's
/// `kind_message` so Dart can render localized error strings without hard-
/// coding them.
#[frb(sync)]
#[must_use]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}

// ───────────────────────────── log capture surface ─────────────────────────────

/// Severity tag mirrored from `minos_mobile::log_capture::LogLevel`.
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<CoreLogLevel> for LogLevel {
    fn from(level: CoreLogLevel) -> Self {
        match level {
            CoreLogLevel::Trace => Self::Trace,
            CoreLogLevel::Debug => Self::Debug,
            CoreLogLevel::Info => Self::Info,
            CoreLogLevel::Warn => Self::Warn,
            CoreLogLevel::Error => Self::Error,
        }
    }
}

/// Single tracing event captured by the in-process ring buffer.
pub struct LogRecord {
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub ts_ms: i64,
}

impl From<CoreLogRecord> for LogRecord {
    fn from(record: CoreLogRecord) -> Self {
        Self {
            level: record.level.into(),
            target: record.target,
            message: record.message,
            ts_ms: record.ts_ms,
        }
    }
}

/// Snapshot the records currently held in the ring buffer (oldest first).
/// Pair this with [`subscribe_log_records`] when populating a freshly
/// mounted log panel so prior events are not lost.
#[frb(sync)]
#[must_use]
pub fn recent_log_records() -> Vec<LogRecord> {
    minos_mobile::log_capture::recent()
        .into_iter()
        .map(LogRecord::from)
        .collect()
}

/// Subscribe to the live tail. Each subscriber gets its own broadcast
/// receiver; lagging subscribers drop old records (the producer is never
/// blocked). The spawned task exits when the Dart side drops the stream.
pub fn subscribe_log_records(sink: StreamSink<LogRecord>) {
    let mut rx = minos_mobile::log_capture::subscribe();
    frb_runtime().spawn(async move {
        loop {
            match rx.recv().await {
                Ok(record) => {
                    if sink.add(LogRecord::from(record)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Best-effort tail; the Dart side can re-snapshot
                    // recent_log_records() if it cares about the gap.
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

// ───────────────────────── request trace surface ─────────────────────────

pub enum RequestTraceTransport {
    Http,
    Rpc,
}

impl From<CoreRequestTransport> for RequestTraceTransport {
    fn from(value: CoreRequestTransport) -> Self {
        match value {
            CoreRequestTransport::Http => Self::Http,
            CoreRequestTransport::Rpc => Self::Rpc,
        }
    }
}

pub enum RequestTraceStatus {
    Pending,
    Success,
    Failure,
}

impl From<CoreRequestTraceStatus> for RequestTraceStatus {
    fn from(value: CoreRequestTraceStatus) -> Self {
        match value {
            CoreRequestTraceStatus::Pending => Self::Pending,
            CoreRequestTraceStatus::Success => Self::Success,
            CoreRequestTraceStatus::Failure => Self::Failure,
        }
    }
}

pub struct RequestTraceRecord {
    pub id: u64,
    pub transport: RequestTraceTransport,
    pub method: String,
    pub target: String,
    pub thread_id: Option<String>,
    pub request_summary: Option<String>,
    pub response_summary: Option<String>,
    pub error_detail: Option<String>,
    pub status: RequestTraceStatus,
    pub status_code: Option<u16>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub duration_ms: Option<u32>,
}

impl From<CoreRequestTraceRecord> for RequestTraceRecord {
    fn from(record: CoreRequestTraceRecord) -> Self {
        Self {
            id: record.id,
            transport: record.transport.into(),
            method: record.method,
            target: record.target,
            thread_id: record.thread_id,
            request_summary: record.request_summary,
            response_summary: record.response_summary,
            error_detail: record.error_detail,
            status: record.status.into(),
            status_code: record.status_code,
            started_at_ms: record.started_at_ms,
            completed_at_ms: record.completed_at_ms,
            duration_ms: record.duration_ms,
        }
    }
}

#[frb(sync)]
#[must_use]
pub fn recent_request_traces() -> Vec<RequestTraceRecord> {
    minos_mobile::request_trace::recent()
        .into_iter()
        .map(RequestTraceRecord::from)
        .collect()
}

#[frb(sync)]
pub fn clear_request_traces() {
    minos_mobile::request_trace::clear();
}

pub fn subscribe_request_traces(sink: StreamSink<RequestTraceRecord>) {
    let mut rx = minos_mobile::request_trace::subscribe();
    frb_runtime().spawn(async move {
        loop {
            match rx.recv().await {
                Ok(record) => {
                    if sink.add(RequestTraceRecord::from(record)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

// ─────────────────────────── mirrored domain types ───────────────────────────
//
// frb requires us to re-declare any foreign type we want to expose to Dart.
// The `#[frb(mirror(T))]` attribute tells the codegen "this declaration is the
// shape of `T` from `crate::domain`; emit Dart bindings that encode/decode
// the real `T`". The mirror declarations themselves are never instantiated;
// they exist purely as codegen hints.

#[allow(dead_code)]
#[frb(mirror(ConnectionState))]
pub enum _ConnectionState {
    Disconnected,
    Pairing,
    Connected,
    Reconnecting { attempt: u32 },
}

#[allow(dead_code)]
#[frb(mirror(PairingState))]
pub enum _PairingState {
    Unpaired,
    AwaitingPeer,
    Paired,
}

#[allow(dead_code)]
#[frb(mirror(Lang))]
pub enum _Lang {
    Zh,
    En,
}

#[allow(dead_code)]
#[frb(mirror(AgentName))]
pub enum _AgentName {
    Codex,
    Claude,
    Gemini,
}

#[allow(dead_code)]
#[frb(mirror(AgentStatus))]
pub enum _AgentStatus {
    Ok,
    Missing,
    Error { reason: String },
}

#[allow(dead_code)]
#[frb(mirror(AgentDescriptor))]
pub struct _AgentDescriptor {
    pub name: AgentName,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: AgentStatus,
}

#[allow(dead_code)]
#[frb(mirror(ErrorKind))]
pub enum _ErrorKind {
    BindFailed,
    ConnectFailed,
    Disconnected,
    PairingTokenInvalid,
    PairingStateMismatch,
    DeviceNotTrusted,
    StoreIo,
    StoreCorrupt,
    CliProbeTimeout,
    CliProbeFailed,
    RpcCallFailed,
    Unauthorized,
    ConnectionStateMismatch,
    EnvelopeVersionUnsupported,
    PeerOffline,
    BackendInternal,
    CfAuthFailed,
    CodexSpawnFailed,
    CodexConnectFailed,
    CodexProtocolError,
    AgentAlreadyRunning,
    AgentNotRunning,
    AgentNotSupported,
    AgentSessionIdMismatch,
    CfAccessMisconfigured,
    IngestSeqConflict,
    ThreadNotFound,
    TranslationNotImplemented,
    TranslationFailed,
    PairingQrVersionUnsupported,
    Timeout,
    NotConnected,
    RequestDropped,
    AuthRefreshFailed,
    EmailTaken,
    WeakPassword,
    RateLimited,
    InvalidCredentials,
    AgentStartFailed,
    PairingTokenExpired,
}

#[allow(dead_code)]
#[frb(mirror(MinosError))]
pub enum _MinosError {
    BindFailed { addr: String, message: String },
    ConnectFailed { url: String, message: String },
    Disconnected { reason: String },
    PairingTokenInvalid,
    PairingStateMismatch { actual: PairingState },
    DeviceNotTrusted { device_id: String },
    StoreIo { path: String, message: String },
    StoreCorrupt { path: String, message: String },
    CliProbeTimeout { bin: String, timeout_ms: u64 },
    CliProbeFailed { bin: String, message: String },
    RpcCallFailed { method: String, message: String },
    Unauthorized { reason: String },
    ConnectionStateMismatch { expected: String, actual: String },
    EnvelopeVersionUnsupported { version: u8 },
    PeerOffline { peer_device_id: String },
    BackendInternal { message: String },
    CfAuthFailed { message: String },
    CodexSpawnFailed { message: String },
    CodexConnectFailed { url: String, message: String },
    CodexProtocolError { method: String, message: String },
    AgentAlreadyRunning,
    AgentNotRunning,
    AgentNotSupported { agent: AgentName },
    AgentSessionIdMismatch,
    CfAccessMisconfigured { reason: String },
    IngestSeqConflict { thread_id: String, seq: u64 },
    ThreadNotFound { thread_id: String },
    TranslationNotImplemented { agent: AgentName },
    TranslationFailed { agent: AgentName, message: String },
    PairingQrVersionUnsupported { version: u8 },
    Timeout,
    NotConnected,
    RequestDropped,
    AuthRefreshFailed { message: String },
    EmailTaken,
    WeakPassword,
    RateLimited { retry_after_s: u32 },
    InvalidCredentials,
    AgentStartFailed { reason: String },
    PairingTokenExpired,
}

// ─────────────────────────── mirrored protocol types ──────────────────────────

#[allow(dead_code)]
#[frb(mirror(ListThreadsParams))]
pub struct _ListThreadsParams {
    pub limit: u32,
    pub before_ts_ms: Option<i64>,
    pub agent: Option<AgentName>,
}

#[allow(dead_code)]
#[frb(mirror(ListThreadsResponse))]
pub struct _ListThreadsResponse {
    pub threads: Vec<ThreadSummary>,
    pub next_before_ts_ms: Option<i64>,
}

#[allow(dead_code)]
#[frb(mirror(ReadThreadParams))]
pub struct _ReadThreadParams {
    pub thread_id: String,
    pub from_seq: Option<u64>,
    pub limit: u32,
}

#[allow(dead_code)]
#[frb(mirror(ReadThreadResponse))]
pub struct _ReadThreadResponse {
    pub ui_events: Vec<UiEventMessage>,
    pub next_seq: Option<u64>,
    pub thread_end_reason: Option<ThreadEndReason>,
}

#[allow(dead_code)]
#[frb(mirror(ThreadSummary))]
pub struct _ThreadSummary {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<ThreadEndReason>,
}

#[allow(dead_code)]
#[frb(mirror(MessageRole))]
pub enum _MessageRole {
    User,
    Assistant,
    System,
}

#[allow(dead_code)]
#[frb(mirror(ThreadEndReason))]
pub enum _ThreadEndReason {
    UserStopped,
    AgentDone,
    Crashed { message: String },
    Timeout,
    HostDisconnected,
}

#[allow(dead_code)]
#[frb(mirror(UiEventMessage))]
pub enum _UiEventMessage {
    ThreadOpened {
        thread_id: String,
        agent: AgentName,
        title: Option<String>,
        opened_at_ms: i64,
    },
    ThreadTitleUpdated {
        thread_id: String,
        title: String,
    },
    ThreadClosed {
        thread_id: String,
        reason: ThreadEndReason,
        closed_at_ms: i64,
    },
    MessageStarted {
        message_id: String,
        role: MessageRole,
        started_at_ms: i64,
    },
    MessageCompleted {
        message_id: String,
        finished_at_ms: i64,
    },
    TextDelta {
        message_id: String,
        text: String,
    },
    ReasoningDelta {
        message_id: String,
        text: String,
    },
    ToolCallPlaced {
        message_id: String,
        tool_call_id: String,
        name: String,
        args_json: String,
    },
    ToolCallCompleted {
        tool_call_id: String,
        output: String,
        is_error: bool,
    },
    Error {
        code: String,
        message: String,
        message_id: Option<String>,
    },
    Raw {
        kind: String,
        payload_json: String,
    },
}

// ─────────────────────── mirrored auth + agent types ─────────────────────────

#[allow(dead_code)]
#[frb(mirror(AuthSummary))]
pub struct _AuthSummary {
    pub account_id: String,
    pub email: String,
}

#[allow(dead_code)]
#[frb(mirror(StartAgentResponse))]
pub struct _StartAgentResponse {
    pub session_id: String,
    pub cwd: String,
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn state_forwarder_spawns_without_current_runtime() {
        let (tx, rx) = watch::channel(ConnectionState::Disconnected);

        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "test must start outside a tokio runtime"
        );

        let (state_tx, state_rx) = mpsc::channel();
        spawn_state_forwarder(rx, move |state| state_tx.send(state).map_err(|_| ()));

        assert_eq!(
            state_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            ConnectionState::Disconnected
        );

        tx.send(ConnectionState::Pairing).unwrap();
        assert_eq!(
            state_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            ConnectionState::Pairing
        );
    }
}
