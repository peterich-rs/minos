use std::path::PathBuf;
use std::sync::Arc;

use minos_agent_runtime::{
    AgentLaunchMode, AgentManager, AgentRuntimeConfig, InstanceCaps, RawIngest, ThreadState,
};
use minos_domain::MinosError;
use minos_protocol::{
    AgentLaunchMode as ProtoAgentLaunchMode, CloseReason as ProtoCloseReason, CloseThreadRequest,
    GetThreadParams, GetThreadResponse, InterruptThreadRequest, ListThreadsParams,
    ListThreadsResponse, PauseReason as ProtoPauseReason, SendUserMessageRequest,
    StartAgentRequest, StartAgentResponse, ThreadState as ProtoThreadState, ThreadSummary,
};
use tokio::sync::{broadcast, mpsc, watch};

use crate::store::event_writer::EventWriter;
use crate::store::LocalStore;
use crate::subscription::{AgentStateObserver, Subscription};

/// `AgentGlue` is the daemon-side wrapper that:
/// 1. Owns the `AgentManager` (multi-workspace codex instance manager).
/// 2. Owns the `EventWriter` (single-writer SQLite + relay forwarder).
/// 3. Bridges `AgentManager::ingest_stream()` -> `EventWriter::write_live` so
///    every codex notification is persisted before being broadcast outbound.
///
/// The legacy single-session `AgentRuntime` was retired in Phase C; the
/// existing daemon FFI surface (`StartAgentRequest` / `SendUserMessageRequest`
/// / `stop_agent` / `state_stream`) is preserved here as a thin shim until
/// Tasks C16-C18 rewrite the protocol + FFI together.
pub struct AgentGlue {
    pub manager: Arc<AgentManager>,
    pub writer: Arc<EventWriter>,
    /// Local SQLite store. Owned so `start_agent` / `close_thread` can keep
    /// the parent `threads` / `workspaces` rows in sync with the in-memory
    /// `AgentManager`. Without these the events FK in §8.2 fails the
    /// moment codex emits its first ingest frame.
    store: Arc<LocalStore>,
    /// Watch channel mirroring the most recently observed thread state. The
    /// legacy FFI surface exposes a single `state_stream()` shaped like the
    /// pre-Phase-C `AgentRuntime`. Multi-thread fan-out lands in C17.
    state_tx: Arc<watch::Sender<ThreadState>>,
    state_rx: watch::Receiver<ThreadState>,
    /// Default workspace dir used when `start_agent` is invoked under the
    /// legacy surface (no workspace param). Resolved once at construction
    /// time.
    default_workspace: PathBuf,
}

impl AgentGlue {
    /// Construct a new glue and spawn the `RawIngest -> EventWriter` bridge.
    /// `relay_out_tx` is the single `/devices` outbound channel owned by the
    /// `RelayClient`.
    #[must_use]
    pub fn new(
        workspace_root: PathBuf,
        subprocess_env: Arc<std::collections::HashMap<String, String>>,
        store: Arc<LocalStore>,
        relay_out_tx: mpsc::Sender<minos_protocol::Envelope>,
    ) -> Self {
        let mut cfg = AgentRuntimeConfig::new(workspace_root.clone());
        cfg.subprocess_env = subprocess_env;
        let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let writer = Arc::new(EventWriter::spawn(store.clone(), relay_out_tx));
        Self::wire_with(manager, writer, store, workspace_root)
    }

    /// Test-time / advanced constructor that accepts a pre-built manager and
    /// writer so unit tests can stub one or both.
    pub fn wire_with(
        manager: Arc<AgentManager>,
        writer: Arc<EventWriter>,
        store: Arc<LocalStore>,
        default_workspace: PathBuf,
    ) -> Self {
        // Spawn the bridge: every RawIngest from the manager is forwarded to
        // the EventWriter (which persists + broadcasts the corresponding
        // `Envelope::Ingest` outbound).
        //
        // Each ingest gets one info-level log line so the daemon log shows
        // the codex → host event stream at a glance. Pre-fix this slot was
        // the FK-error spam; post-fix the success path was silent and the
        // user couldn't tell whether codex was active. Volume is bounded
        // by codex's own emit rate (~tens/s/thread per spec §8.7).
        let mut rx = manager.ingest_stream();
        let writer_clone = writer.clone();
        tokio::spawn(async move {
            while let Ok(ingest) = rx.recv().await {
                let thread_id = ingest.thread_id.clone();
                let payload_bytes = serde_json::to_vec(&ingest.payload).map_or(0, |v| v.len());
                match writer_clone.write_live(ingest).await {
                    Ok(seq) => tracing::info!(
                        target: "minos_daemon::agent",
                        thread_id = %thread_id,
                        seq,
                        bytes = payload_bytes,
                        "ingest event committed",
                    ),
                    Err(e) => tracing::error!(
                        target: "minos_daemon::agent",
                        error = %e,
                        thread_id = %thread_id,
                        "EventWriter.write_live failed; event dropped",
                    ),
                }
            }
        });

        let (state_tx, state_rx) = watch::channel(ThreadState::Idle);
        let state_tx = Arc::new(state_tx);
        Self {
            manager,
            writer,
            store,
            state_tx,
            state_rx,
            default_workspace,
        }
    }

    pub async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> Result<StartAgentResponse, MinosError> {
        // Plan note (C16): `Jsonl` is treated identically to `Server` because
        // the JSONL exec path was retired in C18. The mode field stays in the
        // wire shape for forward-compatibility but is effectively ignored.
        let _mode = req.mode.map_or(AgentLaunchMode::Server, runtime_mode);
        // An empty `workspace` falls back to the daemon's default workspace
        // dir for clients (mobile pre-Phase-D) that have not been updated to
        // pick a directory yet.
        let workspace = if req.workspace.is_empty() {
            self.default_workspace.clone()
        } else {
            PathBuf::from(&req.workspace)
        };
        let outcome = self
            .manager
            .start_agent(req.agent, workspace)
            .await
            .map_err(map_anyhow)?;
        let cwd = outcome.cwd.display().to_string();

        // Persist the parent rows the events FK depends on. The codex
        // session id doubles as the thread id (manager.rs `rpc_start_thread`),
        // so the same value lands in both columns. `INSERT OR IGNORE` makes
        // both calls safe to repeat for the same workspace / thread (e.g.
        // a UI retry after a transient error). Failure here is logged but
        // not fatal — start_agent has already spawned codex and the manager
        // has the thread in memory; surfacing a hard error to the caller
        // would leave us with a live codex thread the user can never
        // interact with again.
        let now_ms = current_unix_ms();
        let agent_label = match req.agent {
            minos_domain::AgentName::Codex => "codex",
            minos_domain::AgentName::Claude => "claude",
            minos_domain::AgentName::Gemini => "gemini",
        };
        if let Err(e) = self.store.upsert_workspace(&cwd, now_ms).await {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                workspace = %cwd,
                "store.upsert_workspace failed; events FK may reject ingest",
            );
        }
        if let Err(e) = self
            .store
            .insert_thread(
                &outcome.thread_id,
                &cwd,
                agent_label,
                Some(&outcome.thread_id),
                "idle",
                now_ms,
            )
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                thread_id = %outcome.thread_id,
                "store.insert_thread failed; events FK may reject ingest",
            );
        }

        // Legacy single-state mirror: emit Idle (not Running) because the
        // multi-thread manager keeps per-thread state internally; the
        // single-channel mirror just signals "something is alive". The mobile
        // / Swift surfaces will switch to per-thread state streams in C17/D.
        let _ = self.state_tx.send(ThreadState::Idle);
        Ok(StartAgentResponse {
            session_id: outcome.thread_id,
            cwd,
        })
    }

    pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError> {
        self.manager
            .send_user_message(&req.session_id, req.text)
            .await
            .map_err(map_anyhow)
    }

    pub async fn interrupt_thread(&self, req: InterruptThreadRequest) -> Result<(), MinosError> {
        self.manager
            .interrupt_thread(&req.thread_id)
            .await
            .map_err(map_anyhow)
    }

    pub async fn close_thread(&self, req: CloseThreadRequest) -> Result<(), MinosError> {
        self.manager
            .close_thread(&req.thread_id)
            .await
            .map_err(map_anyhow)?;

        // Mirror the in-memory transition into the local DB so the next
        // daemon start sees the thread as `closed` instead of flipping it
        // to `suspended { daemon_restart }` via §8.6 startup recovery.
        // Logged on failure but non-fatal — the manager has already
        // released the thread.
        if let Err(e) = self
            .store
            .close_thread_row(&req.thread_id, "user_close", current_unix_ms())
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                thread_id = %req.thread_id,
                "store.close_thread_row failed; row will look orphan on next restart",
            );
        }

        let _ = self.state_tx.send(ThreadState::Idle);
        Ok(())
    }

    pub async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        let _ = req; // Filter / agent / pagination plumbing lands with the
                     // SQLite-backed history list (C21+).
        let snap = self.manager.list_threads().await;
        let threads: Vec<ThreadSummary> = snap
            .into_iter()
            .map(|s| ThreadSummary {
                thread_id: s.thread_id,
                agent: minos_domain::AgentName::Codex,
                title: None,
                first_ts_ms: 0,
                last_ts_ms: 0,
                message_count: 0,
                ended_at_ms: None,
                end_reason: None,
            })
            .collect();
        Ok(ListThreadsResponse {
            threads,
            next_before_ts_ms: None,
        })
    }

    pub async fn get_thread(&self, req: GetThreadParams) -> Result<GetThreadResponse, MinosError> {
        let snap = self.manager.list_threads().await;
        let s = snap
            .into_iter()
            .find(|s| s.thread_id == req.thread_id)
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        let thread = ThreadSummary {
            thread_id: s.thread_id.clone(),
            agent: minos_domain::AgentName::Codex,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            end_reason: None,
        };
        Ok(GetThreadResponse {
            thread,
            state: state_to_proto(&s.state),
        })
    }

    #[must_use]
    pub fn subscribe_state(&self, observer: Arc<dyn AgentStateObserver>) -> Arc<Subscription> {
        crate::subscription::spawn_agent_observer(self.state_stream(), observer)
    }

    #[must_use]
    pub fn current_state(&self) -> ThreadState {
        self.state_rx.borrow().clone()
    }

    #[must_use]
    pub fn state_stream(&self) -> watch::Receiver<ThreadState> {
        self.state_rx.clone()
    }

    #[must_use]
    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest> {
        self.manager.ingest_stream()
    }

    pub async fn shutdown(&self) -> Result<(), MinosError> {
        // Best-effort: walk every thread and request close. The detailed
        // shutdown sequence (SIGTERM + grace) lands in C20.
        let snap = self.manager.list_threads().await;
        for s in snap {
            let _ = self.manager.close_thread(&s.thread_id).await;
        }
        Ok(())
    }
}

fn runtime_mode(mode: ProtoAgentLaunchMode) -> AgentLaunchMode {
    match mode {
        ProtoAgentLaunchMode::Jsonl => AgentLaunchMode::Jsonl,
        ProtoAgentLaunchMode::Server => AgentLaunchMode::Server,
    }
}

fn map_anyhow(e: anyhow::Error) -> MinosError {
    MinosError::CodexProtocolError {
        method: "agent_manager".into(),
        message: e.to_string(),
    }
}

fn current_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn state_to_proto(state: &minos_agent_runtime::ThreadState) -> ProtoThreadState {
    use minos_agent_runtime::ThreadState as RtState;
    match state {
        RtState::Starting => ProtoThreadState::Starting,
        RtState::Idle => ProtoThreadState::Idle,
        RtState::Running { turn_started_at_ms } => ProtoThreadState::Running {
            turn_started_at_ms: *turn_started_at_ms,
        },
        RtState::Suspended { reason } => ProtoThreadState::Suspended {
            reason: pause_to_proto(reason),
        },
        RtState::Resuming => ProtoThreadState::Resuming,
        RtState::Closed { reason } => ProtoThreadState::Closed {
            reason: close_to_proto(reason),
        },
    }
}

fn pause_to_proto(r: &minos_agent_runtime::PauseReason) -> ProtoPauseReason {
    use minos_agent_runtime::PauseReason as Rt;
    match r {
        Rt::UserInterrupt => ProtoPauseReason::UserInterrupt,
        Rt::CodexCrashed => ProtoPauseReason::CodexCrashed,
        Rt::DaemonRestart => ProtoPauseReason::DaemonRestart,
        Rt::InstanceReaped => ProtoPauseReason::InstanceReaped,
    }
}

fn close_to_proto(r: &minos_agent_runtime::CloseReason) -> ProtoCloseReason {
    use minos_agent_runtime::CloseReason as Rt;
    match r {
        Rt::UserClose => ProtoCloseReason::UserClose,
        Rt::TerminalError => ProtoCloseReason::TerminalError,
    }
}
