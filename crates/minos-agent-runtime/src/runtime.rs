//! `AgentRuntime` — the state-machine façade glue between `CodexProcess`,
//! `CodexClient`, and the outbound `RawIngest` / `AgentState` streams.
//!
//! Spec §5.1 drives the surface; §6.1 / §6.3 / §6.4 drive the sequencing.
//!
//! Invariants preserved from Phase B and earlier:
//! - `start()` returns [`MinosError::AgentAlreadyRunning`] when state ≠ `Idle`.
//! - `stop()` is idempotent: `Idle | Crashed` short-circuit to `Ok(())`.
//! - `send_user_message()` distinguishes `AgentNotRunning` (no session at
//!   all) from `AgentSessionIdMismatch` (session id drift).
//! - Unexpected ServerRequest methods are warn-logged and forwarded as a
//!   `RawIngest` carrying a synthetic method name (`server_request/<name>`)
//!   but NOT replied to.
//! - Broadcast capacity defaults to 256; lagged subscribers log warnings but
//!   are not disconnected.
//!
//! ## Concurrency model
//!
//! - A single `supervisor_task` owns the `tokio::process::Child`. It waits
//!   in a `tokio::select!` between `child.wait()` and a `oneshot::Receiver`
//!   driven by `stop()`. The supervisor is the only code that drives the
//!   `state_tx` on process exit — guaranteeing we don't race between
//!   expected and unexpected terminations.
//! - A single `event_pump_task` owns the `CodexClient` (move-consumed from
//!   the active session) and reads `next_inbound()` in a loop. It forwards
//!   every notification verbatim to the `ingest_tx` broadcast as
//!   `RawIngest { agent, thread_id, payload, ts_ms }`; the backend's
//!   ingest handler translates on write (plan §B6). It handles approvals +
//!   unknown server requests by replying through the same client handle.
//! - Public `send_user_message` / `stop` use a separate `CodexClient` handle
//!   (Clone-safe) to issue outbound requests. Clone-safety is achieved via
//!   `Arc<CodexClient>` — every operation goes through the pump task's
//!   mpsc, so concurrent writers don't contend on the WS.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use minos_domain::{AgentName, MinosError};
use serde_json::Value;
use tokio::sync::{broadcast, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use url::Url;

use crate::approvals::{build_auto_reject, APPROVAL_METHODS};
use crate::codex_client::{CodexClient, Inbound};
use crate::process::{reason_from_exit, CodexProcess};
use crate::state::AgentState;

/// One raw native event emitted by an agent CLI, as captured by the
/// `event_pump_task`. The backend's ingest handler (plan §B6) persists
/// these verbatim under `(thread_id, seq)` and runs the per-agent
/// translator on read / live fan-out. Seq is **not** carried here: it is
/// a transport concern assigned by the `Ingestor` (plan §B4).
#[derive(Debug, Clone)]
pub struct RawIngest {
    pub agent: AgentName,
    pub thread_id: String,
    /// The full JSON-RPC notification as a single `Value`, e.g.
    /// `{ "method": "item/agentMessage/delta", "params": {...} }`. The
    /// shape is CLI-specific; translators are the only code that interprets
    /// it.
    pub payload: Value,
    pub ts_ms: i64,
}

fn current_unix_ms() -> i64 {
    use std::time::UNIX_EPOCH;
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Default timeout for the one-shot `initialize` + `thread/start` handshake.
const DEFAULT_HANDSHAKE_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// MCP protocol version sent to `codex app-server` during startup.
const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

/// Fire-and-observe window for `turn/start`. The send itself awaits the
/// response (so we can surface a protocol error synchronously), but we don't
/// wait for `turn/completed` — that flows via the event stream.
const TURN_START_TIMEOUT: Duration = Duration::from_secs(10);

/// Max time to spend on each polite-goodbye call during `stop`. Short by
/// design — the authoritative termination is the signal.
const STOP_POLITE_TIMEOUT: Duration = Duration::from_millis(500);

/// Default broadcast channel capacity. Slow subscribers that fall behind get
/// a `Lagged` error; we log a warning but do not disconnect them.
const DEFAULT_EVENT_BUFFER: usize = 256;

/// Successful `start_agent` outcome — the caller needs the session id to
/// correlate subsequent `send_user_message` calls, and the canonicalised
/// workspace dir for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartAgentOutcome {
    pub session_id: String,
    pub cwd: String,
}

/// Runtime configuration — carries the workspace root, optional explicit
/// binary path, port range, event buffer size, and the snapshot of the
/// user's login-shell env that the daemon captured at bootstrap. The
/// `test_ws_url` seam is gated behind `test-support` and skips subprocess
/// spawn entirely.
#[derive(Debug, Clone)]
pub struct AgentRuntimeConfig {
    pub workspace_root: PathBuf,
    pub codex_bin: Option<PathBuf>,
    pub ws_port_range: std::ops::RangeInclusive<u16>,
    pub event_buffer: usize,
    pub handshake_call_timeout: Duration,
    /// Env snapshot applied with `env_clear` to every spawned codex
    /// subprocess. Defaults to an empty map (caller wiring tested with
    /// the test_ws_url seam, which never spawns).
    pub subprocess_env: Arc<std::collections::HashMap<String, String>>,
    /// Test-only seam: when `Some`, `start()` skips port-probing + codex
    /// spawn + workspace creation and connects directly to this URL.
    /// Production code must leave this as `None`.
    #[cfg(feature = "test-support")]
    pub test_ws_url: Option<Url>,
}

impl AgentRuntimeConfig {
    /// Minimal constructor that fills in sensible defaults for `ws_port_range`
    /// and `event_buffer`. Callers who need custom values set the fields
    /// afterwards.
    #[must_use]
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            codex_bin: None,
            ws_port_range: 7879..=7883,
            event_buffer: DEFAULT_EVENT_BUFFER,
            handshake_call_timeout: DEFAULT_HANDSHAKE_CALL_TIMEOUT,
            subprocess_env: Arc::new(std::collections::HashMap::new()),
            #[cfg(feature = "test-support")]
            test_ws_url: None,
        }
    }
}

/// The agent runtime handle. Share by cloning the `Arc`; all methods take
/// `&self`.
pub struct AgentRuntime {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: AgentRuntimeConfig,
    state_tx: watch::Sender<AgentState>,
    _state_rx_guard: watch::Receiver<AgentState>,
    ingest_tx: broadcast::Sender<RawIngest>,
    active: Mutex<Option<Active>>,
}

/// The live session state — client for outbound calls, the task handles for
/// the supervisor + event pump, and the signal that distinguishes expected
/// from unexpected termination.
struct Active {
    client: Arc<CodexClient>,
    thread_id: String,
    #[allow(dead_code)]
    started_at: SystemTime,
    #[allow(dead_code)]
    agent: AgentName,
    expected_exit: Arc<AtomicBool>,
    stop_signal_tx: Option<oneshot::Sender<()>>,
    supervisor_task: Option<JoinHandle<()>>,
    event_pump_task: Option<JoinHandle<()>>,
}

impl AgentRuntime {
    /// Build a runtime in the `Idle` state. Returns an `Arc` so downstream
    /// observers can share the handle cheaply.
    #[must_use]
    pub fn new(cfg: AgentRuntimeConfig) -> Arc<Self> {
        let (state_tx, state_rx_guard) = watch::channel(AgentState::Idle);
        let ingest_tx = broadcast::Sender::new(cfg.event_buffer.max(1));
        Arc::new(Self {
            inner: Arc::new(Inner {
                cfg,
                state_tx,
                _state_rx_guard: state_rx_guard,
                ingest_tx,
                active: Mutex::new(None),
            }),
        })
    }

    /// Current [`AgentState`] snapshot.
    #[must_use]
    pub fn current_state(&self) -> AgentState {
        self.inner.state_tx.borrow().clone()
    }

    /// A `watch::Receiver` that fires on every state transition. Freshly
    /// created — the caller should treat the initial `borrow_and_update()`
    /// value as the starting state.
    #[must_use]
    pub fn state_stream(&self) -> watch::Receiver<AgentState> {
        self.inner.state_tx.subscribe()
    }

    /// A fresh `broadcast::Receiver<RawIngest>`. Channel capacity is fixed
    /// at `cfg.event_buffer` (default 256); slow subscribers get
    /// `RecvError::Lagged(n)` — the runtime logs a warning once per lag and
    /// does not attempt to reconnect them.
    ///
    /// Each `RawIngest` carries the verbatim JSON-RPC notification the CLI
    /// sent. The backend's ingest dispatcher is the only layer that
    /// translates; downstream subscribers that need a `UiEventMessage` stream
    /// should go through the backend path (plan §B6).
    #[must_use]
    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest> {
        self.inner.ingest_tx.subscribe()
    }

    /// Start a codex session. See spec §5.1 "Start sequence" for the
    /// step-by-step contract.
    pub async fn start(&self, agent: AgentName) -> Result<StartAgentOutcome, MinosError> {
        // Validation: state must be Idle.
        {
            let current = self.inner.state_tx.borrow().clone();
            if !matches!(current, AgentState::Idle | AgentState::Crashed { .. }) {
                return Err(MinosError::AgentAlreadyRunning);
            }
        }

        // MVP only supports Codex.
        if agent != AgentName::Codex {
            return Err(MinosError::AgentNotSupported { agent });
        }

        // Announce "Starting" so observers see the transition even if spawn fails.
        let _ = self.inner.state_tx.send(AgentState::Starting { agent });

        // Execute the start sequence; on any failure roll state back to Idle.
        match self.start_inner(agent).await {
            Ok(outcome) => Ok(outcome),
            Err(e) => {
                let _ = self.inner.state_tx.send(AgentState::Idle);
                Err(e)
            }
        }
    }

    async fn start_inner(&self, agent: AgentName) -> Result<StartAgentOutcome, MinosError> {
        // --- Test seam: skip spawn + port probe, use a pre-bound fake URL ---
        #[cfg(feature = "test-support")]
        if let Some(url) = &self.inner.cfg.test_ws_url {
            return self.start_on_url(agent, url.clone(), None).await;
        }

        // --- Production path: canonicalise workspace_root, spawn codex, connect WS ---
        let workspace_root = ensure_workspace_dir(&self.inner.cfg.workspace_root)?;
        let workspace_display = workspace_root.display().to_string();

        let port = pick_free_port(self.inner.cfg.ws_port_range.clone())?;
        let url =
            Url::parse(&format!("ws://127.0.0.1:{port}")).expect("127.0.0.1:<port> is a valid URL");

        let bin = self
            .inner
            .cfg
            .codex_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(agent.bin_name()));

        let sandbox_arg = format!(
            "sandbox_permissions=['disk-full-read-access','disk-write-folder={workspace_display}']"
        );
        let listen_arg = format!("ws://127.0.0.1:{port}");
        let args: Vec<&str> = vec![
            "app-server",
            "--listen",
            &listen_arg,
            "-c",
            "approval_policy=never",
            "-c",
            &sandbox_arg,
            "-c",
            "shell_environment_policy.inherit=all",
        ];
        let mut process = CodexProcess::spawn(&bin, &args, &self.inner.cfg.subprocess_env)?;
        process.stderr_drain();
        info!(bin = %bin.display(), port, "spawned codex app-server");

        self.start_on_url(agent, url, Some(process)).await
    }

    /// Shared tail of the start sequence: connect WS, initialize + thread/start,
    /// wire up supervisor + pump, commit state to `Running`.
    async fn start_on_url(
        &self,
        agent: AgentName,
        url: Url,
        mut process: Option<CodexProcess>,
    ) -> Result<StartAgentOutcome, MinosError> {
        // Connect WS; retry budget sits inside CodexClient::connect.
        let client = match CodexClient::connect(&url).await {
            Ok(c) => c,
            Err(e) => {
                stop_process_if_present(&mut process).await;
                return Err(e);
            }
        };
        let client = Arc::new(client);
        let handshake_call_timeout = self.inner.cfg.handshake_call_timeout;
        // Handshake: `initialize` → `notifications/initialized` → `thread/start`.
        let init_res = tokio::time::timeout(
            handshake_call_timeout,
            client.call("initialize", initialize_params()),
        )
        .await;
        if let Err(e) = map_timeout(init_res, "initialize", handshake_call_timeout) {
            stop_process_if_present(&mut process).await;
            return Err(e);
        }

        let initialized_res = tokio::time::timeout(
            handshake_call_timeout,
            client.notify("notifications/initialized", serde_json::json!({})),
        )
        .await;
        if let Err(e) = map_timeout_unit(
            initialized_res,
            "notifications/initialized",
            handshake_call_timeout,
        ) {
            stop_process_if_present(&mut process).await;
            return Err(e);
        }

        let cwd = self.cwd_for_session();
        let start_res = tokio::time::timeout(
            handshake_call_timeout,
            client.call("thread/start", serde_json::json!({ "cwd": cwd })),
        )
        .await;
        let start_result = match map_timeout(start_res, "thread/start", handshake_call_timeout) {
            Ok(v) => v,
            Err(e) => {
                stop_process_if_present(&mut process).await;
                return Err(e);
            }
        };
        let thread_id = thread_id_from_response(&start_result)
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "thread/start".into(),
                message: "response missing thread_id/threadId".into(),
            })?
            .to_string();

        // Wire up supervisor + event pump.
        let expected_exit = Arc::new(AtomicBool::new(false));
        let state_tx = self.inner.state_tx.clone();
        let ingest_tx = self.inner.ingest_tx.clone();

        // Two termination signals feed the supervisor:
        // 1. `stop_signal_rx` — `stop()` asks for a graceful teardown.
        // 2. `ws_closed_rx` — the event pump observed `Inbound::Closed`; in
        //    the real-process path this is redundant with `child.wait()`, but
        //    the fake-port path has no child, so the WS close is the only
        //    termination signal available.
        let (stop_signal_tx, stop_signal_rx) = oneshot::channel::<()>();
        let (ws_closed_tx, ws_closed_rx) = oneshot::channel::<()>();
        let supervisor_task = spawn_supervisor(
            process,
            stop_signal_rx,
            ws_closed_rx,
            expected_exit.clone(),
            state_tx.clone(),
        );

        // Event pump: owns the `Arc<CodexClient>` for inbound reads.
        let event_pump_client = Arc::clone(&client);
        let event_pump_task = tokio::spawn(event_pump_loop(
            event_pump_client,
            ingest_tx,
            agent,
            thread_id.clone(),
            Some(ws_closed_tx),
        ));

        let started_at = SystemTime::now();
        let active = Active {
            client: Arc::clone(&client),
            thread_id: thread_id.clone(),
            started_at,
            agent,
            expected_exit,
            stop_signal_tx: Some(stop_signal_tx),
            supervisor_task: Some(supervisor_task),
            event_pump_task: Some(event_pump_task),
        };
        {
            let mut slot = self.inner.active.lock().await;
            *slot = Some(active);
        }

        let _ = self.inner.state_tx.send(AgentState::Running {
            agent,
            thread_id: thread_id.clone(),
            started_at,
        });

        Ok(StartAgentOutcome {
            session_id: thread_id,
            cwd,
        })
    }

    fn cwd_for_session(&self) -> String {
        // Canonicalise if the directory exists; otherwise fall back to display.
        self.inner.cfg.workspace_root.canonicalize().map_or_else(
            |_| self.inner.cfg.workspace_root.display().to_string(),
            |path| path.display().to_string(),
        )
    }

    /// Fire a `turn/start` on the running session. Does NOT await
    /// `turn/completed` — that arrives as a broadcast event.
    pub async fn send_user_message(&self, session_id: &str, text: &str) -> Result<(), MinosError> {
        // Snapshot the state + active; minimise lock-held time.
        let state = self.inner.state_tx.borrow().clone();
        match &state {
            AgentState::Running { thread_id, .. } => {
                if thread_id != session_id {
                    return Err(MinosError::AgentSessionIdMismatch);
                }
            }
            _ => return Err(MinosError::AgentNotRunning),
        }

        let client = {
            let guard = self.inner.active.lock().await;
            match guard.as_ref() {
                Some(a) if a.thread_id == session_id => Arc::clone(&a.client),
                Some(_) => return Err(MinosError::AgentSessionIdMismatch),
                None => return Err(MinosError::AgentNotRunning),
            }
        };

        let params = serde_json::json!({
            "threadId": session_id,
            "input": [{ "type": "text", "text": text }],
        });
        let res = tokio::time::timeout(TURN_START_TIMEOUT, client.call("turn/start", params)).await;
        match res {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => Err(MinosError::CodexProtocolError {
                method: "turn/start".into(),
                message: format!("timeout after {}s", TURN_START_TIMEOUT.as_secs()),
            }),
        }
    }

    /// Stop the running session. Idempotent on `Idle` / `Crashed`.
    pub async fn stop(&self) -> Result<(), MinosError> {
        // Fast path: Idle or Crashed → no-op.
        {
            let state = self.inner.state_tx.borrow().clone();
            match state {
                AgentState::Idle | AgentState::Crashed { .. } => return Ok(()),
                AgentState::Starting { .. } | AgentState::Stopping => {
                    return Err(MinosError::AgentNotRunning);
                }
                AgentState::Running { .. } => {}
            }
        }

        let _ = self.inner.state_tx.send(AgentState::Stopping);

        let active_opt = {
            let mut guard = self.inner.active.lock().await;
            guard.take()
        };
        let Some(mut active) = active_opt else {
            // Lost the race; still transition to Idle for the caller.
            let _ = self.inner.state_tx.send(AgentState::Idle);
            return Ok(());
        };

        // Mark the exit as expected so the supervisor broadcasts Idle, not Crashed.
        active.expected_exit.store(true, Ordering::SeqCst);

        // Best-effort polite goodbyes (bounded).
        let thread_id = active.thread_id.clone();
        let polite_client = Arc::clone(&active.client);
        let _ = tokio::time::timeout(
            STOP_POLITE_TIMEOUT,
            polite_client.call("turn/interrupt", thread_id_request_params(&thread_id)),
        )
        .await;
        let _ = tokio::time::timeout(
            STOP_POLITE_TIMEOUT,
            polite_client.call("thread/archive", thread_id_request_params(&thread_id)),
        )
        .await;

        // Signal the supervisor to tear down the child.
        if let Some(tx) = active.stop_signal_tx.take() {
            let _ = tx.send(());
        }

        // Wait for the supervisor task to finish (bounded). The supervisor
        // sends `state_tx.send(Idle)` on expected exit; we don't need to
        // send it again ourselves. Keep a ceiling so a pathological
        // supervisor doesn't hang `stop()`.
        if let Some(sup) = active.supervisor_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), sup).await;
        }

        // Drain the event pump (it'll exit once the client observes the WS
        // close from the killed child).
        if let Some(pump) = active.event_pump_task.take() {
            pump.abort();
            let _ = pump.await;
        }

        // Shut down the client. Dropping the Arc is insufficient because the
        // event pump held a clone; aborting pump above releases its clone,
        // so our remaining clone (in `polite_client`) + `active.client` get
        // dropped together below.
        drop(polite_client);
        // `active.client` is the last Arc holder at this point; on drop it
        // closes the outbound channel and the internal pump exits.
        drop(active);

        // Ensure the state lands on Idle (supervisor may have already
        // transitioned, but re-sending Idle on already-Idle is a no-op watch
        // write).
        let _ = self.inner.state_tx.send(AgentState::Idle);
        Ok(())
    }
}

/// Choose the first free port in `range` by bind-probing. Mirrors
/// `minos-daemon::handle::start_on_port_range`.
fn pick_free_port(range: std::ops::RangeInclusive<u16>) -> Result<u16, MinosError> {
    let (first, last) = (*range.start(), *range.end());
    for port in range {
        let addr = format!("127.0.0.1:{port}");
        if std::net::TcpListener::bind(&addr).is_ok() {
            return Ok(port);
        }
    }
    Err(MinosError::CodexSpawnFailed {
        message: format!("all ports in range {first}..={last} occupied"),
    })
}

fn ensure_workspace_dir(root: &Path) -> Result<PathBuf, MinosError> {
    std::fs::create_dir_all(root).map_err(|e| MinosError::StoreIo {
        path: root.display().to_string(),
        message: format!("create workspace_root failed: {e}"),
    })?;
    root.canonicalize().map_err(|e| MinosError::StoreIo {
        path: root.display().to_string(),
        message: format!("canonicalize workspace_root failed: {e}"),
    })
}

async fn stop_process_if_present(process: &mut Option<CodexProcess>) {
    if let Some(proc) = process.as_mut() {
        let _ = proc.stop_graceful().await;
    }
}

fn map_timeout(
    res: Result<Result<Value, MinosError>, tokio::time::error::Elapsed>,
    method: &str,
    timeout: Duration,
) -> Result<Value, MinosError> {
    match res {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(MinosError::CodexProtocolError {
            method: method.into(),
            message: format!("timeout after {}s", timeout.as_secs()),
        }),
    }
}

fn map_timeout_unit(
    res: Result<Result<(), MinosError>, tokio::time::error::Elapsed>,
    method: &str,
    timeout: Duration,
) -> Result<(), MinosError> {
    match res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(MinosError::CodexProtocolError {
            method: method.into(),
            message: format!("timeout after {}s", timeout.as_secs()),
        }),
    }
}

fn initialize_params() -> Value {
    serde_json::json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {
            "name": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn thread_id_from_response(value: &Value) -> Option<&str> {
    value
        .get("thread_id")
        .and_then(Value::as_str)
        .or_else(|| value.get("threadId").and_then(Value::as_str))
        .or_else(|| {
            value
                .get("thread")
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
        })
}

fn thread_id_request_params(thread_id: &str) -> Value {
    serde_json::json!({
        "threadId": thread_id,
    })
}

/// Spawn the supervisor task. It owns `process` (which owns the child); it
/// waits in a `select!` between `stop_signal` and the child's natural exit.
///
/// Spec §6.3: on expected exit (stop signal fired and `expected_exit` is
/// true), state_tx.send(Idle). On unexpected exit, state_tx.send(Crashed {
/// reason }). The supervisor is the ONLY code that changes state on process
/// termination — eliminating the race between stop() and a simultaneous
/// crash.
fn spawn_supervisor(
    process: Option<CodexProcess>,
    mut stop_signal_rx: oneshot::Receiver<()>,
    mut ws_closed_rx: oneshot::Receiver<()>,
    expected_exit: Arc<AtomicBool>,
    state_tx: watch::Sender<AgentState>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let child_opt = process.and_then(|mut p| p.take_child());
        match child_opt {
            Some(mut child) => {
                tokio::select! {
                    _ = &mut stop_signal_rx => {
                        // Expected stop. SIGTERM → 3s → SIGKILL.
                        if let Err(e) = child.start_kill() {
                            warn!(error = %e, "supervisor: start_kill failed on expected stop");
                        }
                        match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                            Ok(Ok(_status)) => { /* exited politely */ }
                            Ok(Err(e)) => warn!(
                                error = %e,
                                "supervisor: wait() errored on expected stop",
                            ),
                            Err(_) => {
                                warn!(
                                    "supervisor: SIGTERM grace expired; escalating to SIGKILL",
                                );
                                let _ = child.start_kill();
                                let _ = child.wait().await;
                            }
                        }
                        let _ = state_tx.send(AgentState::Idle);
                    }
                    exit = child.wait() => {
                        let expected = expected_exit.load(Ordering::SeqCst);
                        if expected {
                            let _ = state_tx.send(AgentState::Idle);
                        } else {
                            let reason = match exit {
                                Ok(status) => reason_from_exit(status),
                                Err(e) => format!("wait failed: {e}"),
                            };
                            let _ = state_tx.send(AgentState::Crashed { reason });
                        }
                    }
                    _ = &mut ws_closed_rx => {
                        // WS dropped without child.exit — still "unexpected" unless
                        // stop() set expected_exit. We need to kill the child too
                        // so it doesn't linger.
                        let expected = expected_exit.load(Ordering::SeqCst);
                        let _ = child.start_kill();
                        let exit = child.wait().await;
                        if expected {
                            let _ = state_tx.send(AgentState::Idle);
                        } else {
                            let reason = match exit {
                                Ok(status) => reason_from_exit(status),
                                Err(e) => format!("wait failed: {e}"),
                            };
                            let _ = state_tx.send(AgentState::Crashed { reason });
                        }
                    }
                }
            }
            None => {
                // Test path: no child to supervise. The WS close signal is
                // the authoritative termination.
                tokio::select! {
                    _ = &mut stop_signal_rx => {
                        if expected_exit.load(Ordering::SeqCst) {
                            let _ = state_tx.send(AgentState::Idle);
                        } else {
                            let _ = state_tx.send(AgentState::Crashed {
                                reason: "stop without expected_exit (test path)".into(),
                            });
                        }
                    }
                    _ = &mut ws_closed_rx => {
                        if expected_exit.load(Ordering::SeqCst) {
                            let _ = state_tx.send(AgentState::Idle);
                        } else {
                            // Fake crash: no exit status to decode; the fake
                            // dropping the WS abruptly is our only signal.
                            let _ = state_tx.send(AgentState::Crashed {
                                reason: "codex WS closed unexpectedly".into(),
                            });
                        }
                    }
                }
            }
        }
    })
}

async fn event_pump_loop(
    client: Arc<CodexClient>,
    ingest_tx: broadcast::Sender<RawIngest>,
    agent: AgentName,
    thread_id: String,
    mut ws_closed_tx: Option<oneshot::Sender<()>>,
) {
    while let Some(inbound) = client.next_inbound().await {
        match inbound {
            Inbound::Notification { method, params } => {
                let payload = serde_json::json!({ "method": method, "params": params });
                let ingest = RawIngest {
                    agent,
                    thread_id: thread_id.clone(),
                    payload,
                    ts_ms: current_unix_ms(),
                };
                // `send` fails only when there are no receivers — fine to ignore.
                if let Err(e) = ingest_tx.send(ingest) {
                    debug!(
                        target: "minos_agent_runtime::runtime",
                        error = %e,
                        "ingest broadcast send failed (no subscribers)",
                    );
                }
            }
            Inbound::ServerRequest { id, method, params } => {
                let known = APPROVAL_METHODS.contains(&method.as_str());
                if known {
                    let payload = build_auto_reject(id.clone(), &method);
                    // Extract the inner "result" so we hand `reply()` the
                    // result-only value — the client wraps it in the
                    // {jsonrpc, id, result} envelope itself.
                    let result = payload
                        .get("result")
                        .cloned()
                        .unwrap_or(serde_json::json!({"decision": "rejected"}));
                    if let Err(e) = client.reply(id.clone(), result).await {
                        warn!(
                            target: "minos_agent_runtime::runtime",
                            error = %e,
                            method = %method,
                            "auto-reject reply failed",
                        );
                    } else {
                        info!(
                            target: "minos_agent_runtime::runtime",
                            method = %method,
                            "auto-rejected approval server request",
                        );
                    }
                } else {
                    warn!(
                        target: "minos_agent_runtime::runtime",
                        method = %method,
                        "unexpected server request; forwarding as ingest and not replying",
                    );
                }
                // Forward as a synthetic notification so ingest subscribers see
                // the server request too. The backend's translator will fall
                // through to the Raw variant for unknown method names.
                let synthetic_method = format!("server_request/{method}");
                let payload = serde_json::json!({ "method": synthetic_method, "params": params });
                let _ = ingest_tx.send(RawIngest {
                    agent,
                    thread_id: thread_id.clone(),
                    payload,
                    ts_ms: current_unix_ms(),
                });
            }
            Inbound::Closed => break,
        }
    }
    debug!(
        target: "minos_agent_runtime::runtime",
        "event pump exiting (WS closed)",
    );
    // Inform the supervisor that the WS is gone. Fire the signal once; the
    // receiver may already be closed if the supervisor already exited via
    // another path, which is fine — we just drop on send error.
    if let Some(tx) = ws_closed_tx.take() {
        let _ = tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_new_is_idle_shape() {
        let cfg = AgentRuntimeConfig::new(PathBuf::from("/tmp/ws"));
        assert_eq!(cfg.ws_port_range, 7879..=7883);
        assert_eq!(cfg.event_buffer, DEFAULT_EVENT_BUFFER);
    }

    #[test]
    fn initialize_params_include_mcp_client_info() {
        let params = initialize_params();
        assert_eq!(params["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(params["clientInfo"]["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(params["clientInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(params["capabilities"].is_object());
    }

    #[test]
    fn thread_id_from_response_accepts_nested_thread_object() {
        let value = serde_json::json!({"thread": {"id": "thr-real"}});
        assert_eq!(thread_id_from_response(&value), Some("thr-real"));
    }

    #[test]
    fn pick_free_port_from_empty_range_errors() {
        // Reverse range yields no iteration; pick_free_port must error.
        let r = std::ops::RangeInclusive::new(60000, 59999);
        let err = pick_free_port(r).expect_err("must fail");
        match err {
            MinosError::CodexSpawnFailed { message } => {
                assert!(message.contains("all ports"), "{message}");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn new_starts_in_idle() {
        let cfg = AgentRuntimeConfig::new(PathBuf::from("/tmp/minos-runtime-test"));
        let rt = AgentRuntime::new(cfg);
        assert_eq!(rt.current_state(), AgentState::Idle);
    }

    #[tokio::test]
    async fn stop_on_idle_is_ok() {
        let cfg = AgentRuntimeConfig::new(PathBuf::from("/tmp/minos-runtime-test"));
        let rt = AgentRuntime::new(cfg);
        rt.stop().await.unwrap();
        rt.stop().await.unwrap();
        assert_eq!(rt.current_state(), AgentState::Idle);
    }

    #[tokio::test]
    async fn send_user_message_on_idle_errors_not_running() {
        let cfg = AgentRuntimeConfig::new(PathBuf::from("/tmp/minos-runtime-test"));
        let rt = AgentRuntime::new(cfg);
        let err = rt
            .send_user_message("any", "hi")
            .await
            .expect_err("must fail");
        assert!(matches!(err, MinosError::AgentNotRunning));
    }

    #[tokio::test]
    async fn start_unsupported_agent_errors() {
        let cfg = AgentRuntimeConfig::new(PathBuf::from("/tmp/minos-runtime-test"));
        let rt = AgentRuntime::new(cfg);
        let err = rt.start(AgentName::Claude).await.expect_err("must fail");
        match err {
            MinosError::AgentNotSupported { agent } => {
                assert_eq!(agent, AgentName::Claude);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
