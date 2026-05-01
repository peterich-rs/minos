use std::path::PathBuf;
use std::sync::Arc;

use minos_agent_runtime::{
    AgentLaunchMode, AgentManager, AgentRuntimeConfig, AgentState, InstanceCaps, RawIngest,
};
use minos_domain::MinosError;
use minos_protocol::{
    AgentLaunchMode as ProtoAgentLaunchMode, SendUserMessageRequest, StartAgentRequest,
    StartAgentResponse,
};
use tokio::sync::{broadcast, mpsc, watch};

use crate::store::LocalStore;
use crate::store::event_writer::EventWriter;
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
    /// Watch channel mirroring the most recently observed thread state. The
    /// legacy FFI surface exposes a single `state_stream()` shaped like the
    /// pre-Phase-C `AgentRuntime`. Multi-thread fan-out lands in C17.
    state_tx: Arc<watch::Sender<AgentState>>,
    state_rx: watch::Receiver<AgentState>,
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
        Self::wire_with(manager, writer, workspace_root)
    }

    /// Test-time / advanced constructor that accepts a pre-built manager and
    /// writer so unit tests can stub one or both.
    pub fn wire_with(
        manager: Arc<AgentManager>,
        writer: Arc<EventWriter>,
        default_workspace: PathBuf,
    ) -> Self {
        // Spawn the bridge: every RawIngest from the manager is forwarded to
        // the EventWriter (which persists + broadcasts the corresponding
        // `Envelope::Ingest` outbound).
        let mut rx = manager.ingest_stream();
        let writer_clone = writer.clone();
        tokio::spawn(async move {
            while let Ok(ingest) = rx.recv().await {
                if let Err(e) = writer_clone.write_live(ingest).await {
                    tracing::error!(
                        target: "minos_daemon::agent",
                        error = %e,
                        "EventWriter.write_live failed; event dropped",
                    );
                }
            }
        });

        let (state_tx, state_rx) = watch::channel(AgentState::Idle);
        let state_tx = Arc::new(state_tx);
        Self {
            manager,
            writer,
            state_tx,
            state_rx,
            default_workspace,
        }
    }

    pub async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> Result<StartAgentResponse, MinosError> {
        // Phase C plan deviation: the protocol's `StartAgentRequest` does not
        // yet carry a `workspace` field (introduced by C16). Until the FFI
        // rewrite lands we route every legacy `start_agent` call through the
        // daemon's default workspace dir.
        let _ = req.mode.map_or(AgentLaunchMode::Server, runtime_mode);
        let workspace = self.default_workspace.clone();
        let outcome = self
            .manager
            .start_agent(req.agent, workspace)
            .await
            .map_err(map_anyhow)?;
        let cwd = outcome.cwd.display().to_string();
        let _ = self.state_tx.send(AgentState::Running {
            agent: req.agent,
            thread_id: outcome.thread_id.clone(),
            started_at: std::time::SystemTime::now(),
        });
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

    pub async fn stop_agent(&self) -> Result<(), MinosError> {
        // Legacy surface: `stop_agent` becomes `close_thread` on whichever
        // thread is most recently active. Multi-thread support comes via
        // `close_thread` (C16+).
        let snap = self.manager.list_threads().await;
        if let Some(s) = snap.first() {
            self.manager
                .close_thread(&s.thread_id)
                .await
                .map_err(map_anyhow)?;
        }
        let _ = self.state_tx.send(AgentState::Idle);
        Ok(())
    }

    #[must_use]
    pub fn subscribe_state(&self, observer: Arc<dyn AgentStateObserver>) -> Arc<Subscription> {
        crate::subscription::spawn_agent_observer(self.state_stream(), observer)
    }

    #[must_use]
    pub fn current_state(&self) -> AgentState {
        self.state_rx.borrow().clone()
    }

    #[must_use]
    pub fn state_stream(&self) -> watch::Receiver<AgentState> {
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
