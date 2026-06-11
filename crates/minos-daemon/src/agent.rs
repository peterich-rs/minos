use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use minos_agent_runtime::{
    AgentLaunchMode, AgentManager, AgentRuntimeConfig, InstanceCaps, ManagerEvent, RawIngest,
    SessionPolicies, ThreadState,
};
use minos_codex_protocol::SkillsListResponse as CodexSkillsListResponse;
use minos_domain::MinosError;
use minos_protocol::{
    AgentDispatchRequest, AgentDispatchResponse, AgentLaunchMode as ProtoAgentLaunchMode,
    ApprovalDecisionRequest, CloseReason as ProtoCloseReason, CloseThreadRequest, GetThreadParams,
    GetThreadResponse, HostSkillError, HostSkillSummary, HostSkillsEntry, InterruptThreadRequest,
    ListHostSkillsRequest, ListHostSkillsResponse, ListHostWorkspacesRequest,
    ListHostWorkspacesResponse, ListThreadsParams, ListThreadsResponse,
    PauseReason as ProtoPauseReason, SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
    ThreadState as ProtoThreadState, ThreadSummary, WriteHostSkillConfigRequest,
    WriteHostSkillConfigResponse,
};
use minos_ui_protocol::ThreadEndReason;
use tokio::sync::{broadcast, mpsc, watch};

use crate::store::event_writer::{provider_session_id_from_ingest, EventWriter};
use crate::store::{EventRow, LocalStore, ThreadRow};
use crate::subscription::{AgentStateObserver, Subscription};

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentThreadSnapshot {
    pub thread_id: String,
    pub workspace_root: String,
    pub state: ThreadState,
}

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
        relay_out_tx: mpsc::Sender<minos_protocol::realtime::ClientFrame>,
    ) -> Self {
        let mut cfg = AgentRuntimeConfig::new(workspace_root.clone());
        if let Err(error) = cfg.enable_default_mcp() {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %error,
                "failed to enable default MCP"
            );
        }
        cfg.subprocess_env = subprocess_env;
        #[cfg(feature = "test-support")]
        apply_test_ws_override(&mut cfg);
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
        let mut manager_events = manager.manager_event_stream();
        let store_clone = store.clone();
        let state_tx_clone = state_tx.clone();
        tokio::spawn(async move {
            loop {
                match manager_events.recv().await {
                    Ok(ManagerEvent::ThreadAdded {
                        thread_id,
                        workspace,
                        agent,
                    }) => {
                        let cwd = workspace.display().to_string();
                        persist_thread_parent_rows_inner(
                            &store_clone,
                            &thread_id,
                            &cwd,
                            agent,
                            None,
                        )
                        .await;
                    }
                    Ok(ManagerEvent::ThreadStateChanged {
                        thread_id,
                        new,
                        at_ms,
                        ..
                    }) => {
                        persist_runtime_state_inner(&store_clone, &thread_id, &new, at_ms).await;
                        let _ = state_tx_clone.send(new);
                    }
                    Ok(ManagerEvent::ThreadClosed { thread_id, reason }) => {
                        let state = ThreadState::Closed { reason };
                        let at_ms = current_unix_ms();
                        persist_runtime_state_inner(&store_clone, &thread_id, &state, at_ms).await;
                        let _ = state_tx_clone.send(state);
                    }
                    Ok(ManagerEvent::InstanceCrashed {
                        affected_threads,
                        reason,
                        ..
                    }) => {
                        let state = ThreadState::Suspended { reason };
                        let at_ms = current_unix_ms();
                        for thread_id in affected_threads {
                            persist_runtime_state_inner(&store_clone, &thread_id, &state, at_ms)
                                .await;
                            let _ = state_tx_clone.send(state.clone());
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => tracing::warn!(
                        target: "minos_daemon::agent",
                        skipped,
                        "manager event bridge lagged",
                    ),
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Self {
            manager,
            writer,
            store,
            state_tx,
            state_rx,
            default_workspace,
        }
    }

    pub fn store(&self) -> &Arc<LocalStore> {
        &self.store
    }

    pub async fn read_thread_raw_history(
        &self,
        thread_id: &str,
        from_seq: Option<u64>,
        limit: u32,
    ) -> Result<(Vec<minos_protocol::LocalIngestFrame>, Option<u64>), MinosError> {
        let row = self
            .store
            .get_thread(thread_id)
            .await
            .map_err(|e| map_store_error("read_thread_raw_history", e))?
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        let max_seq = u64::try_from(row.last_seq.max(0)).unwrap_or(0);
        let start = from_seq.unwrap_or(0).saturating_add(1);
        let effective_limit = limit.min(1000);
        let end = start
            .saturating_add(u64::from(effective_limit))
            .saturating_sub(1)
            .min(max_seq);
        let rows = self
            .store
            .read_events(thread_id, start, end)
            .await
            .map_err(|e| map_store_error("read_thread_raw_history", e))?;
        let agent = parse_agent_label(&row.agent)?;
        let mut events = Vec::with_capacity(rows.len());
        for event in rows {
            let payload: serde_json::Value =
                serde_json::from_slice(&event.payload).map_err(|e| {
                    MinosError::CodexProtocolError {
                        method: "read_thread_raw_history".into(),
                        message: e.to_string(),
                    }
                })?;
            events.push(minos_protocol::LocalIngestFrame {
                thread_id: thread_id.to_owned(),
                seq: u64::try_from(event.seq.max(0)).unwrap_or(0),
                agent,
                payload,
                ts_ms: event.ts_ms,
            });
        }
        let next_seq = if end < max_seq {
            Some(end.saturating_add(1))
        } else {
            None
        };
        Ok((events, next_seq))
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
        let workspace = resolve_workspace(&self.default_workspace, &req.workspace);
        let outcome = self
            .manager
            .start_agent(req.agent, workspace)
            .await
            .map_err(map_anyhow)?;
        let cwd = outcome.cwd.display().to_string();
        self.persist_thread_parent_rows(
            &outcome.thread_id,
            &cwd,
            req.agent,
            outcome.provider_session_id.as_deref(),
        )
        .await;

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

    pub async fn start_agent_with_session_id(
        &self,
        session_id: String,
        req: StartAgentRequest,
        initial_user_message: Option<String>,
    ) -> Result<StartAgentResponse, MinosError> {
        let _mode = req.mode.map_or(AgentLaunchMode::Server, runtime_mode);
        let workspace = resolve_workspace(&self.default_workspace, &req.workspace);
        let outcome = self
            .manager
            .start_agent_with_thread_id(req.agent, workspace, session_id.clone(), None)
            .await
            .map_err(map_anyhow)?;
        let cwd = outcome.cwd.display().to_string();
        self.persist_thread_parent_rows(
            &session_id,
            &cwd,
            req.agent,
            outcome.provider_session_id.as_deref(),
        )
        .await;

        if let Some(message) = initial_user_message
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty())
        {
            self.manager
                .send_user_message(&session_id, message.to_string())
                .await
                .map_err(map_anyhow)?;
        }

        let _ = self.state_tx.send(ThreadState::Idle);
        Ok(StartAgentResponse { session_id, cwd })
    }

    pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError> {
        self.manager
            .send_user_message(&req.session_id, req.text)
            .await
            .map_err(map_anyhow)?;
        self.persist_current_provider_session_id(&req.session_id)
            .await;
        Ok(())
    }

    pub async fn resolve_approval(&self, req: ApprovalDecisionRequest) -> Result<(), MinosError> {
        self.manager
            .resolve_approval(&req.request_id, &req.thread_id, req.decision)
            .await
            .map_err(map_anyhow)
    }

    pub async fn respond_opencode_permission(
        &self,
        req: minos_protocol::RespondOpencodePermissionRequest,
    ) -> Result<(), MinosError> {
        self.manager
            .respond_opencode_permission(&req.thread_id, &req.permission_id, &req.response)
            .await
            .map_err(map_anyhow)
    }

    pub async fn dispatch_message(
        &self,
        req: AgentDispatchRequest,
    ) -> Result<AgentDispatchResponse, MinosError> {
        let AgentDispatchRequest {
            agent,
            session_id,
            text,
            workspace,
            approval_policy,
            sandbox_policy,
            conversation_id: _,
            origin_message_id: _,
        } = req;

        if let Some(existing_session_id) = session_id.as_deref() {
            self.ensure_thread_registered(existing_session_id).await?;
        }

        let policies = if approval_policy.is_none() && sandbox_policy.is_none() {
            None
        } else {
            Some(SessionPolicies {
                approval_policy,
                sandbox_policy,
            })
        };

        let outcome = self
            .manager
            .dispatch_message(
                agent,
                resolve_workspace(&self.default_workspace, &workspace),
                session_id,
                text,
                policies,
            )
            .await
            .map_err(map_anyhow)?;
        let cwd = outcome.cwd.display().to_string();
        self.persist_thread_parent_rows(
            &outcome.session_id,
            &cwd,
            agent,
            outcome.provider_session_id.as_deref(),
        )
        .await;

        Ok(AgentDispatchResponse {
            session_id: outcome.session_id,
        })
    }

    async fn persist_thread_parent_rows(
        &self,
        thread_id: &str,
        cwd: &str,
        agent: minos_domain::AgentName,
        provider_session_id: Option<&str>,
    ) {
        persist_thread_parent_rows_inner(&self.store, thread_id, cwd, agent, provider_session_id)
            .await;
    }

    async fn persist_current_provider_session_id(&self, thread_id: &str) {
        let provider_session_id = self.manager.thread_provider_session_id(thread_id).await;
        if provider_session_id.is_none() {
            return;
        }
        if let Err(e) = self
            .store
            .update_thread_provider_session_id(thread_id, provider_session_id.as_deref())
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                thread_id,
                "store.update_thread_provider_session_id failed",
            );
        }
    }

    async fn resolve_provider_session_id(
        &self,
        row: &ThreadRow,
        agent: minos_domain::AgentName,
    ) -> Result<Option<String>, MinosError> {
        if agent == minos_domain::AgentName::Codex {
            return Ok(row.codex_session_id.clone());
        }

        if let Some(session_id) = row
            .codex_session_id
            .as_deref()
            .filter(|session_id| *session_id != row.thread_id)
        {
            return Ok(Some(session_id.to_string()));
        }

        self.latest_provider_session_id_from_events(row, agent)
            .await
    }

    async fn latest_provider_session_id_from_events(
        &self,
        row: &ThreadRow,
        agent: minos_domain::AgentName,
    ) -> Result<Option<String>, MinosError> {
        let max_seq = u64::try_from(row.last_seq.max(0)).unwrap_or(0);
        if max_seq == 0 {
            return Ok(None);
        }

        let rows = self
            .store
            .read_events(&row.thread_id, 1, max_seq)
            .await
            .map_err(|e| map_store_error("latest_provider_session_id_from_events", e))?;
        Ok(rows
            .iter()
            .rev()
            .find_map(|event| provider_session_id_from_event(&row.thread_id, agent, event)))
    }

    pub async fn ensure_thread_registered(&self, thread_id: &str) -> Result<(), MinosError> {
        if self.manager.has_thread(thread_id).await {
            return Ok(());
        }
        let row = self
            .store
            .get_thread(thread_id)
            .await
            .map_err(|e| map_store_error("ensure_thread_registered", e))?
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        let state = row_state_to_runtime(&row)?;
        let agent = parse_agent_label(&row.agent)?;
        let provider_session_id = self.resolve_provider_session_id(&row, agent).await?;
        self.manager
            .register_persisted_thread(
                row.thread_id.clone(),
                PathBuf::from(&row.workspace_root),
                agent,
                provider_session_id,
                state,
                u64::try_from(row.last_seq.max(0)).unwrap_or(u64::MAX),
            )
            .await
            .map_err(map_anyhow)
    }

    pub async fn resume_thread(&self, thread_id: &str) -> Result<StartAgentResponse, MinosError> {
        let row = self
            .store
            .get_thread(thread_id)
            .await
            .map_err(|e| map_store_error("resume_thread", e))?
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        if matches!(
            row_state_to_runtime(&row)?,
            minos_agent_runtime::ThreadState::Closed { .. }
        ) {
            return Err(MinosError::AgentSessionIdMismatch);
        }
        let agent = parse_agent_label(&row.agent)?;
        let provider_session_id = self.resolve_provider_session_id(&row, agent).await?;
        self.manager
            .register_persisted_thread(
                row.thread_id.clone(),
                PathBuf::from(&row.workspace_root),
                agent,
                provider_session_id,
                row_state_to_runtime(&row)?,
                u64::try_from(row.last_seq.max(0)).unwrap_or(u64::MAX),
            )
            .await
            .map_err(map_anyhow)?;
        Ok(StartAgentResponse {
            session_id: row.thread_id,
            cwd: row.workspace_root,
        })
    }

    pub async fn list_host_skills(
        &self,
        req: ListHostSkillsRequest,
    ) -> Result<ListHostSkillsResponse, MinosError> {
        let workspace = resolve_workspace(&self.default_workspace, &req.workspace);
        let response = self
            .manager
            .list_host_skills(workspace, req.force_reload)
            .await
            .map_err(map_anyhow)?;
        Ok(map_host_skills_response(response))
    }

    pub fn list_host_workspaces(
        &self,
        req: ListHostWorkspacesRequest,
    ) -> Result<ListHostWorkspacesResponse, MinosError> {
        list_host_workspaces(req)
    }

    pub async fn write_host_skill_config(
        &self,
        req: WriteHostSkillConfigRequest,
    ) -> Result<WriteHostSkillConfigResponse, MinosError> {
        let workspace = resolve_workspace(&self.default_workspace, &req.workspace);
        let response = self
            .manager
            .write_host_skill_config(workspace, PathBuf::from(req.path), req.enabled)
            .await
            .map_err(map_anyhow)?;
        Ok(WriteHostSkillConfigResponse {
            effective_enabled: response.effective_enabled,
        })
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

    pub async fn delete_thread(&self, req: CloseThreadRequest) -> Result<(), MinosError> {
        if let Err(e) = self.manager.close_thread(&req.thread_id).await {
            tracing::debug!(
                target: "minos_daemon::agent",
                error = %e,
                thread_id = %req.thread_id,
                "manager.close_thread skipped during local delete",
            );
        }

        let deleted = self
            .store
            .delete_thread(&req.thread_id)
            .await
            .map_err(|e| map_store_error("delete_thread", e))?;
        if deleted == 0 {
            return Err(MinosError::ThreadNotFound {
                thread_id: req.thread_id,
            });
        }

        let _ = self.state_tx.send(ThreadState::Idle);
        Ok(())
    }

    pub async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        let agent_filter = req.agent.map(agent_label);
        let threads = self
            .store
            .list_threads(req.before_ts_ms, Some(req.limit), agent_filter)
            .await
            .map_err(|e| map_store_error("list_threads", e))?
            .into_iter()
            .map(thread_summary_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListThreadsResponse {
            threads,
            next_before_ts_ms: None,
        })
    }

    pub async fn get_thread(&self, req: GetThreadParams) -> Result<GetThreadResponse, MinosError> {
        let row = self
            .store
            .get_thread(&req.thread_id)
            .await
            .map_err(|e| map_store_error("get_thread", e))?
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        let live_state = self
            .manager
            .list_threads()
            .await
            .into_iter()
            .find(|snapshot| snapshot.thread_id == req.thread_id)
            .map(|snapshot| state_to_proto(&snapshot.state));
        let thread = thread_summary_from_row(row.clone())?;
        Ok(GetThreadResponse {
            thread,
            state: live_state.unwrap_or(row_state_to_proto(&row)?),
        })
    }

    pub async fn current_agent_thread(&self) -> Result<Option<AgentThreadSnapshot>, MinosError> {
        let live_snapshots = self.manager.list_threads().await;
        let rows = self
            .store
            .list_threads(None, Some(500), None)
            .await
            .map_err(|e| map_store_error("current_agent_thread", e))?;
        let row_by_thread = rows
            .iter()
            .map(|row| (row.thread_id.as_str(), row))
            .collect::<HashMap<_, _>>();

        let mut live_candidates = live_snapshots
            .into_iter()
            .filter(|snapshot| !matches!(snapshot.state, ThreadState::Closed { .. }))
            .map(|snapshot| {
                let last_activity_at = row_by_thread
                    .get(snapshot.thread_id.as_str())
                    .map_or(0, |row| row.last_activity_at);
                (state_priority(&snapshot.state), last_activity_at, snapshot)
            })
            .collect::<Vec<_>>();
        live_candidates.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.thread_id.as_str().cmp(right.2.thread_id.as_str()))
        });
        if let Some((_, _, snapshot)) = live_candidates.into_iter().next() {
            return Ok(Some(AgentThreadSnapshot {
                thread_id: snapshot.thread_id,
                workspace_root: snapshot.workspace.display().to_string(),
                state: snapshot.state,
            }));
        }

        for row in rows {
            let state = row_state_to_runtime(&row)?;
            if matches!(
                state,
                ThreadState::Starting
                    | ThreadState::Idle
                    | ThreadState::Running { .. }
                    | ThreadState::Resuming
            ) {
                return Ok(Some(AgentThreadSnapshot {
                    thread_id: row.thread_id,
                    workspace_root: row.workspace_root,
                    state,
                }));
            }
        }
        Ok(None)
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

    // ── Project operations ──────────────────────────────────────────────

    pub async fn create_project(
        &self,
        req: minos_protocol::CreateProjectRequest,
    ) -> Result<minos_protocol::CreateProjectResponse, MinosError> {
        let project_id = uuid::Uuid::new_v4().to_string();
        let now_ms = current_unix_ms();

        // Ensure the workspace directory exists under .minos/workspaces/<slug>
        let workspace_dir = self
            .default_workspace
            .parent()
            .unwrap_or(&self.default_workspace)
            .join("workspaces")
            .join(&req.workspace_slug);
        let workspace_path = req
            .workspace_path
            .clone()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| workspace_dir.display().to_string());
        if let Err(e) = std::fs::create_dir_all(&workspace_dir) {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                path = %workspace_dir.display(),
                "failed to create project workspace directory",
            );
        }

        self.store
            .create_project(&project_id, &req.name, &req.workspace_slug, now_ms)
            .await
            .map_err(|e| map_store_error("create_project", e))?;

        // Also register the workspace in the workspaces table so threads
        // can reference it.
        let ws_root = workspace_dir.display().to_string();
        if let Err(e) = self.store.upsert_workspace(&ws_root, now_ms).await {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                "upsert_workspace for project failed",
            );
        }

        Ok(minos_protocol::CreateProjectResponse {
            project: minos_protocol::ProjectSummary {
                project_id,
                name: req.name,
                workspace_slug: req.workspace_slug,
                workspace_path: Some(workspace_path),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
                thread_count: 0,
            },
        })
    }

    pub async fn list_projects(&self) -> Result<minos_protocol::ListProjectsResponse, MinosError> {
        let rows = self
            .store
            .list_projects()
            .await
            .map_err(|e| map_store_error("list_projects", e))?;

        let mut projects = Vec::with_capacity(rows.len());
        for row in rows {
            #[allow(clippy::cast_possible_truncation)]
            let thread_count = self
                .store
                .list_threads_by_project(&row.project_id, None, Some(500))
                .await
                .map_or(0, |threads| threads.len() as u32);
            projects.push(minos_protocol::ProjectSummary {
                project_id: row.project_id,
                name: row.name,
                workspace_path: Some(project_workspace_dir(
                    &self.default_workspace,
                    &row.workspace_slug,
                )),
                workspace_slug: row.workspace_slug,
                created_at_ms: row.created_at,
                updated_at_ms: row.updated_at,
                thread_count,
            });
        }

        Ok(minos_protocol::ListProjectsResponse { projects })
    }

    pub async fn update_project(
        &self,
        req: minos_protocol::UpdateProjectRequest,
    ) -> Result<(), MinosError> {
        let now_ms = current_unix_ms();
        self.store
            .update_project_name(&req.project_id, &req.name, now_ms)
            .await
            .map_err(|e| map_store_error("update_project", e))
    }

    pub async fn delete_project(
        &self,
        req: minos_protocol::DeleteProjectRequest,
    ) -> Result<(), MinosError> {
        self.store
            .delete_project(&req.project_id)
            .await
            .map_err(|e| map_store_error("delete_project", e))
    }

    pub async fn list_project_threads(
        &self,
        req: minos_protocol::ListProjectThreadsParams,
    ) -> Result<minos_protocol::ListProjectThreadsResponse, MinosError> {
        let threads = self
            .store
            .list_threads_by_project(&req.project_id, req.before_ts_ms, Some(req.limit))
            .await
            .map_err(|e| map_store_error("list_project_threads", e))?
            .into_iter()
            .map(thread_summary_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::ListProjectThreadsResponse { threads })
    }

    /// Start an agent within a project context. Creates the thread and
    /// assigns it to the project.
    pub async fn start_agent_in_project(
        &self,
        req: StartAgentRequest,
        project_id: &str,
        workspace_slug: Option<&str>,
    ) -> Result<StartAgentResponse, MinosError> {
        let local_project = self
            .store
            .get_project(project_id)
            .await
            .map_err(|e| map_store_error("start_agent_in_project", e))?;
        let resolved_workspace_slug = local_project
            .as_ref()
            .map(|project| project.workspace_slug.as_str())
            .or(workspace_slug)
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "start_agent_in_project".into(),
                message: format!("project not found: {project_id}"),
            })?;

        let workspace_dir = if req.workspace.trim().is_empty() {
            self.default_workspace
                .parent()
                .unwrap_or(&self.default_workspace)
                .join("workspaces")
                .join(resolved_workspace_slug)
        } else {
            PathBuf::from(req.workspace.trim())
        };
        if let Err(e) = std::fs::create_dir_all(&workspace_dir) {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                path = %workspace_dir.display(),
                "failed to create project workspace directory",
            );
        }

        let mut project_req = req;
        project_req.workspace = workspace_dir.display().to_string();

        let response = self.start_agent(project_req).await?;

        if local_project.is_some() {
            if let Err(e) = self
                .store
                .assign_thread_to_project(&response.session_id, project_id)
                .await
            {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    thread_id = %response.session_id,
                    project_id = %project_id,
                    "assign_thread_to_project failed",
                );
            }

            let now_ms = current_unix_ms();
            let _ = self.store.touch_project(project_id, now_ms).await;
        }

        Ok(response)
    }
}

#[cfg(feature = "test-support")]
fn apply_test_ws_override(cfg: &mut AgentRuntimeConfig) {
    let Some(raw) = std::env::var("MINOS_TEST_CODEX_WS_URL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    cfg.test_ws_url = Some(
        url::Url::parse(&raw)
            .unwrap_or_else(|error| panic!("invalid MINOS_TEST_CODEX_WS_URL `{raw}`: {error}")),
    );
}

fn runtime_mode(mode: ProtoAgentLaunchMode) -> AgentLaunchMode {
    match mode {
        ProtoAgentLaunchMode::Jsonl => AgentLaunchMode::Jsonl,
        ProtoAgentLaunchMode::Server => AgentLaunchMode::Server,
    }
}

fn resolve_workspace(default_workspace: &std::path::Path, workspace: &str) -> PathBuf {
    if workspace.is_empty() {
        default_workspace.to_path_buf()
    } else {
        PathBuf::from(workspace)
    }
}

fn list_host_workspaces(
    req: ListHostWorkspacesRequest,
) -> Result<ListHostWorkspacesResponse, MinosError> {
    let home = home_dir().ok_or_else(|| MinosError::CodexProtocolError {
        method: "list_host_workspaces".into(),
        message: "HOME is not set".into(),
    })?;
    let home = canonicalize_dir(&home, "home")?;
    let requested_root = req
        .root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(|root| expand_home_path(root, &home))
        .unwrap_or_else(|| home.clone());
    let root = canonicalize_dir(&requested_root, "root")?;
    if !root.starts_with(&home) {
        return Err(MinosError::CodexProtocolError {
            method: "list_host_workspaces".into(),
            message: "root must be under the host user's home directory".into(),
        });
    }

    let mut entries = std::fs::read_dir(&root)
        .map_err(|error| MinosError::CodexProtocolError {
            method: "list_host_workspaces".into(),
            message: format!("failed to read {}: {error}", root.display()),
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') {
                return None;
            }
            let path = entry.path();
            Some(minos_protocol::HostWorkspaceSummary {
                is_git_repo: path.join(".git").is_dir(),
                path: path.display().to_string(),
                display_name: file_name,
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    let limit = if req.limit == 0 {
        100
    } else {
        req.limit.min(500)
    };
    entries.truncate(usize::try_from(limit).unwrap_or(500));

    Ok(ListHostWorkspacesResponse {
        root: root.display().to_string(),
        workspaces: entries,
    })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn expand_home_path(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    }
}

fn canonicalize_dir(path: &Path, label: &str) -> Result<PathBuf, MinosError> {
    let canonical =
        std::fs::canonicalize(path).map_err(|error| MinosError::CodexProtocolError {
            method: "list_host_workspaces".into(),
            message: format!("failed to resolve {label} {}: {error}", path.display()),
        })?;
    if canonical.is_dir() {
        Ok(canonical)
    } else {
        Err(MinosError::CodexProtocolError {
            method: "list_host_workspaces".into(),
            message: format!("{label} is not a directory: {}", canonical.display()),
        })
    }
}

fn project_workspace_dir(default_workspace: &std::path::Path, workspace_slug: &str) -> String {
    default_workspace
        .parent()
        .unwrap_or(default_workspace)
        .join("workspaces")
        .join(workspace_slug)
        .display()
        .to_string()
}

fn thread_summary_from_row(row: crate::store::ThreadRow) -> Result<ThreadSummary, MinosError> {
    let end_reason = row_end_reason(&row);
    Ok(ThreadSummary {
        thread_id: row.thread_id,
        agent: parse_agent_label(&row.agent)?,
        title: None,
        first_ts_ms: row.started_at,
        last_ts_ms: row.last_activity_at,
        message_count: u32::try_from(row.last_seq.max(0)).unwrap_or(u32::MAX),
        ended_at_ms: row.ended_at,
        end_reason,
    })
}

fn row_end_reason(row: &crate::store::ThreadRow) -> Option<ThreadEndReason> {
    match row.last_close_reason.as_deref() {
        Some("user_close") => Some(ThreadEndReason::UserStopped),
        Some("terminal_error") => Some(ThreadEndReason::Crashed {
            message: "terminal_error".into(),
        }),
        Some(other) => Some(ThreadEndReason::Crashed {
            message: other.to_string(),
        }),
        None => None,
    }
}

pub(crate) fn row_state_to_proto(
    row: &crate::store::ThreadRow,
) -> Result<ProtoThreadState, MinosError> {
    match row.status.as_str() {
        "starting" => Ok(ProtoThreadState::Starting),
        "idle" => Ok(ProtoThreadState::Idle),
        "running" => Ok(ProtoThreadState::Running {
            turn_started_at_ms: row.last_activity_at,
        }),
        "resuming" => Ok(ProtoThreadState::Resuming),
        "suspended" => Ok(ProtoThreadState::Suspended {
            reason: parse_pause_reason(row.last_pause_reason.as_deref())?,
        }),
        "closed" => Ok(ProtoThreadState::Closed {
            reason: parse_close_reason(row.last_close_reason.as_deref())?,
        }),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.thread_status".into(),
            message: format!("unknown persisted thread status: {other}"),
        }),
    }
}

fn row_state_to_runtime(
    row: &crate::store::ThreadRow,
) -> Result<minos_agent_runtime::ThreadState, MinosError> {
    match row.status.as_str() {
        "starting" => Ok(minos_agent_runtime::ThreadState::Starting),
        "idle" => Ok(minos_agent_runtime::ThreadState::Idle),
        "running" => Ok(minos_agent_runtime::ThreadState::Running {
            turn_started_at_ms: row.last_activity_at,
        }),
        "resuming" => Ok(minos_agent_runtime::ThreadState::Resuming),
        "suspended" => Ok(minos_agent_runtime::ThreadState::Suspended {
            reason: parse_pause_reason_runtime(row.last_pause_reason.as_deref())?,
        }),
        "closed" => Ok(minos_agent_runtime::ThreadState::Closed {
            reason: parse_close_reason_runtime(row.last_close_reason.as_deref())?,
        }),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.thread_status".into(),
            message: format!("unknown persisted thread status: {other}"),
        }),
    }
}

pub(crate) fn parse_agent_label(agent: &str) -> Result<minos_domain::AgentName, MinosError> {
    match agent {
        "codex" => Ok(minos_domain::AgentName::Codex),
        "claude" => Ok(minos_domain::AgentName::Claude),
        "gemini" => Ok(minos_domain::AgentName::Gemini),
        "opencode" => Ok(minos_domain::AgentName::Opencode),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.thread_agent".into(),
            message: format!("unknown persisted agent: {other}"),
        }),
    }
}

fn agent_label(agent: minos_domain::AgentName) -> &'static str {
    match agent {
        minos_domain::AgentName::Codex => "codex",
        minos_domain::AgentName::Claude => "claude",
        minos_domain::AgentName::Gemini => "gemini",
        minos_domain::AgentName::Opencode => "opencode",
    }
}

fn parse_pause_reason(reason: Option<&str>) -> Result<ProtoPauseReason, MinosError> {
    match reason.unwrap_or("daemon_restart") {
        "user_interrupt" => Ok(ProtoPauseReason::UserInterrupt),
        "codex_crashed" => Ok(ProtoPauseReason::CodexCrashed),
        "daemon_restart" => Ok(ProtoPauseReason::DaemonRestart),
        "instance_reaped" => Ok(ProtoPauseReason::InstanceReaped),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.pause_reason".into(),
            message: format!("unknown persisted pause reason: {other}"),
        }),
    }
}

fn parse_pause_reason_runtime(
    reason: Option<&str>,
) -> Result<minos_agent_runtime::PauseReason, MinosError> {
    match reason.unwrap_or("daemon_restart") {
        "user_interrupt" => Ok(minos_agent_runtime::PauseReason::UserInterrupt),
        "codex_crashed" => Ok(minos_agent_runtime::PauseReason::CodexCrashed),
        "daemon_restart" => Ok(minos_agent_runtime::PauseReason::DaemonRestart),
        "instance_reaped" => Ok(minos_agent_runtime::PauseReason::InstanceReaped),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.pause_reason".into(),
            message: format!("unknown persisted pause reason: {other}"),
        }),
    }
}

fn parse_close_reason(reason: Option<&str>) -> Result<ProtoCloseReason, MinosError> {
    match reason.unwrap_or("user_close") {
        "user_close" => Ok(ProtoCloseReason::UserClose),
        "terminal_error" => Ok(ProtoCloseReason::TerminalError),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.close_reason".into(),
            message: format!("unknown persisted close reason: {other}"),
        }),
    }
}

fn parse_close_reason_runtime(
    reason: Option<&str>,
) -> Result<minos_agent_runtime::CloseReason, MinosError> {
    match reason.unwrap_or("user_close") {
        "user_close" => Ok(minos_agent_runtime::CloseReason::UserClose),
        "terminal_error" => Ok(minos_agent_runtime::CloseReason::TerminalError),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.close_reason".into(),
            message: format!("unknown persisted close reason: {other}"),
        }),
    }
}

fn state_priority(state: &ThreadState) -> u8 {
    match state {
        ThreadState::Running { .. } => 0,
        ThreadState::Starting | ThreadState::Resuming => 1,
        ThreadState::Idle => 2,
        ThreadState::Suspended { .. } => 3,
        ThreadState::Closed { .. } => 4,
    }
}

async fn persist_runtime_state_inner(
    store: &LocalStore,
    thread_id: &str,
    state: &ThreadState,
    at_ms: i64,
) {
    let (status, pause_reason, close_reason, ended_at) = runtime_state_columns(state, at_ms);
    match store
        .update_thread_status(
            thread_id,
            status,
            pause_reason,
            close_reason,
            ended_at,
            at_ms,
        )
        .await
    {
        Ok(0) => tracing::warn!(
            target: "minos_daemon::agent",
            thread_id,
            status,
            "store.update_thread_status affected no rows",
        ),
        Ok(_) => {}
        Err(error) => tracing::warn!(
            target: "minos_daemon::agent",
            error = %error,
            thread_id,
            status,
            "store.update_thread_status failed",
        ),
    }
}

fn runtime_state_columns(
    state: &ThreadState,
    at_ms: i64,
) -> (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<i64>,
) {
    match state {
        ThreadState::Starting => ("starting", None, None, None),
        ThreadState::Idle => ("idle", None, None, None),
        ThreadState::Running { .. } => ("running", None, None, None),
        ThreadState::Suspended { reason } => (
            "suspended",
            Some(runtime_pause_reason_label(reason)),
            None,
            None,
        ),
        ThreadState::Resuming => ("resuming", None, None, None),
        ThreadState::Closed { reason } => (
            "closed",
            None,
            Some(runtime_close_reason_label(reason)),
            Some(at_ms),
        ),
    }
}

fn runtime_pause_reason_label(reason: &minos_agent_runtime::PauseReason) -> &'static str {
    match reason {
        minos_agent_runtime::PauseReason::UserInterrupt => "user_interrupt",
        minos_agent_runtime::PauseReason::CodexCrashed => "codex_crashed",
        minos_agent_runtime::PauseReason::DaemonRestart => "daemon_restart",
        minos_agent_runtime::PauseReason::InstanceReaped => "instance_reaped",
    }
}

fn runtime_close_reason_label(reason: &minos_agent_runtime::CloseReason) -> &'static str {
    match reason {
        minos_agent_runtime::CloseReason::UserClose => "user_close",
        minos_agent_runtime::CloseReason::TerminalError => "terminal_error",
    }
}

fn map_host_skills_response(response: CodexSkillsListResponse) -> ListHostSkillsResponse {
    ListHostSkillsResponse {
        data: response
            .data
            .into_iter()
            .map(|entry| HostSkillsEntry {
                cwd: entry.cwd,
                errors: entry
                    .errors
                    .into_iter()
                    .map(|error| HostSkillError {
                        path: error.path,
                        message: error.message,
                    })
                    .collect(),
                skills: entry
                    .skills
                    .into_iter()
                    .map(|skill| HostSkillSummary {
                        name: skill.name,
                        path: skill.path.0,
                        description: skill.description,
                        enabled: skill.enabled,
                        scope: skill.scope.to_string(),
                        display_name: skill
                            .interface
                            .as_ref()
                            .and_then(|interface| interface.display_name.clone()),
                        short_description: skill
                            .interface
                            .and_then(|interface| interface.short_description)
                            .or(skill.short_description),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn map_anyhow(e: anyhow::Error) -> MinosError {
    MinosError::CodexProtocolError {
        method: "agent_manager".into(),
        message: e.to_string(),
    }
}

pub(crate) fn map_store_error(operation: &str, e: anyhow::Error) -> MinosError {
    MinosError::StoreIo {
        path: "local_store".into(),
        message: format!("{operation}: {e}"),
    }
}

fn current_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

async fn persist_thread_parent_rows_inner(
    store: &LocalStore,
    thread_id: &str,
    cwd: &str,
    agent: minos_domain::AgentName,
    provider_session_id: Option<&str>,
) {
    let now_ms = current_unix_ms();
    if let Err(e) = store.upsert_workspace(cwd, now_ms).await {
        tracing::warn!(
            target: "minos_daemon::agent",
            error = %e,
            workspace = %cwd,
            "store.upsert_workspace failed; events FK may reject ingest",
        );
    }
    if let Err(e) = store
        .insert_thread(
            thread_id,
            cwd,
            agent_label(agent),
            provider_session_id,
            "idle",
            now_ms,
        )
        .await
    {
        tracing::warn!(
            target: "minos_daemon::agent",
            error = %e,
            thread_id = %thread_id,
            "store.insert_thread failed; events FK may reject ingest",
        );
    }
    if let Some(provider_session_id) = provider_session_id {
        if let Err(e) = store
            .update_thread_provider_session_id(thread_id, Some(provider_session_id))
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                thread_id = %thread_id,
                "store.update_thread_provider_session_id failed",
            );
        }
    }
}

fn provider_session_id_from_event(
    thread_id: &str,
    agent: minos_domain::AgentName,
    event: &EventRow,
) -> Option<String> {
    let payload = serde_json::from_slice(&event.payload).ok()?;
    provider_session_id_from_ingest(&RawIngest {
        agent,
        thread_id: thread_id.to_string(),
        payload,
        ts_ms: event.ts_ms,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestGlue {
        _tmp: tempfile::TempDir,
        glue: AgentGlue,
    }

    async fn test_glue() -> TestGlue {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            crate::store::LocalStore::open(&tmp.path().join("daemon.sqlite"))
                .await
                .unwrap(),
        );
        let (out_tx, _out_rx) = mpsc::channel(8);
        TestGlue {
            glue: AgentGlue::new(
                tmp.path().join("workspaces"),
                Arc::new(std::collections::HashMap::new()),
                store,
                out_tx,
            ),
            _tmp: tmp,
        }
    }

    async fn seed_thread(
        glue: &AgentGlue,
        thread_id: &str,
        agent: &str,
        started_at: i64,
        last_activity_at: i64,
    ) {
        glue.store.upsert_workspace("/w", started_at).await.unwrap();
        sqlx::query(
            "INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES (?, '/w', ?, 'idle', 3, ?, ?)",
        )
        .bind(thread_id)
        .bind(agent)
        .bind(started_at)
        .bind(last_activity_at)
        .execute(glue.store.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_threads_reads_persisted_rows_and_filters_agent() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-a", "codex", 10, 20).await;
        seed_thread(&test.glue, "thr-b", "claude", 30, 40).await;

        let response = test
            .glue
            .list_threads(ListThreadsParams {
                limit: 50,
                before_ts_ms: None,
                agent: Some(minos_domain::AgentName::Claude),
            })
            .await
            .unwrap();

        assert_eq!(response.threads.len(), 1);
        assert_eq!(response.threads[0].thread_id, "thr-b");
        assert_eq!(response.threads[0].agent, minos_domain::AgentName::Claude);
        assert_eq!(response.threads[0].message_count, 3);
        assert_eq!(response.threads[0].first_ts_ms, 30);
        assert_eq!(response.threads[0].last_ts_ms, 40);
    }

    #[tokio::test]
    async fn get_thread_uses_persisted_suspended_state() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-s", "codex", 10, 20).await;
        sqlx::query(
            "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart' WHERE thread_id = 'thr-s'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();

        let response = test
            .glue
            .get_thread(GetThreadParams {
                thread_id: "thr-s".into(),
            })
            .await
            .unwrap();

        assert_eq!(response.thread.thread_id, "thr-s");
        assert_eq!(
            response.state,
            ProtoThreadState::Suspended {
                reason: ProtoPauseReason::DaemonRestart,
            }
        );
    }

    #[tokio::test]
    async fn get_thread_maps_closed_reason_from_store() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-c", "codex", 10, 20).await;
        test.glue
            .store
            .close_thread_row("thr-c", "user_close", 55)
            .await
            .unwrap();

        let response = test
            .glue
            .get_thread(GetThreadParams {
                thread_id: "thr-c".into(),
            })
            .await
            .unwrap();

        assert_eq!(response.thread.ended_at_ms, Some(55));
        assert_eq!(
            response.thread.end_reason,
            Some(ThreadEndReason::UserStopped)
        );
        assert_eq!(
            response.state,
            ProtoThreadState::Closed {
                reason: ProtoCloseReason::UserClose,
            }
        );
    }

    #[tokio::test]
    async fn manager_state_change_persists_thread_status() {
        let test = test_glue().await;
        let workspace = test._tmp.path().join("live-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_root = workspace.display().to_string();
        test.glue
            .store
            .upsert_workspace(&workspace_root, 10)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES ('thr-live', ?, 'codex', 'idle', 0, 10, 10)",
        )
        .bind(&workspace_root)
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        test.glue
            .manager
            .register_persisted_thread(
                "thr-live".into(),
                workspace,
                minos_domain::AgentName::Codex,
                Some("thr-live".into()),
                ThreadState::Idle,
                0,
            )
            .await
            .unwrap();

        let result = test
            .glue
            .manager
            .send_user_message("thr-live", "ping".into())
            .await;
        assert!(result.is_err());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let row = test
            .glue
            .store
            .get_thread("thr-live")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "running");
        assert!(matches!(
            test.glue.current_state(),
            ThreadState::Running { .. }
        ));
    }

    #[tokio::test]
    async fn current_agent_thread_prefers_live_running_thread() {
        let test = test_glue().await;
        let workspace = test._tmp.path().join("snapshot-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_root = std::fs::canonicalize(&workspace)
            .unwrap()
            .display()
            .to_string();
        test.glue
            .store
            .upsert_workspace(&workspace_root, 10)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO threads(thread_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES ('thr-snapshot', ?, 'codex', 'idle', 0, 10, 20)",
        )
        .bind(&workspace_root)
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        let state = ThreadState::Running {
            turn_started_at_ms: 99,
        };
        test.glue
            .manager
            .register_persisted_thread(
                "thr-snapshot".into(),
                workspace,
                minos_domain::AgentName::Codex,
                Some("thr-snapshot".into()),
                state.clone(),
                0,
            )
            .await
            .unwrap();

        let snapshot = test
            .glue
            .current_agent_thread()
            .await
            .unwrap()
            .expect("live thread snapshot");

        assert_eq!(snapshot.thread_id, "thr-snapshot");
        assert_eq!(snapshot.workspace_root, workspace_root);
        assert_eq!(snapshot.state, state);
    }

    #[tokio::test]
    async fn resume_thread_registers_persisted_thread_and_returns_workspace() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-r", "codex", 10, 20).await;
        sqlx::query(
            "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart', codex_session_id = 'thr-r' WHERE thread_id = 'thr-r'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();

        let response = test.glue.resume_thread("thr-r").await.unwrap();
        assert_eq!(response.session_id, "thr-r");
        assert_eq!(response.cwd, "/w");
        assert!(test.glue.manager.has_thread("thr-r").await);
    }

    #[tokio::test]
    async fn resume_thread_recovers_non_codex_provider_session_id_from_events() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-g", "gemini", 10, 20).await;
        sqlx::query(
            "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart', codex_session_id = 'thr-g', last_seq = 1 WHERE thread_id = 'thr-g'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO events(thread_id, seq, payload, ts_ms, source) VALUES (?, 1, ?, 25, 'live')",
        )
        .bind("thr-g")
        .bind(
            serde_json::to_vec(&serde_json::json!({
                "kind":"acp_notification",
                "params":{"sessionId":"gemini-provider-session"}
            }))
            .unwrap(),
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();

        let response = test.glue.resume_thread("thr-g").await.unwrap();

        assert_eq!(response.session_id, "thr-g");
        assert_eq!(
            test.glue
                .manager
                .thread_provider_session_id("thr-g")
                .await
                .as_deref(),
            Some("gemini-provider-session")
        );
    }
}
