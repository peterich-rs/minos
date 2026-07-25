use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use minos_agent_runtime::{
    AgentLaunchMode, AgentManager, AgentRuntimeConfig, InstanceCaps, ManagerEvent, RawIngest,
    SessionPolicies, SessionState,
};
use minos_chat_store::mcp_socket::{SocketRequest, SocketResponse};
use minos_codex_protocol::SkillsListResponse as CodexSkillsListResponse;
use minos_domain::{AgentName, MinosError};
use minos_protocol::{
    AgentDispatchRequest, AgentDispatchResponse, AgentLaunchMode as ProtoAgentLaunchMode,
    ApprovalDecisionRequest, CloseReason as ProtoCloseReason, CloseSessionRequest,
    GetSessionParams, GetSessionResponse, HostSkillError, HostSkillSummary, HostSkillsEntry,
    InterruptSessionRequest, ListHostSkillsRequest, ListHostSkillsResponse,
    ListHostWorkspacesRequest, ListHostWorkspacesResponse, ListSessionsParams,
    ListSessionsResponse, LocalConversationEvent, LocalIngestFrame, LocalManagerEvent,
    PauseReason as ProtoPauseReason, SendUserMessageRequest, SessionState as ProtoSessionState,
    SessionSummary, StartAgentRequest, StartAgentResponse, WriteHostSkillConfigRequest,
    WriteHostSkillConfigResponse,
};
use minos_ui_protocol::SessionEndReason;
use tokio::sync::{broadcast, watch};

use crate::store::event_writer::{provider_session_id_from_ingest, EventWriter};
use crate::store::{ChatMessageRow, ConversationRow, EventRow, LocalStore, SessionRow};
use crate::subscription::{AgentStateObserver, Subscription};
use crate::{ingest_coalescer::IngestCoalescer, ingest_sync::IngestSyncHandle};

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSessionSnapshot {
    pub session_id: String,
    pub workspace_root: String,
    pub state: SessionState,
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
    /// Local SQLite store. Owned so `start_agent` / `close_session` can keep
    /// the parent `sessions` / `workspaces` rows in sync with the in-memory
    /// `AgentManager`. Without these the events FK in §8.2 fails the
    /// moment codex emits its first ingest frame.
    store: Arc<LocalStore>,
    /// Watch channel mirroring the most recently observed session state. The
    /// legacy FFI surface exposes a single `state_stream()` shaped like the
    /// pre-Phase-C `AgentRuntime`. Multi-thread fan-out lands in C17.
    state_tx: Arc<watch::Sender<SessionState>>,
    state_rx: watch::Receiver<SessionState>,
    persisted_ingest_tx: broadcast::Sender<LocalIngestFrame>,
    local_manager_event_tx: broadcast::Sender<LocalManagerEvent>,
    local_conversation_event_tx: broadcast::Sender<LocalConversationEvent>,
    ingest_sync: Arc<StdMutex<Option<IngestSyncHandle>>>,
    /// Default workspace dir used when `start_agent` is invoked under the
    /// legacy surface (no workspace param). Resolved once at construction
    /// time.
    default_workspace: PathBuf,
}

impl AgentGlue {
    /// Construct a new glue and spawn the `RawIngest -> chunk -> local DB`
    /// bridge. Network upload is attached later with [`Self::set_ingest_sync`]
    /// after the relay client exists.
    #[must_use]
    pub fn new(
        workspace_root: PathBuf,
        subprocess_env: Arc<std::collections::HashMap<String, String>>,
        store: Arc<LocalStore>,
    ) -> Self {
        let mut cfg = AgentRuntimeConfig::new(workspace_root.clone());
        if let Err(error) = cfg.enable_default_mcp() {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %error,
                "failed to enable default MCP"
            );
        }
        let mcp_config = cfg.mcp.clone();
        cfg.subprocess_env = subprocess_env;
        #[cfg(feature = "test-support")]
        apply_test_ws_override(&mut cfg);
        let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let writer = Arc::new(EventWriter::spawn(store.clone()));
        let glue = Self::wire_with(manager.clone(), writer, store, workspace_root.clone());
        if let Some(mcp_config) = mcp_config {
            spawn_mcp_socket_handler(
                mcp_config,
                manager,
                glue.store.clone(),
                glue.local_conversation_event_tx.clone(),
                workspace_root,
            );
        }
        glue
    }

    /// Test-time / advanced constructor that accepts a pre-built manager and
    /// writer so unit tests can stub one or both.
    pub fn wire_with(
        manager: Arc<AgentManager>,
        writer: Arc<EventWriter>,
        store: Arc<LocalStore>,
        default_workspace: PathBuf,
    ) -> Self {
        // Spawn the bridge: every durable RawIngest from the manager is forwarded to
        // the EventWriter (which persists + broadcasts the corresponding
        // `Envelope::Ingest` outbound).
        //
        // Each ingest gets one info-level log line so the daemon log shows
        // the codex → host event stream at a glance. Pre-fix this slot was
        // the FK-error spam; post-fix the success path was silent and the
        // user couldn't tell whether codex was active. Volume is bounded
        // by codex's own emit rate (~tens/s/thread per spec §8.7).
        let (persisted_ingest_tx, _) = broadcast::channel(256);
        let (local_manager_event_tx, _) = broadcast::channel(256);
        let (local_conversation_event_tx, _) = broadcast::channel(256);
        let completion = crate::conversation_completion::ConversationCompletion::new(
            store.clone(),
            manager.clone(),
            default_workspace.clone(),
            local_conversation_event_tx.clone(),
        );
        let ingest_sync = Arc::new(StdMutex::new(None::<IngestSyncHandle>));
        let coalescer = IngestCoalescer::new(store.clone());
        let mut rx = manager.install_durable_ingest_stream();
        let writer_clone = writer.clone();
        let coalescer_clone = coalescer.clone();
        let ingest_sync_clone = ingest_sync.clone();
        let persisted_ingest_tx_clone = persisted_ingest_tx.clone();
        let completion_for_ingest = completion.clone();
        tokio::spawn(async move {
            while let Some(ingest) = rx.recv().await {
                let session_id = ingest.session_id.clone();
                let agent = ingest.agent;
                let ts_ms = ingest.ts_ms;
                let payload_bytes = ingest.body_len();
                let chunk = match coalescer_clone.coalesce(ingest).await {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        tracing::error!(
                            target: "minos_daemon::agent",
                            error = %error,
                            session_id = %session_id,
                            "failed to coalesce ingest event; event dropped",
                        );
                        continue;
                    }
                };
                let sync = ingest_sync_clone
                    .lock()
                    .ok()
                    .and_then(|guard| guard.clone());
                if let Some(sync) = sync {
                    sync.submit_live(chunk.clone()).await;
                }
                match writer_clone.write_chunk(chunk).await {
                    Ok(committed) => {
                        let seq = committed.seq;
                        let ui_events = committed.projection;
                        completion_for_ingest
                            .on_ingest_frame(&session_id, agent, &ui_events)
                            .await;
                        let _ = persisted_ingest_tx_clone.send(LocalIngestFrame {
                            session_id: session_id.clone(),
                            seq,
                            agent,
                            ui_events,
                            ts_ms,
                        });
                        tracing::info!(
                            target: "minos_daemon::agent",
                            session_id = %session_id,
                            seq,
                            bytes = payload_bytes,
                            "ingest event committed",
                        );
                    }
                    Err(e) => tracing::error!(
                        target: "minos_daemon::agent",
                        error = %e,
                        session_id = %session_id,
                        "EventWriter.write_chunk failed; event not persisted locally",
                    ),
                }
            }
        });

        let (state_tx, state_rx) = watch::channel(SessionState::Idle);
        let state_tx = Arc::new(state_tx);
        let mut manager_events = manager.manager_event_stream();
        let store_clone = store.clone();
        let state_tx_clone = state_tx.clone();
        let local_manager_event_tx_clone = local_manager_event_tx.clone();
        let completion_for_state = completion.clone();
        tokio::spawn(async move {
            loop {
                match manager_events.recv().await {
                    Ok(event) => {
                        let _ = local_manager_event_tx_clone.send(local_event_from_manager(&event));
                        match event {
                            ManagerEvent::SessionAdded {
                                session_id,
                                workspace,
                                agent,
                                parent_session_id,
                            } => {
                                let cwd = workspace.display().to_string();
                                let now_ms = current_unix_ms();
                                if let Err(e) = store_clone.upsert_workspace(&cwd, now_ms).await {
                                    tracing::warn!(
                                        target: "minos_daemon::agent",
                                        error = %e,
                                        session_id = %session_id,
                                        agent = %agent_label(agent),
                                        workspace = %cwd,
                                        "store.upsert_workspace failed for SessionAdded",
                                    );
                                }
                                if let Some(parent_session_id) = parent_session_id {
                                    persist_subagent_thread_parent_row(
                                        &store_clone,
                                        &session_id,
                                        &parent_session_id,
                                        &cwd,
                                        agent,
                                        now_ms,
                                    )
                                    .await;
                                }
                            }
                            ManagerEvent::SessionStateChanged {
                                session_id,
                                new,
                                at_ms,
                                ..
                            } => {
                                // DaemonRestart is owned by AgentGlue::shutdown's synchronous
                                // suspend_thread_for_daemon_restart (idle vs suspended +
                                // needs_continue). Persisting Suspended{DaemonRestart} here races
                                // that path and rebrands finished turns as Paused.
                                let skip_persist = matches!(
                                    &new,
                                    SessionState::Suspended {
                                        reason: minos_agent_runtime::PauseReason::DaemonRestart
                                    }
                                );
                                if !skip_persist {
                                    persist_runtime_state_inner(
                                        &store_clone,
                                        &session_id,
                                        &new,
                                        at_ms,
                                    )
                                    .await;
                                }
                                completion_for_state
                                    .on_session_state(&session_id, &new)
                                    .await;
                                let _ = state_tx_clone.send(new);
                            }
                            ManagerEvent::SessionClosed { session_id, reason } => {
                                let state = SessionState::Closed { reason };
                                let at_ms = current_unix_ms();
                                persist_runtime_state_inner(
                                    &store_clone,
                                    &session_id,
                                    &state,
                                    at_ms,
                                )
                                .await;
                                completion_for_state
                                    .on_session_state(&session_id, &state)
                                    .await;
                                let _ = state_tx_clone.send(state);
                            }
                            ManagerEvent::InstanceCrashed {
                                affected_threads,
                                reason,
                                ..
                            } => {
                                let state = SessionState::Suspended { reason };
                                let at_ms = current_unix_ms();
                                for session_id in affected_threads {
                                    persist_runtime_state_inner(
                                        &store_clone,
                                        &session_id,
                                        &state,
                                        at_ms,
                                    )
                                    .await;
                                    let _ = state_tx_clone.send(state.clone());
                                }
                            }
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
            persisted_ingest_tx,
            local_manager_event_tx,
            local_conversation_event_tx,
            ingest_sync,
            default_workspace,
        }
    }

    pub fn set_ingest_sync(&self, sync: IngestSyncHandle) {
        if let Ok(mut guard) = self.ingest_sync.lock() {
            *guard = Some(sync);
        }
    }

    pub fn store(&self) -> &Arc<LocalStore> {
        &self.store
    }

    pub async fn read_session_raw_history(
        &self,
        session_id: &str,
        from_seq: Option<u64>,
        limit: u32,
    ) -> Result<(Vec<minos_protocol::LocalIngestFrame>, Option<u64>), MinosError> {
        let row = self
            .store
            .get_session(session_id)
            .await
            .map_err(|e| map_store_error("read_session_raw_history", e))?
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
            .read_events(session_id, start, end)
            .await
            .map_err(|e| map_store_error("read_session_raw_history", e))?;
        let agent = parse_agent_label(&row.agent)?;
        let mut events = Vec::with_capacity(rows.len());
        for event in rows {
            let ui_events: Vec<minos_ui_protocol::UiEventMessage> =
                serde_json::from_slice(&event.projection_json).map_err(|e| {
                    MinosError::CodexProtocolError {
                        method: "read_session_raw_history".into(),
                        message: e.to_string(),
                    }
                })?;
            events.push(minos_protocol::LocalIngestFrame {
                session_id: session_id.to_owned(),
                seq: u64::try_from(event.seq.max(0)).unwrap_or(0),
                agent,
                ui_events,
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
        let launch = self
            .resolve_launch_options(
                req.agent,
                req.profile_id.as_deref(),
                req.model.clone(),
                req.reasoning_effort.clone(),
                req.instructions.clone(),
            )
            .await?;
        let outcome = self
            .manager
            .start_agent_with_policies(req.agent, workspace, None, launch)
            .await
            .map_err(map_anyhow)?;
        let cwd = outcome.cwd.display().to_string();
        self.persist_thread_parent_rows(
            &outcome.session_id,
            &cwd,
            req.agent,
            outcome.provider_session_id.as_deref(),
        )
        .await;

        // Legacy single-state mirror: emit Idle (not Running) because the
        // multi-thread manager keeps per-session state internally; the
        // single-channel mirror just signals "something is alive". The mobile
        // / Swift surfaces will switch to per-session state streams in C17/D.
        let _ = self.state_tx.send(SessionState::Idle);
        tracing::info!(
            target: "minos_daemon::agent",
            profile_id = req.profile_id.as_deref().unwrap_or(""),
            agent = %agent_label(req.agent),
            session_id = %outcome.session_id,
            "agent session started",
        );
        Ok(StartAgentResponse {
            session_id: outcome.session_id,
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
        let launch = self
            .resolve_launch_options(
                req.agent,
                req.profile_id.as_deref(),
                req.model.clone(),
                req.reasoning_effort.clone(),
                req.instructions.clone(),
            )
            .await?;
        let outcome = self
            .manager
            .start_agent_with_session_id_and_options(
                req.agent,
                workspace,
                session_id.clone(),
                None,
                launch,
            )
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

        let _ = self.state_tx.send(SessionState::Idle);
        tracing::info!(
            target: "minos_daemon::agent",
            profile_id = req.profile_id.as_deref().unwrap_or(""),
            agent = %agent_label(req.agent),
            session_id = %session_id,
            "agent session started with fixed session_id",
        );
        Ok(StartAgentResponse { session_id, cwd })
    }

    /// Resolve create-time launch options from optional profile + explicit fields.
    ///
    /// Precedence: explicit request fields > profile fields > None.
    /// When `profile_id` is set, `agent` must equal `profile.runtime_agent`.
    pub async fn resolve_launch_options(
        &self,
        agent: AgentName,
        profile_id: Option<&str>,
        model: Option<String>,
        reasoning_effort: Option<String>,
        instructions: Option<String>,
    ) -> Result<Option<minos_agent_runtime::AgentLaunchOptions>, MinosError> {
        resolve_launch_options(
            &self.store,
            agent,
            profile_id,
            model,
            reasoning_effort,
            instructions,
        )
        .await
        .map_err(|message| MinosError::CodexProtocolError {
            method: "resolve_launch_options".into(),
            message,
        })
    }

    pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError> {
        // User text always wins over a pending auto-continue: claim the flag
        // so open-time inject cannot race a second CONTINUE turn.
        if let Err(e) = self.store.take_needs_continue(&req.session_id).await {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                session_id = %req.session_id,
                "take_needs_continue failed before send_user_message",
            );
        }
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
            .resolve_approval(&req.request_id, &req.session_id, req.decision)
            .await
            .map_err(map_anyhow)
    }

    pub async fn respond_opencode_permission(
        &self,
        req: minos_protocol::RespondOpencodePermissionRequest,
    ) -> Result<(), MinosError> {
        self.manager
            .respond_opencode_permission(&req.session_id, &req.permission_id, &req.response)
            .await
            .map_err(map_anyhow)
    }

    pub async fn respond_opencode_question(
        &self,
        req: minos_protocol::RespondOpencodeQuestionRequest,
    ) -> Result<(), MinosError> {
        self.manager
            .respond_opencode_question(&req.session_id, &req.question_id, req.answers)
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
            model,
            reasoning_effort,
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
        let launch = minos_agent_runtime::AgentLaunchOptions::from_parts(model, reasoning_effort);

        let outcome = self
            .manager
            .dispatch_message_with_options(
                agent,
                resolve_workspace(&self.default_workspace, &workspace),
                session_id,
                text,
                policies,
                launch,
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
        session_id: &str,
        cwd: &str,
        agent: minos_domain::AgentName,
        provider_session_id: Option<&str>,
    ) {
        persist_thread_parent_rows_inner(
            &self.store,
            session_id,
            cwd,
            agent,
            provider_session_id,
            None,
        )
        .await;
    }

    async fn persist_thread_parent_rows_in_conversation(
        &self,
        session_id: &str,
        conversation_id: &str,
        cwd: &str,
        agent: minos_domain::AgentName,
        provider_session_id: Option<&str>,
    ) {
        persist_thread_parent_rows_inner(
            &self.store,
            session_id,
            cwd,
            agent,
            provider_session_id,
            Some(conversation_id),
        )
        .await;
    }

    async fn persist_current_provider_session_id(&self, session_id: &str) {
        let provider_session_id = self.manager.session_provider_session_id(session_id).await;
        if provider_session_id.is_none() {
            return;
        }
        if let Err(e) = self
            .store
            .update_session_provider_session_id(session_id, provider_session_id.as_deref())
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                session_id,
                "store.update_session_provider_session_id failed",
            );
        }
    }

    async fn resolve_provider_session_id(
        &self,
        row: &SessionRow,
        agent: minos_domain::AgentName,
    ) -> Result<Option<String>, MinosError> {
        if agent == minos_domain::AgentName::Codex {
            return Ok(row.provider_session_id.clone());
        }

        if let Some(session_id) = row
            .provider_session_id
            .as_deref()
            .filter(|session_id| *session_id != row.session_id)
        {
            return Ok(Some(session_id.to_string()));
        }

        self.latest_provider_session_id_from_events(row, agent)
            .await
    }

    async fn latest_provider_session_id_from_events(
        &self,
        row: &SessionRow,
        agent: minos_domain::AgentName,
    ) -> Result<Option<String>, MinosError> {
        let max_seq = u64::try_from(row.last_seq.max(0)).unwrap_or(0);
        if max_seq == 0 {
            return Ok(None);
        }

        let rows = self
            .store
            .read_events(&row.session_id, 1, max_seq)
            .await
            .map_err(|e| map_store_error("latest_provider_session_id_from_events", e))?;
        Ok(rows
            .iter()
            .rev()
            .find_map(|event| provider_session_id_from_event(&row.session_id, agent, event)))
    }

    pub async fn ensure_thread_registered(&self, session_id: &str) -> Result<(), MinosError> {
        if self.manager.has_thread(session_id).await {
            return Ok(());
        }
        let row = self
            .store
            .get_session(session_id)
            .await
            .map_err(|e| map_store_error("ensure_thread_registered", e))?
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        let state = row_state_to_runtime(&row)?;
        let agent = parse_agent_label(&row.agent)?;
        let provider_session_id = self.resolve_provider_session_id(&row, agent).await?;
        self.manager
            .register_persisted_thread(
                row.session_id.clone(),
                PathBuf::from(&row.workspace_root),
                agent,
                provider_session_id,
                row.parent_session_id.clone(),
                Some(row.conversation_id.clone()),
                state,
                u64::try_from(row.last_seq.max(0)).unwrap_or(u64::MAX),
            )
            .await
            .map_err(map_anyhow)
    }

    /// Register + provider reattach. When `auto_continue` is true and the
    /// store still has `needs_continue`, inject CONTINUE once (open path).
    /// Send paths must pass `auto_continue = false` so user text wins.
    pub async fn resume_session(
        &self,
        session_id: &str,
        auto_continue: bool,
    ) -> Result<StartAgentResponse, MinosError> {
        let row = self
            .store
            .get_session(session_id)
            .await
            .map_err(|e| map_store_error("resume_session", e))?
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        if matches!(
            row_state_to_runtime(&row)?,
            minos_agent_runtime::SessionState::Closed { .. }
        ) {
            return Err(MinosError::AgentSessionIdMismatch);
        }
        let agent = parse_agent_label(&row.agent)?;
        let provider_session_id = self.resolve_provider_session_id(&row, agent).await?;
        // Register as Suspended when DB says so so reattach can run; live
        // Idle/Running rows also register with their persisted state.
        let register_state = match row_state_to_runtime(&row)? {
            minos_agent_runtime::SessionState::Closed { reason } => {
                minos_agent_runtime::SessionState::Closed { reason }
            }
            // Prefer Suspended for rehydrate so reattach path is used even if
            // status was idle in an older partial write (defensive).
            other
                if matches!(
                    other,
                    minos_agent_runtime::SessionState::Idle
                        | minos_agent_runtime::SessionState::Running { .. }
                        | minos_agent_runtime::SessionState::Starting
                        | minos_agent_runtime::SessionState::Resuming
                ) && !self.manager.has_thread(session_id).await =>
            {
                // Not live yet after daemon restart — treat as suspended rehydrate.
                minos_agent_runtime::SessionState::Suspended {
                    reason: minos_agent_runtime::PauseReason::DaemonRestart,
                }
            }
            other => other,
        };
        self.manager
            .register_persisted_thread(
                row.session_id.clone(),
                PathBuf::from(&row.workspace_root),
                agent,
                provider_session_id,
                row.parent_session_id.clone(),
                Some(row.conversation_id.clone()),
                register_state,
                u64::try_from(row.last_seq.max(0)).unwrap_or(u64::MAX),
            )
            .await
            .map_err(map_anyhow)?;

        // Idle/Running already live → no-op. Suspended → provider reattach → Idle.
        // Provider spawn may fail (missing CLI / no fake server in unit tests);
        // keep the row registered so a later send can re-try reattach.
        if let Err(e) = self.manager.reattach_suspended_thread(session_id).await {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                session_id = %session_id,
                auto_continue,
                "reattach_suspended_thread failed; thread registered, reattach deferred",
            );
            if auto_continue {
                // Cannot continue without a live provider session.
                return Err(map_anyhow(e));
            }
        }

        if auto_continue {
            match self.store.take_needs_continue(session_id).await {
                Ok(true) => {
                    if let Err(e) = self.manager.inject_continue_prompt(session_id).await {
                        // Restore flag so a later open/send can retry.
                        let _ = self.store.set_needs_continue(session_id, true).await;
                        tracing::warn!(
                            target: "minos_daemon::agent",
                            error = %e,
                            session_id = %session_id,
                            "inject_continue_prompt failed; needs_continue restored",
                        );
                        return Err(map_anyhow(e));
                    }
                    self.persist_current_provider_session_id(session_id).await;
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %e,
                        session_id = %session_id,
                        "take_needs_continue failed during resume",
                    );
                }
            }
        }

        Ok(StartAgentResponse {
            session_id: row.session_id,
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

    pub async fn interrupt_session(&self, req: InterruptSessionRequest) -> Result<(), MinosError> {
        self.manager
            .interrupt_session(&req.session_id)
            .await
            .map_err(map_anyhow)
    }

    pub async fn close_session(&self, req: CloseSessionRequest) -> Result<(), MinosError> {
        self.manager
            .close_session(&req.session_id)
            .await
            .map_err(map_anyhow)?;

        // Mirror the in-memory transition into the local DB so the next
        // daemon start sees the session as `closed` instead of flipping it
        // to `suspended { daemon_restart }` via §8.6 startup recovery.
        // Logged on failure but non-fatal — the manager has already
        // released the thread.
        if let Err(e) = self
            .store
            .close_session_row(&req.session_id, "user_close", current_unix_ms())
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                session_id = %req.session_id,
                "store.close_session_row failed; row will look orphan on next restart",
            );
        }

        let _ = self.state_tx.send(SessionState::Idle);
        Ok(())
    }

    pub async fn delete_session(&self, req: CloseSessionRequest) -> Result<(), MinosError> {
        if let Err(e) = self.manager.close_session(&req.session_id).await {
            tracing::debug!(
                target: "minos_daemon::agent",
                error = %e,
                session_id = %req.session_id,
                "manager.close_session skipped during local delete",
            );
        }

        let deleted = self
            .store
            .delete_session(&req.session_id)
            .await
            .map_err(|e| map_store_error("delete_session", e))?;
        if deleted == 0 {
            return Err(MinosError::SessionNotFound {
                session_id: req.session_id,
            });
        }

        let _ = self.state_tx.send(SessionState::Idle);
        Ok(())
    }

    pub async fn list_sessions(
        &self,
        req: ListSessionsParams,
    ) -> Result<ListSessionsResponse, MinosError> {
        let agent_filter = req.agent.map(agent_label);
        let sessions = self
            .store
            .list_sessions(req.before_ts_ms, Some(req.limit), agent_filter)
            .await
            .map_err(|e| map_store_error("list_sessions", e))?
            .into_iter()
            .map(session_summary_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListSessionsResponse {
            sessions,
            next_before_ts_ms: None,
        })
    }

    pub async fn get_session(
        &self,
        req: GetSessionParams,
    ) -> Result<GetSessionResponse, MinosError> {
        let row = self
            .store
            .get_session(&req.session_id)
            .await
            .map_err(|e| map_store_error("get_session", e))?
            .ok_or(MinosError::AgentSessionIdMismatch)?;
        let live_state = self
            .manager
            .list_sessions()
            .await
            .into_iter()
            .find(|snapshot| snapshot.session_id == req.session_id)
            .map(|snapshot| state_to_proto(&snapshot.state));
        let thread = session_summary_from_row(row.clone())?;
        Ok(GetSessionResponse {
            thread,
            state: live_state.unwrap_or(row_state_to_proto(&row)?),
        })
    }

    pub async fn current_agent_session(&self) -> Result<Option<AgentSessionSnapshot>, MinosError> {
        let live_snapshots = self.manager.list_sessions().await;
        let rows = self
            .store
            .list_sessions(None, Some(500), None)
            .await
            .map_err(|e| map_store_error("current_agent_session", e))?;
        let row_by_thread = rows
            .iter()
            .map(|row| (row.session_id.as_str(), row))
            .collect::<HashMap<_, _>>();

        let mut live_candidates = live_snapshots
            .into_iter()
            .filter(|snapshot| !matches!(snapshot.state, SessionState::Closed { .. }))
            .map(|snapshot| {
                let last_activity_at = row_by_thread
                    .get(snapshot.session_id.as_str())
                    .map_or(0, |row| row.last_activity_at);
                (state_priority(&snapshot.state), last_activity_at, snapshot)
            })
            .collect::<Vec<_>>();
        live_candidates.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.session_id.as_str().cmp(right.2.session_id.as_str()))
        });
        if let Some((_, _, snapshot)) = live_candidates.into_iter().next() {
            return Ok(Some(AgentSessionSnapshot {
                session_id: snapshot.session_id,
                workspace_root: snapshot.workspace.display().to_string(),
                state: snapshot.state,
            }));
        }

        for row in rows {
            let state = row_state_to_runtime(&row)?;
            if matches!(
                state,
                SessionState::Starting
                    | SessionState::Idle
                    | SessionState::Running { .. }
                    | SessionState::Resuming
            ) {
                return Ok(Some(AgentSessionSnapshot {
                    session_id: row.session_id,
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
    pub fn current_state(&self) -> SessionState {
        self.state_rx.borrow().clone()
    }

    #[must_use]
    pub fn state_stream(&self) -> watch::Receiver<SessionState> {
        self.state_rx.clone()
    }

    #[must_use]
    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest> {
        self.manager.ingest_stream()
    }

    #[must_use]
    pub fn persisted_ingest_stream(&self) -> broadcast::Receiver<LocalIngestFrame> {
        self.persisted_ingest_tx.subscribe()
    }

    #[must_use]
    pub fn local_manager_event_stream(&self) -> broadcast::Receiver<LocalManagerEvent> {
        self.local_manager_event_tx.subscribe()
    }

    #[must_use]
    pub fn local_conversation_event_stream(&self) -> broadcast::Receiver<LocalConversationEvent> {
        self.local_conversation_event_tx.subscribe()
    }

    fn publish_conversation_message_appended(&self, conversation_id: &str, message_seq: i64) {
        publish_conversation_message_appended(
            &self.local_conversation_event_tx,
            conversation_id,
            message_seq,
        );
    }

    pub async fn shutdown(&self) -> Result<(), MinosError> {
        // Suspend (not close) every live thread so the next daemon start can
        // rehydrate sessions. Persist status synchronously — the manager event
        // bridge is async and races process exit.
        let snap = self.manager.list_sessions().await;
        let now_ms = current_unix_ms();
        for s in snap {
            match self.manager.suspend_for_daemon_stop(&s.session_id).await {
                Ok(needs_continue) => {
                    if let Err(e) = self
                        .store
                        .suspend_thread_for_daemon_restart(&s.session_id, needs_continue, now_ms)
                        .await
                    {
                        tracing::warn!(
                            target: "minos_daemon::agent",
                            error = %e,
                            session_id = %s.session_id,
                            "suspend_thread_for_daemon_restart failed during shutdown",
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %e,
                        session_id = %s.session_id,
                        "suspend_for_daemon_stop failed during shutdown",
                    );
                }
            }
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
            .create_project(
                &project_id,
                &req.name,
                &req.workspace_slug,
                Some(workspace_path.as_str()),
                now_ms,
            )
            .await
            .map_err(|e| map_store_error("create_project", e))?;

        // Also register the workspace in the workspaces table so sessions
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
            let thread_count = self
                .store
                .count_conversations_by_project(&row.project_id)
                .await
                .unwrap_or(0);
            projects.push(minos_protocol::ProjectSummary {
                project_id: row.project_id,
                name: row.name,
                workspace_path: Some(
                    row.workspace_path
                        .clone()
                        .filter(|p| !p.trim().is_empty())
                        .unwrap_or_else(|| {
                            project_workspace_dir(&self.default_workspace, &row.workspace_slug)
                        }),
                ),
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

    pub async fn create_conversation(
        &self,
        req: minos_protocol::CreateConversationParams,
    ) -> Result<minos_protocol::CreateConversationResponse, MinosError> {
        let project = self
            .store
            .get_project(&req.project_id)
            .await
            .map_err(|e| map_store_error("create_conversation.get_project", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "create_conversation".into(),
                message: format!("project not found: {}", req.project_id),
            })?;
        let title = req.title.trim();
        if title.is_empty() {
            return Err(MinosError::CodexProtocolError {
                method: "create_conversation".into(),
                message: "conversation title cannot be empty".into(),
            });
        }
        let priority = match req.priority.as_deref() {
            None => None,
            Some(p) => {
                let normalized = p.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "" => None,
                    "high" | "medium" | "low" => Some(normalized),
                    other => {
                        return Err(MinosError::CodexProtocolError {
                            method: "create_conversation".into(),
                            message: format!(
                                "invalid priority '{other}'; expected high|medium|low"
                            ),
                        });
                    }
                }
            }
        };
        let conversation_id = uuid::Uuid::new_v4().to_string();
        let now_ms = current_unix_ms();

        // Snapshot git context from the project workspace at create time.
        let (branch, worktree_path) = project
            .workspace_path
            .as_deref()
            .map(|p| crate::git_snapshot::detect_git_snapshot(std::path::Path::new(p)))
            .unwrap_or((None, None));
        let meta = crate::store::ConversationCreateMeta {
            priority,
            progress: Some("todo".into()),
            branch,
            worktree_path,
        };

        // Normalize + validate roster up front (membership gates @mention / start).
        let mut member_agents: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for raw in &req.agents {
            let label = raw.trim();
            if label.is_empty() || !seen.insert(label.to_ascii_lowercase()) {
                continue;
            }
            let agent = parse_agent_label(label).map_err(|e| MinosError::CodexProtocolError {
                method: "create_conversation".into(),
                message: format!("invalid agent member '{label}': {e}"),
            })?;
            member_agents.push(agent_label(agent).to_owned());
        }

        self.store
            .create_conversation_with_meta(
                &conversation_id,
                &project.project_id,
                title,
                now_ms,
                &meta,
            )
            .await
            .map_err(|e| map_store_error("create_conversation", e))?;
        self.store
            .set_conversation_agent_members(&conversation_id, &member_agents, now_ms)
            .await
            .map_err(|e| map_store_error("create_conversation.set_members", e))?;
        tracing::info!(
            target: "minos_daemon::agent",
            project_id = %project.project_id,
            conversation_id = %conversation_id,
            branch = ?meta.branch,
            agent_count = member_agents.len(),
            "conversation created",
        );
        let row = self
            .store
            .get_conversation(&conversation_id)
            .await
            .map_err(|e| map_store_error("create_conversation.reload", e))?
            .expect("conversation inserted above");
        let participating_agents = member_agents
            .iter()
            .map(|a| parse_agent_label(a))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::CreateConversationResponse {
            conversation: conversation_summary_from_row(row, participating_agents)?,
        })
    }

    pub async fn update_conversation(
        &self,
        req: minos_protocol::UpdateConversationParams,
    ) -> Result<minos_protocol::UpdateConversationResponse, MinosError> {
        let existing = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("update_conversation.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "update_conversation".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;

        let title = match req.title.as_deref() {
            Some(t) => {
                let trimmed = t.trim();
                if trimmed.is_empty() {
                    return Err(MinosError::CodexProtocolError {
                        method: "update_conversation".into(),
                        message: "conversation title cannot be empty".into(),
                    });
                }
                Some(trimmed.to_owned())
            }
            None => None,
        };

        let priority_patch: Option<Option<&str>> = match req.priority.as_deref() {
            None => None,
            Some("") => Some(None),
            Some(p) => {
                let normalized = p.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "high" | "medium" | "low" => Some(Some(match normalized.as_str() {
                        "high" => "high",
                        "medium" => "medium",
                        "low" => "low",
                        _ => unreachable!(),
                    })),
                    other => {
                        return Err(MinosError::CodexProtocolError {
                            method: "update_conversation".into(),
                            message: format!(
                                "invalid priority '{other}'; expected high|medium|low or empty"
                            ),
                        });
                    }
                }
            }
        };

        let progress_patch: Option<&str> = match req.progress.as_deref() {
            None => None,
            Some(p) => {
                let normalized = p.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "todo" | "in_progress" | "in_review" | "done" => {
                        Some(match normalized.as_str() {
                            "todo" => "todo",
                            "in_progress" => "in_progress",
                            "in_review" => "in_review",
                            "done" => "done",
                            _ => unreachable!(),
                        })
                    }
                    other => {
                        return Err(MinosError::CodexProtocolError {
                            method: "update_conversation".into(),
                            message: format!(
                                "invalid progress '{other}'; expected todo|in_progress|in_review|done"
                            ),
                        });
                    }
                }
            }
        };

        if title.is_none() && priority_patch.is_none() && progress_patch.is_none() {
            return Ok(minos_protocol::UpdateConversationResponse {
                conversation: conversation_summary_from_row(existing, Vec::new())?,
            });
        }

        let now_ms = current_unix_ms();
        self.store
            .update_conversation_fields(
                &req.conversation_id,
                title.as_deref(),
                priority_patch,
                progress_patch,
                now_ms,
            )
            .await
            .map_err(|e| map_store_error("update_conversation", e))?;

        let row = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("update_conversation.reload", e))?
            .expect("conversation updated above");
        let agents = self
            .store
            .list_agents_for_conversations(&[req.conversation_id.clone()])
            .await
            .map_err(|e| map_store_error("update_conversation.agents", e))?;
        let participating_agents = agents
            .get(&req.conversation_id)
            .into_iter()
            .flatten()
            .map(|agent| parse_agent_label(agent))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::UpdateConversationResponse {
            conversation: conversation_summary_from_row(row, participating_agents)?,
        })
    }

    /// Remove a runtime agent from the conversation roster and tear down its
    /// live work: close open sessions and cancel running teamwork delegations
    /// that involve the agent as source or target.
    pub async fn remove_conversation_agent(
        &self,
        req: minos_protocol::RemoveConversationAgentParams,
    ) -> Result<minos_protocol::RemoveConversationAgentResponse, MinosError> {
        let _existing = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("remove_conversation_agent.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "remove_conversation_agent".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;

        let agent =
            parse_agent_label(req.agent.trim()).map_err(|e| MinosError::CodexProtocolError {
                method: "remove_conversation_agent".into(),
                message: format!("invalid agent '{}': {e}", req.agent),
            })?;
        let agent_label = agent_label(agent).to_owned();

        let removed = self
            .store
            .remove_conversation_agent_member(&req.conversation_id, &agent_label)
            .await
            .map_err(|e| map_store_error("remove_conversation_agent.remove", e))?;
        if !removed {
            return Err(MinosError::CodexProtocolError {
                method: "remove_conversation_agent".into(),
                message: format!(
                    "agent '{agent_label}' is not a member of conversation {}",
                    req.conversation_id
                ),
            });
        }

        let now_ms = current_unix_ms();
        let session_rows = self
            .store
            .list_sessions_by_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("remove_conversation_agent.list_sessions", e))?;
        let mut closed_session_ids = Vec::new();
        for row in session_rows {
            if row.agent != agent_label || row.status == "closed" {
                continue;
            }
            let session_id = row.session_id.clone();
            if let Err(error) = self.manager.close_session(&session_id).await {
                tracing::debug!(
                    target: "minos_daemon::agent",
                    error = %error,
                    session_id = %session_id,
                    "manager.close_session during roster remove",
                );
            }
            if let Err(error) = self
                .store
                .close_session_row(&session_id, "roster_removed", now_ms)
                .await
            {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %error,
                    session_id = %session_id,
                    "store.close_session_row failed during roster remove",
                );
            }
            closed_session_ids.push(session_id);
        }

        let mut cancelled_delegation_ids = Vec::new();
        match minos_chat_store::TeamworkStore::open(self.store.db_path()).await {
            Ok(teamwork) => {
                match teamwork
                    .list_running_delegations_involving_agent(&req.conversation_id, agent)
                    .await
                {
                    Ok(running) => {
                        for delegation in running {
                            match teamwork
                                .cancel_delegation(
                                    &req.conversation_id,
                                    &delegation.delegation_id,
                                    Some(format!(
                                        "agent '{agent_label}' removed from conversation roster"
                                    )),
                                )
                                .await
                            {
                                Ok(cancelled) => {
                                    if let Some(session_id) = cancelled.session_id.as_deref() {
                                        let _ = self.manager.interrupt_session(session_id).await;
                                    }
                                    cancelled_delegation_ids.push(cancelled.delegation_id);
                                }
                                Err(error) => {
                                    tracing::debug!(
                                        target: "minos_daemon::agent",
                                        error = %error,
                                        delegation_id = %delegation.delegation_id,
                                        "cancel_delegation during roster remove skipped",
                                    );
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "minos_daemon::agent",
                            error = %error,
                            conversation_id = %req.conversation_id,
                            agent = %agent_label,
                            "list_running_delegations_involving_agent failed during roster remove",
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %error,
                    "open teamwork store failed during roster remove",
                );
            }
        }

        tracing::info!(
            target: "minos_daemon::agent",
            conversation_id = %req.conversation_id,
            agent = %agent_label,
            closed_sessions = closed_session_ids.len(),
            cancelled_delegations = cancelled_delegation_ids.len(),
            "removed conversation agent from roster",
        );

        let row = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("remove_conversation_agent.reload", e))?
            .expect("conversation exists above");
        let agents = self
            .store
            .list_agents_for_conversations(&[req.conversation_id.clone()])
            .await
            .map_err(|e| map_store_error("remove_conversation_agent.agents", e))?;
        let participating_agents = agents
            .get(&req.conversation_id)
            .into_iter()
            .flatten()
            .map(|agent| parse_agent_label(agent))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::RemoveConversationAgentResponse {
            conversation: conversation_summary_from_row(row, participating_agents)?,
            closed_session_ids,
            cancelled_delegation_ids,
        })
    }

    pub async fn list_conversations(
        &self,
        req: minos_protocol::ListConversationsParams,
    ) -> Result<minos_protocol::ListConversationsResponse, MinosError> {
        let rows = self
            .store
            .list_conversations_by_project(&req.project_id, req.before_updated_at_ms, req.limit)
            .await
            .map_err(|e| map_store_error("list_conversations", e))?;
        let ids = rows
            .iter()
            .map(|row| row.conversation_id.clone())
            .collect::<Vec<_>>();
        let agents = self
            .store
            .list_agents_for_conversations(&ids)
            .await
            .map_err(|e| map_store_error("list_agents_for_conversations", e))?;
        let conversations = rows
            .into_iter()
            .map(|row| {
                let participating_agents = agents
                    .get(&row.conversation_id)
                    .into_iter()
                    .flatten()
                    .map(|agent| parse_agent_label(agent))
                    .collect::<Result<Vec<_>, _>>()?;
                conversation_summary_from_row(row, participating_agents)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::ListConversationsResponse { conversations })
    }

    pub async fn list_conversation_messages(
        &self,
        req: minos_protocol::ListConversationMessagesParams,
    ) -> Result<minos_protocol::ListConversationMessagesResponse, MinosError> {
        let requested_limit = req.limit.unwrap_or(100).min(500);
        let rows = self
            .store
            .list_conversation_messages(
                &req.conversation_id,
                req.before_seq,
                Some(requested_limit.saturating_add(1)),
            )
            .await
            .map_err(|e| map_store_error("list_conversation_messages", e))?;
        let has_more = rows.len() > requested_limit as usize;
        let page_rows: Vec<ChatMessageRow> =
            rows.into_iter().take(requested_limit as usize).collect();
        let message_ids: Vec<String> = page_rows.iter().map(|r| r.message_id.clone()).collect();
        let reaction_rows = self
            .store
            .list_reactions_for_messages(&message_ids)
            .await
            .map_err(|e| map_store_error("list_conversation_messages.reactions", e))?;
        let reactions_by_message = aggregate_reactions_by_message(reaction_rows);
        let messages = page_rows
            .into_iter()
            .map(|row| {
                let reactions = reactions_by_message
                    .get(&row.message_id)
                    .cloned()
                    .unwrap_or_default();
                local_conversation_message_from_row(row, reactions)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::ListConversationMessagesResponse { messages, has_more })
    }

    pub async fn toggle_conversation_message_reaction(
        &self,
        req: minos_protocol::ToggleConversationMessageReactionParams,
    ) -> Result<minos_protocol::ToggleConversationMessageReactionResponse, MinosError> {
        let emoji = req.emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 32 {
            return Err(MinosError::CodexProtocolError {
                method: "toggle_conversation_message_reaction".into(),
                message: "emoji must be 1..=32 characters".into(),
            });
        }
        let reaction_id = format!("rx-{}", uuid::Uuid::new_v4());
        let now_ms = current_unix_ms();
        let (conversation_id, added) = self
            .store
            .toggle_local_message_reaction(&req.message_id, emoji, &reaction_id, now_ms)
            .await
            .map_err(|e| map_store_error("toggle_conversation_message_reaction", e))?;
        let reaction_rows = self
            .store
            .list_reactions_for_messages(&[req.message_id.clone()])
            .await
            .map_err(|e| map_store_error("toggle_conversation_message_reaction.list", e))?;
        let reactions = aggregate_reactions_by_message(reaction_rows)
            .remove(&req.message_id)
            .unwrap_or_default();
        tracing::info!(
            target: "minos_daemon::agent",
            conversation_id = %conversation_id,
            message_id = %req.message_id,
            emoji = %emoji,
            added,
            reaction_count = reactions.len(),
            "toggled conversation message reaction",
        );
        let _ = self.local_conversation_event_tx.send(
            LocalConversationEvent::ConversationReactionToggled {
                conversation_id: conversation_id.clone(),
                message_id: req.message_id.clone(),
                reactions: reactions.clone(),
            },
        );
        Ok(minos_protocol::ToggleConversationMessageReactionResponse {
            message_id: req.message_id,
            conversation_id,
            reactions,
        })
    }

    pub async fn list_conversation_agent_sessions(
        &self,
        req: minos_protocol::ListConversationAgentSessionsParams,
    ) -> Result<minos_protocol::ListConversationAgentSessionsResponse, MinosError> {
        let live_states: HashMap<String, ProtoSessionState> = self
            .manager
            .list_sessions()
            .await
            .into_iter()
            .map(|snapshot| (snapshot.session_id.clone(), state_to_proto(&snapshot.state)))
            .collect();
        let sessions = self
            .store
            .list_sessions_by_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("list_conversation_agent_sessions", e))?
            .into_iter()
            .map(|row| {
                let mut summary = session_summary_from_row(row.clone())?;
                if let Some(state) = live_states.get(&summary.session_id) {
                    summary.state = state.clone();
                }
                Ok(summary)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::ListConversationAgentSessionsResponse { sessions })
    }

    pub async fn append_conversation_message(
        &self,
        req: minos_protocol::AppendConversationMessageParams,
    ) -> Result<minos_protocol::AppendConversationMessageResponse, MinosError> {
        let now_ms = current_unix_ms();
        let agent = req.agent.map(agent_label);
        let mentions_json = serde_json::to_string(&req.mentions).unwrap_or_else(|_| "[]".into());
        let message_seq = self
            .store
            .upsert_conversation_message(
                &req.conversation_id,
                &req.message_id,
                req.session_id.as_deref(),
                &req.sender_role,
                agent,
                &req.body,
                now_ms,
                req.reply_to_message_id.as_deref(),
                req.delegation_id.as_deref(),
                &mentions_json,
            )
            .await
            .map_err(|e| map_store_error("append_conversation_message", e))?;
        self.publish_conversation_message_appended(&req.conversation_id, message_seq);
        Ok(minos_protocol::AppendConversationMessageResponse { message_seq })
    }

    pub async fn start_agent_in_conversation(
        &self,
        req: minos_protocol::StartAgentInConversationRequest,
    ) -> Result<StartAgentResponse, MinosError> {
        let conversation = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("start_agent_in_conversation.get_conversation", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "start_agent_in_conversation".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;
        let project = self
            .store
            .get_project(&conversation.project_id)
            .await
            .map_err(|e| map_store_error("start_agent_in_conversation.get_project", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "start_agent_in_conversation".into(),
                message: format!("project not found: {}", conversation.project_id),
            })?;
        let agent_name = agent_label(req.agent);
        let is_member = self
            .store
            .is_conversation_agent_member(&req.conversation_id, agent_name)
            .await
            .map_err(|e| map_store_error("start_agent_in_conversation.is_member", e))?;
        if !is_member {
            return Err(MinosError::CodexProtocolError {
                method: "start_agent_in_conversation".into(),
                message: format!(
                    "agent '{agent_name}' is not a member of this conversation; \
                     add it when creating the conversation"
                ),
            });
        }
        let workspace = if req.workspace.trim().is_empty() {
            project
                .workspace_path
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    self.default_workspace
                        .parent()
                        .unwrap_or(&self.default_workspace)
                        .join("workspaces")
                        .join(project.workspace_slug)
                })
        } else {
            PathBuf::from(req.workspace.trim())
        };
        if let Err(e) = std::fs::create_dir_all(&workspace) {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                path = %workspace.display(),
                conversation_id = %req.conversation_id,
                "failed to create conversation workspace directory",
            );
        }
        let launch = self
            .resolve_launch_options(
                req.agent,
                req.profile_id.as_deref(),
                req.model.clone(),
                req.reasoning_effort.clone(),
                req.instructions.clone(),
            )
            .await?;
        let outcome = self
            .manager
            .start_agent_in_conversation_with_options(
                req.agent,
                workspace,
                req.conversation_id.clone(),
                launch,
            )
            .await
            .map_err(map_anyhow)?;
        let cwd = outcome.cwd.display().to_string();
        self.persist_thread_parent_rows_in_conversation(
            &outcome.session_id,
            &req.conversation_id,
            &cwd,
            req.agent,
            outcome.provider_session_id.as_deref(),
        )
        .await;
        // First real work: promote workflow progress out of todo.
        if let Err(e) = self
            .store
            .promote_conversation_in_progress_if_todo(&req.conversation_id, current_unix_ms())
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                conversation_id = %req.conversation_id,
                "failed to promote conversation progress to in_progress",
            );
        }
        let _ = self.state_tx.send(SessionState::Idle);
        tracing::info!(
            target: "minos_daemon::agent",
            conversation_id = %req.conversation_id,
            session_id = %outcome.session_id,
            profile_id = req.profile_id.as_deref().unwrap_or(""),
            agent = %agent_label(req.agent),
            workspace = %cwd,
            "agent session started in conversation",
        );
        Ok(StartAgentResponse {
            session_id: outcome.session_id,
            cwd,
        })
    }

    pub async fn list_agent_profiles(
        &self,
    ) -> Result<minos_protocol::ListAgentProfilesResponse, MinosError> {
        let rows = self
            .store
            .list_agent_profiles()
            .await
            .map_err(|e| map_store_error("list_agent_profiles", e))?;
        Ok(minos_protocol::ListAgentProfilesResponse {
            profiles: rows
                .into_iter()
                .filter_map(profile_row_to_summary)
                .collect(),
        })
    }

    pub async fn create_agent_profile(
        &self,
        req: minos_protocol::CreateAgentProfileRequest,
    ) -> Result<minos_protocol::AgentProfileSummary, MinosError> {
        let name = validate_agent_profile_name(&req.name, "create_agent_profile")?;
        let model = req.model.trim();
        if model.is_empty() {
            return Err(MinosError::CodexProtocolError {
                method: "create_agent_profile".into(),
                message: "model is required".into(),
            });
        }
        let id = format!("profile-{}", uuid::Uuid::new_v4());
        let now = current_unix_ms();
        let row = self
            .store
            .create_agent_profile(
                &id,
                name,
                req.description.trim(),
                req.runtime_agent.bin_name(),
                model,
                req.reasoning_effort.trim(),
                req.instructions.trim(),
                now,
            )
            .await
            .map_err(|e| map_store_error("create_agent_profile", e))?;
        profile_row_to_summary(row).ok_or_else(|| MinosError::CodexProtocolError {
            method: "create_agent_profile".into(),
            message: "invalid runtime_agent after insert".into(),
        })
    }

    pub async fn update_agent_profile(
        &self,
        req: minos_protocol::UpdateAgentProfileRequest,
    ) -> Result<minos_protocol::AgentProfileSummary, MinosError> {
        let name = validate_agent_profile_name(&req.name, "update_agent_profile")?;
        let row = self
            .store
            .update_agent_profile(
                &req.id,
                name,
                req.description.trim(),
                req.instructions.trim(),
                current_unix_ms(),
            )
            .await
            .map_err(|e| map_store_error("update_agent_profile", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "update_agent_profile".into(),
                message: format!("profile not found: {}", req.id),
            })?;
        profile_row_to_summary(row).ok_or_else(|| MinosError::CodexProtocolError {
            method: "update_agent_profile".into(),
            message: "invalid runtime_agent".into(),
        })
    }

    pub async fn delete_agent_profile(
        &self,
        req: minos_protocol::DeleteAgentProfileRequest,
    ) -> Result<(), MinosError> {
        let ok = self
            .store
            .delete_agent_profile(&req.id)
            .await
            .map_err(|e| map_store_error("delete_agent_profile", e))?;
        if !ok {
            return Err(MinosError::CodexProtocolError {
                method: "delete_agent_profile".into(),
                message: format!("profile not found: {}", req.id),
            });
        }
        Ok(())
    }
}

/// Profile display names double as `@Name` mention tokens.
/// Reject characters that break single-token `@` routing: whitespace, `#` (session form), `@`.
fn validate_agent_profile_name<'a>(name: &'a str, method: &str) -> Result<&'a str, MinosError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(MinosError::CodexProtocolError {
            method: method.into(),
            message: "name is required".into(),
        });
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || c == '#' || c == '@')
    {
        return Err(MinosError::CodexProtocolError {
            method: method.into(),
            message: "name cannot contain whitespace, #, or @".into(),
        });
    }
    Ok(name)
}

fn profile_row_to_summary(
    row: crate::store::AgentProfileRow,
) -> Option<minos_protocol::AgentProfileSummary> {
    let runtime_agent = parse_agent_label(&row.runtime_agent).ok()?;
    Some(minos_protocol::AgentProfileSummary {
        id: row.id,
        name: row.name,
        description: row.description,
        runtime_agent,
        model: row.model,
        reasoning_effort: row.reasoning_effort,
        instructions: row.instructions,
        created_at_ms: row.created_at_ms,
        updated_at_ms: row.updated_at_ms,
    })
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

fn session_summary_from_row(row: crate::store::SessionRow) -> Result<SessionSummary, MinosError> {
    let end_reason = row_end_reason(&row);
    let state = row_state_to_proto(&row)?;
    Ok(SessionSummary {
        session_id: row.session_id,
        agent: parse_agent_label(&row.agent)?,
        title: None,
        first_ts_ms: row.started_at,
        last_ts_ms: row.last_activity_at,
        message_count: u32::try_from(row.last_seq.max(0)).unwrap_or(u32::MAX),
        ended_at_ms: row.ended_at,
        end_reason,
        parent_session_id: row.parent_session_id,
        state,
        needs_continue: row.needs_continue,
    })
}

fn conversation_summary_from_row(
    row: ConversationRow,
    participating_agents: Vec<AgentName>,
) -> Result<minos_protocol::LocalConversationSummary, MinosError> {
    Ok(minos_protocol::LocalConversationSummary {
        conversation_id: row.conversation_id,
        project_id: row.project_id,
        title: row.title,
        last_message_preview: row.last_message_preview,
        created_at_ms: row.created_at_ms,
        updated_at_ms: row.updated_at_ms,
        message_count: u32::try_from(row.message_count.max(0)).unwrap_or(u32::MAX),
        agent_session_count: u32::try_from(row.agent_session_count.max(0)).unwrap_or(u32::MAX),
        participating_agents,
        priority: row.priority.filter(|p| !p.is_empty()),
        progress: if row.progress.is_empty() {
            "todo".into()
        } else {
            row.progress
        },
        branch: row.branch.filter(|b| !b.is_empty()),
        worktree_path: row.worktree_path.filter(|w| !w.is_empty()),
        running_count: u32::try_from(row.running_count.max(0)).unwrap_or(u32::MAX),
        needs_attention_count: u32::try_from(row.needs_attention_count.max(0)).unwrap_or(u32::MAX),
    })
}

fn local_conversation_message_from_row(
    row: ChatMessageRow,
    reactions: Vec<minos_protocol::LocalReactionGroup>,
) -> Result<minos_protocol::LocalConversationMessage, MinosError> {
    let mentions = if row.mentions_json.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&row.mentions_json).map_err(|error| {
            MinosError::CodexProtocolError {
                method: "list_conversation_messages".into(),
                message: format!("invalid mentions_json: {error}"),
            }
        })?
    };
    Ok(minos_protocol::LocalConversationMessage {
        message_seq: row.message_seq,
        message_id: row.message_id,
        conversation_id: row.conversation_id,
        session_id: row.session_id,
        created_at_ms: row.created_at_ms,
        sender_role: row.sender_role,
        agent: row.agent.as_deref().map(parse_agent_label).transpose()?,
        body: row.body,
        reply_to_message_id: row.reply_to_message_id,
        delegation_id: row.delegation_id,
        mentions,
        reactions,
    })
}

/// Group raw reaction rows into per-message emoji aggregates.
fn aggregate_reactions_by_message(
    rows: Vec<crate::store::ChatMessageReactionRow>,
) -> HashMap<String, Vec<minos_protocol::LocalReactionGroup>> {
    use std::collections::BTreeMap;
    // message_id -> emoji -> actors (ordered by first seen)
    let mut by_message: HashMap<String, BTreeMap<String, Vec<minos_protocol::LocalReactionActor>>> =
        HashMap::new();
    for row in rows {
        let actors = by_message
            .entry(row.message_id)
            .or_default()
            .entry(row.emoji)
            .or_default();
        actors.push(minos_protocol::LocalReactionActor {
            actor_id: row.actor_id,
            actor_kind: row.actor_kind,
            display_name: row.display_name,
        });
    }
    let mut out = HashMap::with_capacity(by_message.len());
    for (message_id, emoji_map) in by_message {
        let mut groups: Vec<minos_protocol::LocalReactionGroup> = emoji_map
            .into_iter()
            .map(|(emoji, actors)| {
                let reacted_by_me = actors
                    .iter()
                    .any(|a| a.actor_id == minos_protocol::LOCAL_REACTION_ACTOR_ID);
                let count = u32::try_from(actors.len()).unwrap_or(u32::MAX);
                minos_protocol::LocalReactionGroup {
                    emoji,
                    count,
                    reacted_by_me,
                    actors,
                }
            })
            .collect();
        // Stable display order: higher count first, then emoji.
        groups.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.emoji.cmp(&b.emoji)));
        out.insert(message_id, groups);
    }
    out
}

fn row_end_reason(row: &crate::store::SessionRow) -> Option<SessionEndReason> {
    match row.last_close_reason.as_deref() {
        Some("user_close") => Some(SessionEndReason::UserStopped),
        Some("terminal_error") => Some(SessionEndReason::Crashed {
            message: "terminal_error".into(),
        }),
        Some(other) => Some(SessionEndReason::Crashed {
            message: other.to_string(),
        }),
        None => None,
    }
}

pub(crate) fn row_state_to_proto(
    row: &crate::store::SessionRow,
) -> Result<ProtoSessionState, MinosError> {
    match row.status.as_str() {
        "starting" => Ok(ProtoSessionState::Starting),
        "idle" => Ok(ProtoSessionState::Idle),
        "running" => Ok(ProtoSessionState::Running {
            turn_started_at_ms: row.last_activity_at,
        }),
        "resuming" => Ok(ProtoSessionState::Resuming),
        "suspended" => Ok(ProtoSessionState::Suspended {
            reason: parse_pause_reason(row.last_pause_reason.as_deref())?,
        }),
        "closed" => Ok(ProtoSessionState::Closed {
            reason: parse_close_reason(row.last_close_reason.as_deref())?,
        }),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.thread_status".into(),
            message: format!("unknown persisted session status: {other}"),
        }),
    }
}

fn row_state_to_runtime(
    row: &crate::store::SessionRow,
) -> Result<minos_agent_runtime::SessionState, MinosError> {
    match row.status.as_str() {
        "starting" => Ok(minos_agent_runtime::SessionState::Starting),
        "idle" => Ok(minos_agent_runtime::SessionState::Idle),
        "running" => Ok(minos_agent_runtime::SessionState::Running {
            turn_started_at_ms: row.last_activity_at,
        }),
        "resuming" => Ok(minos_agent_runtime::SessionState::Resuming),
        "suspended" => Ok(minos_agent_runtime::SessionState::Suspended {
            reason: parse_pause_reason_runtime(row.last_pause_reason.as_deref())?,
        }),
        "closed" => Ok(minos_agent_runtime::SessionState::Closed {
            reason: parse_close_reason_runtime(row.last_close_reason.as_deref())?,
        }),
        other => Err(MinosError::CodexProtocolError {
            method: "local_store.thread_status".into(),
            message: format!("unknown persisted session status: {other}"),
        }),
    }
}

pub(crate) fn parse_agent_label(agent: &str) -> Result<minos_domain::AgentName, MinosError> {
    match agent {
        "codex" => Ok(minos_domain::AgentName::Codex),
        "claude" => Ok(minos_domain::AgentName::Claude),
        "gemini" => Ok(minos_domain::AgentName::Gemini),
        "opencode" => Ok(minos_domain::AgentName::Opencode),
        "grok" => Ok(minos_domain::AgentName::Grok),
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
        minos_domain::AgentName::Grok => "grok",
    }
}

/// Treat missing / blank strings as unset for launch-option merge.
fn nonempty_opt(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_owned())
        }
    })
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

fn state_priority(state: &SessionState) -> u8 {
    match state {
        SessionState::Running { .. } => 0,
        SessionState::Starting | SessionState::Resuming => 1,
        SessionState::Idle => 2,
        SessionState::Suspended { .. } => 3,
        SessionState::Closed { .. } => 4,
    }
}

async fn persist_runtime_state_inner(
    store: &LocalStore,
    session_id: &str,
    state: &SessionState,
    at_ms: i64,
) {
    let (status, pause_reason, close_reason, ended_at) = runtime_state_columns(state, at_ms);
    match store
        .update_session_status(
            session_id,
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
            session_id,
            status,
            "store.update_session_status affected no rows",
        ),
        Ok(_) => {}
        Err(error) => tracing::warn!(
            target: "minos_daemon::agent",
            error = %error,
            session_id,
            status,
            "store.update_session_status failed",
        ),
    }
}

fn runtime_state_columns(
    state: &SessionState,
    at_ms: i64,
) -> (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<i64>,
) {
    match state {
        SessionState::Starting => ("starting", None, None, None),
        SessionState::Idle => ("idle", None, None, None),
        SessionState::Running { .. } => ("running", None, None, None),
        SessionState::Suspended { reason } => (
            "suspended",
            Some(runtime_pause_reason_label(reason)),
            None,
            None,
        ),
        SessionState::Resuming => ("resuming", None, None, None),
        SessionState::Closed { reason } => (
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

fn spawn_mcp_socket_handler(
    mcp_config: minos_agent_runtime::config::McpConfig,
    manager: Arc<AgentManager>,
    store: Arc<LocalStore>,
    local_conversation_event_tx: broadcast::Sender<LocalConversationEvent>,
    default_workspace: PathBuf,
) {
    let socket_path = mcp_config.socket_path.clone();
    let db_path = mcp_config.db_path.clone();
    let callback: minos_chat_store::mcp_handler::ToolCallback = Arc::new(move |request| {
        let manager = manager.clone();
        let store = store.clone();
        let db_path = db_path.clone();
        let default_workspace = default_workspace.clone();
        let local_conversation_event_tx = local_conversation_event_tx.clone();
        tokio::spawn(async move {
            handle_daemon_mcp_request(
                manager,
                store,
                db_path,
                default_workspace,
                local_conversation_event_tx,
                request,
            )
            .await
        })
    });
    tokio::spawn(async move {
        let handler = minos_chat_store::mcp_handler::McpSocketHandler::new(socket_path, callback);
        if let Err(error) = handler.run().await {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %error,
                "MCP socket handler stopped"
            );
        }
    });
}

async fn handle_daemon_mcp_request(
    manager: Arc<AgentManager>,
    store: Arc<LocalStore>,
    db_path: PathBuf,
    default_workspace: PathBuf,
    local_conversation_event_tx: broadcast::Sender<LocalConversationEvent>,
    request: SocketRequest,
) -> anyhow::Result<SocketResponse> {
    match request {
        SocketRequest::Ping => Ok(SocketResponse::Pong),
        SocketRequest::ListConversationMessages {
            conversation_id,
            before_seq,
            limit,
        } => {
            let rows = store
                .list_conversation_messages(
                    &conversation_id,
                    before_seq.map(|seq| i64::try_from(seq).unwrap_or(i64::MAX)),
                    limit,
                )
                .await?;
            let message_ids: Vec<String> = rows.iter().map(|r| r.message_id.clone()).collect();
            let reaction_rows = store.list_reactions_for_messages(&message_ids).await?;
            let reactions_by_message = aggregate_reactions_by_message(reaction_rows);
            let messages = rows
                .into_iter()
                .map(|row| {
                    let reactions = reactions_by_message
                        .get(&row.message_id)
                        .cloned()
                        .unwrap_or_default();
                    local_conversation_message_from_row(row, reactions)
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
            let has_more = messages.len() >= limit;
            let next_before_seq = if has_more {
                messages.last().map(|message| message.message_seq)
            } else {
                None
            };
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({
                    "conversation_id": conversation_id,
                    "messages": messages,
                    "next_before_seq": next_before_seq,
                    "has_more": has_more,
                })),
            })
        }
        SocketRequest::DelegateToAgent {
            conversation_id,
            source_agent,
            source_session_id,
            target_agent,
            profile_id,
            target_profile,
            prompt,
        } => {
            let source_agent = source_agent
                .as_deref()
                .map(parse_socket_agent)
                .transpose()?;
            let prompt = prompt.trim().to_owned();
            anyhow::ensure!(!prompt.is_empty(), "delegate_to_agent prompt is empty");
            let (target_agent, resolved_profile_id, launch) = resolve_delegate_launch_target(
                &store,
                target_agent.as_deref(),
                profile_id.as_deref(),
                target_profile.as_deref(),
            )
            .await?;
            validate_mcp_source_session(
                &store,
                &conversation_id,
                source_agent,
                source_session_id.as_deref(),
            )
            .await?;
            let teamwork_store = open_teamwork_store_for_conversation(
                &db_path,
                &conversation_id,
                &default_workspace,
            )
            .await?;
            teamwork_store
                .ensure_delegate_target_allowed(
                    &conversation_id,
                    source_session_id.as_deref(),
                    target_agent,
                )
                .await?;
            let workspace =
                workspace_for_mcp_conversation(&store, &conversation_id, &default_workspace)
                    .await?;
            let target_label = agent_label(target_agent);
            anyhow::ensure!(
                store
                    .is_conversation_agent_member(&conversation_id, target_label)
                    .await?,
                "agent '{target_label}' is not a member of conversation {conversation_id}"
            );
            // Same launch path as start_agent_in_conversation RPC: profile fields via
            // AgentLaunchOptions (explicit request fields are not on the MCP tool).
            let outcome = manager
                .start_agent_in_conversation_with_options(
                    target_agent,
                    workspace.clone(),
                    conversation_id.clone(),
                    launch,
                )
                .await?;
            persist_thread_parent_rows_inner(
                &store,
                &outcome.session_id,
                &outcome.cwd.display().to_string(),
                target_agent,
                outcome.provider_session_id.as_deref(),
                Some(&conversation_id),
            )
            .await;
            manager
                .send_user_message(&outcome.session_id, prompt.clone())
                .await?;
            let delegation = teamwork_store
                .create_delegation(
                    &conversation_id,
                    source_agent,
                    source_session_id.clone(),
                    target_agent,
                    prompt,
                    Some(outcome.session_id.clone()),
                )
                .await?;
            let short_target = short_mcp_session_id(&outcome.session_id);
            let visible_prompt = format!(
                "@{}#{} {}",
                target_agent.bin_name(),
                short_target,
                delegation.prompt.trim()
            );
            let visible_message_id = format!(
                "mcp-delegation:{}:{}",
                conversation_id,
                uuid::Uuid::new_v4()
            );
            let sender_role = if source_agent.is_some() {
                "agent"
            } else {
                "user"
            };
            let mentions = vec![minos_protocol::ConversationMention {
                agent: target_agent,
                session_id: Some(outcome.session_id.clone()),
                session_short_id: Some(short_target),
            }];
            let mentions_json = serde_json::to_string(&mentions).unwrap_or_else(|_| "[]".into());
            // Bind request message so completion can set reply_to.
            let _ = teamwork_store
                .set_delegation_request_message_id(
                    &conversation_id,
                    &delegation.delegation_id,
                    &visible_message_id,
                )
                .await;
            let message_seq = store
                .upsert_conversation_message(
                    &conversation_id,
                    &visible_message_id,
                    source_session_id.as_deref(),
                    sender_role,
                    source_agent.map(agent_label),
                    &visible_prompt,
                    current_unix_ms(),
                    None,
                    Some(delegation.delegation_id.as_str()),
                    &mentions_json,
                )
                .await?;
            publish_conversation_message_appended(
                &local_conversation_event_tx,
                &conversation_id,
                message_seq,
            );
            tracing::info!(
                target: "minos_daemon::agent",
                conversation_id = %conversation_id,
                session_id = %outcome.session_id,
                profile_id = resolved_profile_id.as_deref().unwrap_or(""),
                agent = %agent_label(target_agent),
                "MCP delegate_to_agent started session"
            );
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({
                    "accepted": true,
                    "target_agent": target_agent.bin_name(),
                    "profile_id": resolved_profile_id,
                    "session_id": outcome.session_id,
                    "delegation": delegation,
                })),
            })
        }
        SocketRequest::GetDelegationStatus {
            conversation_id,
            delegation_id,
        } => {
            let teamwork_store = open_teamwork_store_for_conversation(
                &db_path,
                &conversation_id,
                &default_workspace,
            )
            .await?;
            let delegation = teamwork_store
                .get_delegation(&conversation_id, &delegation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("delegation not found: {delegation_id}"))?;
            Ok(SocketResponse::Ok {
                data: Some(serde_json::to_value(delegation)?),
            })
        }
        SocketRequest::WaitDelegation {
            conversation_id,
            delegation_id,
            timeout_ms,
        } => {
            let teamwork_store = open_teamwork_store_for_conversation(
                &db_path,
                &conversation_id,
                &default_workspace,
            )
            .await?;
            let timeout = std::time::Duration::from_millis(timeout_ms as u64);
            // Event-driven wake via DelegationSignalBus; this is only a multi-process fallback.
            let poll = minos_chat_store::DEFAULT_DELEGATION_WAIT_FALLBACK_POLL;
            let (delegation, timed_out) = teamwork_store
                .wait_delegation(&conversation_id, &delegation_id, timeout, poll)
                .await?;
            let source_delivery = teamwork_store
                .latest_source_delivery_for_delegation(&conversation_id, &delegation_id)
                .await?
                .map(|delivery| match delivery.status {
                    minos_chat_store::TeamworkSourceDeliveryStatus::Pending => "pending",
                    minos_chat_store::TeamworkSourceDeliveryStatus::Delivered => "delivered",
                    minos_chat_store::TeamworkSourceDeliveryStatus::Failed => "failed",
                });
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({
                    "status": delegation.status,
                    "timed_out": timed_out,
                    "result_text": delegation.result_text,
                    "error": delegation.error,
                    "result_message_id": delegation.result_message_id,
                    "source_delivery": source_delivery,
                    "delegation": delegation,
                })),
            })
        }
        SocketRequest::CancelDelegation {
            conversation_id,
            delegation_id,
            reason,
        } => {
            let teamwork_store = open_teamwork_store_for_conversation(
                &db_path,
                &conversation_id,
                &default_workspace,
            )
            .await?;
            let delegation = teamwork_store
                .cancel_delegation(&conversation_id, &delegation_id, reason)
                .await?;
            if let Some(session_id) = delegation.session_id.as_deref() {
                let _ = manager.interrupt_session(session_id).await;
            }
            Ok(SocketResponse::Ok {
                data: Some(serde_json::to_value(delegation)?),
            })
        }
        SocketRequest::PostConversationUpdate {
            conversation_id,
            source_agent,
            source_session_id,
            message,
        } => {
            let source_agent = source_agent
                .as_deref()
                .map(parse_socket_agent)
                .transpose()?;
            validate_mcp_source_session(
                &store,
                &conversation_id,
                source_agent,
                source_session_id.as_deref(),
            )
            .await?;
            let body = message.trim();
            anyhow::ensure!(
                !body.is_empty(),
                "post_conversation_update message is empty"
            );
            let text = deliver_daemon_post_update_target(
                manager.clone(),
                store.clone(),
                &conversation_id,
                &default_workspace,
                body,
            )
            .await?;
            let message_id = format!("mcp:{}:{}", conversation_id, uuid::Uuid::new_v4());
            let sender_role = if source_agent.is_some() {
                "agent"
            } else {
                "user"
            };
            let mentions = parse_conversation_mentions_from_body(&text);
            let mentions_json = serde_json::to_string(&mentions).unwrap_or_else(|_| "[]".into());
            let message_seq = store
                .upsert_conversation_message(
                    &conversation_id,
                    &message_id,
                    source_session_id.as_deref(),
                    sender_role,
                    source_agent.map(agent_label),
                    &text,
                    current_unix_ms(),
                    None,
                    None,
                    &mentions_json,
                )
                .await?;
            publish_conversation_message_appended(
                &local_conversation_event_tx,
                &conversation_id,
                message_seq,
            );
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({ "accepted": true })),
            })
        }
    }
}

async fn open_teamwork_store_for_conversation(
    db_path: &Path,
    conversation_id: &str,
    workspace: &Path,
) -> anyhow::Result<minos_chat_store::TeamworkStore> {
    let store = minos_chat_store::TeamworkStore::open(db_path).await?;
    let workspace_root = workspace.display().to_string();
    store
        .ensure_conversation(conversation_id, conversation_id, &workspace_root)
        .await?;
    Ok(store)
}

async fn workspace_for_mcp_conversation(
    store: &LocalStore,
    conversation_id: &str,
    default_workspace: &Path,
) -> anyhow::Result<PathBuf> {
    let Some(conversation) = store.get_conversation(conversation_id).await? else {
        anyhow::bail!("conversation not found: {conversation_id}");
    };
    let Some(project) = store.get_project(&conversation.project_id).await? else {
        return Ok(default_workspace.to_path_buf());
    };
    Ok(project
        .workspace_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_workspace.to_path_buf()))
}

async fn validate_mcp_source_session(
    store: &LocalStore,
    conversation_id: &str,
    source_agent: Option<AgentName>,
    source_session_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        source_agent.is_none() || source_session_id.is_some(),
        "MCP source_session_id is required when source_agent is set"
    );
    let Some(source_session_id) = source_session_id else {
        return Ok(());
    };
    let rows = store.list_sessions_by_conversation(conversation_id).await?;
    let Some(row) = rows.iter().find(|row| row.session_id == source_session_id) else {
        anyhow::bail!(
            "MCP source session {source_session_id} does not belong to conversation {conversation_id} \
             (session may have been closed or never started)"
        );
    };
    if row.status == "closed" {
        let reason = row.last_close_reason.as_deref().unwrap_or("unknown");
        anyhow::bail!(
            "MCP source session {source_session_id} is closed (reason={reason}); \
             teamwork tools are unavailable for this session"
        );
    }
    let session_agent = parse_socket_agent(&row.agent)?;
    if let Some(source_agent) = source_agent {
        anyhow::ensure!(
            session_agent == source_agent,
            "MCP source session {source_session_id} belongs to {}, not {}",
            session_agent.bin_name(),
            source_agent.bin_name()
        );
    }
    // Roster is membership SSOT: a removed agent must not keep using MCP.
    let member_label = agent_label(session_agent);
    anyhow::ensure!(
        store
            .is_conversation_agent_member(conversation_id, member_label)
            .await?,
        "MCP source agent '{member_label}' is no longer a member of conversation {conversation_id}; \
         session work was invalidated when the agent left the roster"
    );
    Ok(())
}

async fn deliver_daemon_post_update_target(
    manager: Arc<AgentManager>,
    store: Arc<LocalStore>,
    conversation_id: &str,
    default_workspace: &Path,
    body: &str,
) -> anyhow::Result<String> {
    let Some((target_agent, session_short_id, prompt)) = parse_mcp_agent_routing(body) else {
        return Ok(body.to_owned());
    };
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return Ok(body.to_owned());
    }
    if let Some(session_short_id) = session_short_id {
        let session_id = mcp_session_id_for_agent_short_id(
            &manager,
            &store,
            conversation_id,
            target_agent,
            &session_short_id,
        )
        .await?;
        manager.send_user_message(&session_id, prompt).await?;
        return Ok(body.to_owned());
    }

    let workspace =
        workspace_for_mcp_conversation(&store, conversation_id, default_workspace).await?;
    let target_label = agent_label(target_agent);
    anyhow::ensure!(
        store
            .is_conversation_agent_member(conversation_id, target_label)
            .await?,
        "agent '{target_label}' is not a member of conversation {conversation_id}"
    );
    let outcome = manager
        .start_agent_in_conversation(target_agent, workspace.clone(), conversation_id.to_owned())
        .await?;
    persist_thread_parent_rows_inner(
        &store,
        &outcome.session_id,
        &outcome.cwd.display().to_string(),
        target_agent,
        outcome.provider_session_id.as_deref(),
        Some(conversation_id),
    )
    .await;
    manager
        .send_user_message(&outcome.session_id, prompt.clone())
        .await?;
    Ok(format!(
        "@{}#{} {}",
        target_agent.bin_name(),
        short_mcp_session_id(&outcome.session_id),
        prompt
    ))
}

async fn mcp_session_id_for_agent_short_id(
    manager: &AgentManager,
    store: &LocalStore,
    conversation_id: &str,
    agent: AgentName,
    session_short_id: &str,
) -> anyhow::Result<String> {
    let short_id = session_short_id.to_ascii_lowercase();
    let rows = store.list_sessions_by_conversation(conversation_id).await?;
    let Some(row) = rows.into_iter().find(|row| {
        row.parent_session_id.is_none()
            && row.agent == agent.bin_name()
            && (short_mcp_session_id(&row.session_id).to_ascii_lowercase() == short_id
                || row.session_id.to_ascii_lowercase().starts_with(&short_id))
    }) else {
        anyhow::bail!(
            "No existing {} session matches #{}",
            agent.bin_name(),
            session_short_id
        );
    };
    let state = row_state_to_runtime(&row)?;
    anyhow::ensure!(
        !matches!(state, SessionState::Closed { .. }),
        "{} session #{} is closed",
        agent.bin_name(),
        short_mcp_session_id(&row.session_id)
    );
    if !manager.has_thread(&row.session_id).await {
        manager
            .register_persisted_thread(
                row.session_id.clone(),
                PathBuf::from(&row.workspace_root),
                agent,
                row.provider_session_id.clone(),
                row.parent_session_id.clone(),
                Some(row.conversation_id.clone()),
                state,
                u64::try_from(row.last_seq.max(0)).unwrap_or(u64::MAX),
            )
            .await?;
    }
    Ok(row.session_id)
}

fn parse_mcp_agent_routing(text: &str) -> Option<(AgentName, Option<String>, String)> {
    let rest = text.trim_start().strip_prefix('@')?;
    let split_at = rest
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let target = &rest[..split_at];
    let body = rest[split_at..].trim_start().to_owned();
    let (agent, session_short_id) = match target.split_once('#') {
        Some((agent, session_short_id)) if !session_short_id.is_empty() => (
            parse_socket_agent(agent).ok()?,
            Some(session_short_id.to_owned()),
        ),
        Some(_) => return None,
        None => (parse_socket_agent(target).ok()?, None),
    };
    Some((agent, session_short_id, body))
}

fn parse_socket_agent(value: &str) -> anyhow::Result<AgentName> {
    let normalized = value.trim().to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {value}"))
}

/// Shared launch merge used by RPC start and MCP delegate.
///
/// Precedence: explicit request fields > profile fields > None.
/// When `profile_id` is set, `agent` must equal `profile.runtime_agent`.
/// Returns a plain error message (caller maps to protocol / anyhow).
async fn resolve_launch_options(
    store: &crate::store::LocalStore,
    agent: AgentName,
    profile_id: Option<&str>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    instructions: Option<String>,
) -> Result<Option<minos_agent_runtime::AgentLaunchOptions>, String> {
    let (base_model, base_effort, base_instructions) =
        if let Some(pid) = profile_id.map(str::trim).filter(|s| !s.is_empty()) {
            let row = store
                .get_agent_profile(pid)
                .await
                .map_err(|e| format!("resolve_launch_options.get_agent_profile: {e}"))?
                .ok_or_else(|| format!("agent profile not found: {pid}"))?;
            let profile_agent = parse_agent_label(&row.runtime_agent).map_err(|e| e.to_string())?;
            if profile_agent != agent {
                return Err(format!(
                    "agent mismatch for profile {pid}: request agent is {}, profile runtime is {}",
                    agent_label(agent),
                    agent_label(profile_agent),
                ));
            }
            (
                nonempty_opt(Some(row.model)),
                nonempty_opt(Some(row.reasoning_effort)),
                nonempty_opt(Some(row.instructions)),
            )
        } else {
            (None, None, None)
        };

    // Explicit request fields win over profile (and over empty profile fields).
    let model = nonempty_opt(model).or(base_model);
    let reasoning_effort = nonempty_opt(reasoning_effort).or(base_effort);
    let instructions = nonempty_opt(instructions).or(base_instructions);

    Ok(minos_agent_runtime::AgentLaunchOptions::from_parts_full(
        model,
        reasoning_effort,
        instructions,
    ))
}

/// Resolve MCP/TUI delegate target to `(agent, profile_id)`, then launch options
/// via [`resolve_launch_options`] (same merge as RPC start).
///
/// Returns `(agent, resolved_profile_id, launch_options)`.
async fn resolve_delegate_launch_target(
    store: &crate::store::LocalStore,
    target_agent: Option<&str>,
    profile_id: Option<&str>,
    target_profile: Option<&str>,
) -> anyhow::Result<(
    AgentName,
    Option<String>,
    Option<minos_agent_runtime::AgentLaunchOptions>,
)> {
    let (agent, resolved_profile_id) =
        resolve_delegate_agent_and_profile(store, target_agent, profile_id, target_profile).await?;
    let launch = resolve_launch_options(
        store,
        agent,
        resolved_profile_id.as_deref(),
        None,
        None,
        None,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    Ok((agent, resolved_profile_id, launch))
}

/// Bind delegate identity only: explicit `profile_id` / `target_profile` name, or
/// bare `target_agent` with newest host profile convenience. Does not merge launch fields.
async fn resolve_delegate_agent_and_profile(
    store: &crate::store::LocalStore,
    target_agent: Option<&str>,
    profile_id: Option<&str>,
    target_profile: Option<&str>,
) -> anyhow::Result<(AgentName, Option<String>)> {
    let requested_agent = target_agent
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_socket_agent)
        .transpose()?;

    let explicit_profile_id = profile_id.map(str::trim).filter(|s| !s.is_empty());
    let profile_name = target_profile.map(str::trim).filter(|s| !s.is_empty());

    let profile_row = if let Some(pid) = explicit_profile_id {
        Some(
            store
                .get_agent_profile(pid)
                .await?
                .ok_or_else(|| anyhow::anyhow!("agent profile not found: {pid}"))?,
        )
    } else if let Some(name) = profile_name {
        let key = name.to_ascii_lowercase();
        let matches: Vec<_> = store
            .list_agent_profiles()
            .await?
            .into_iter()
            .filter(|row| row.name.trim().to_ascii_lowercase() == key)
            .collect();
        match matches.len() {
            0 => anyhow::bail!("agent profile not found by name: {name}"),
            1 => Some(matches.into_iter().next().expect("len == 1")),
            _ => anyhow::bail!(
                "agent profile name is ambiguous ({} matches): {name}; use profile_id",
                matches.len()
            ),
        }
    } else {
        None
    };

    if let Some(row) = profile_row {
        let profile_agent = parse_socket_agent(&row.runtime_agent)?;
        if let Some(requested) = requested_agent {
            anyhow::ensure!(
                requested == profile_agent,
                "agent mismatch for profile {}: request agent is {}, profile runtime is {}",
                row.id,
                agent_label(requested),
                agent_label(profile_agent),
            );
        }
        return Ok((profile_agent, Some(row.id)));
    }

    let Some(agent) = requested_agent else {
        anyhow::bail!("delegate_to_agent requires target_agent, profile_id, or target_profile");
    };

    // Bare runtime: newest profile convenience (list is updated_at DESC).
    let newest = store.list_agent_profiles().await?.into_iter().find(|row| {
        parse_socket_agent(&row.runtime_agent)
            .map(|a| a == agent)
            .unwrap_or(false)
    });
    if let Some(row) = newest {
        return Ok((agent, Some(row.id)));
    }
    Ok((agent, None))
}

fn parse_conversation_mentions_from_body(body: &str) -> Vec<minos_protocol::ConversationMention> {
    let mut mentions = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find('@') {
        rest = &rest[at + 1..];
        let token_end = rest
            .find(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == ')' || ch == ']')
            .unwrap_or(rest.len());
        let token = &rest[..token_end];
        rest = &rest[token_end..];
        if token.is_empty() {
            continue;
        }
        let (agent_name, short_id) = match token.split_once('#') {
            Some((agent, short)) if !short.is_empty() => (agent, Some(short.to_owned())),
            Some(_) => continue,
            None => (token, None),
        };
        let Ok(agent) = parse_socket_agent(agent_name) else {
            continue;
        };
        mentions.push(minos_protocol::ConversationMention {
            agent,
            session_id: None,
            session_short_id: short_id,
        });
    }
    mentions
}

fn short_mcp_session_id(session_id: &str) -> String {
    session_id[..8.min(session_id.len())].to_owned()
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
    session_id: &str,
    cwd: &str,
    agent: minos_domain::AgentName,
    provider_session_id: Option<&str>,
    conversation_id: Option<&str>,
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
    let owned_conversation_id;
    let conversation_id = match conversation_id {
        Some(conversation_id) => conversation_id,
        None => match ensure_workspace_conversation(store, cwd, now_ms).await {
            Ok(conversation_id) => {
                owned_conversation_id = conversation_id;
                owned_conversation_id.as_str()
            }
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    session_id = %session_id,
                    workspace = %cwd,
                    "ensure_workspace_conversation failed; events FK may reject ingest",
                );
                return;
            }
        },
    };
    if let Err(e) = store
        .insert_session_in_conversation(
            session_id,
            conversation_id,
            cwd,
            agent_label(agent),
            provider_session_id,
            None,
            "idle",
            now_ms,
            true,
        )
        .await
    {
        tracing::warn!(
            target: "minos_daemon::agent",
            error = %e,
            session_id = %session_id,
            "store.insert_session failed; events FK may reject ingest",
        );
    }
    if let Some(provider_session_id) = provider_session_id {
        if let Err(e) = store
            .update_session_provider_session_id(session_id, Some(provider_session_id))
            .await
        {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                session_id = %session_id,
                "store.update_session_provider_session_id failed",
            );
        }
    }
}

async fn persist_subagent_thread_parent_row(
    store: &LocalStore,
    session_id: &str,
    parent_session_id: &str,
    cwd: &str,
    agent: minos_domain::AgentName,
    now_ms: i64,
) {
    let parent = match store.get_session(parent_session_id).await {
        Ok(Some(parent)) => parent,
        Ok(None) => {
            tracing::warn!(
                target: "minos_daemon::agent",
                session_id,
                parent_session_id,
                workspace = %cwd,
                "subagent parent thread missing; events FK may reject ingest",
            );
            return;
        }
        Err(error) => {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %error,
                session_id,
                parent_session_id,
                "store.get_session failed for subagent parent",
            );
            return;
        }
    };

    let workspace_root = if parent.workspace_root.is_empty() {
        cwd
    } else {
        parent.workspace_root.as_str()
    };
    if let Err(error) = store
        .insert_session_in_conversation(
            session_id,
            &parent.conversation_id,
            workspace_root,
            agent_label(agent),
            None,
            Some(parent_session_id),
            "idle",
            now_ms,
            false,
        )
        .await
    {
        tracing::warn!(
            target: "minos_daemon::agent",
            error = %error,
            session_id,
            parent_session_id,
            "store.insert_session failed for subagent",
        );
    }
}

async fn ensure_workspace_conversation(
    store: &LocalStore,
    cwd: &str,
    now_ms: i64,
) -> anyhow::Result<String> {
    let slug = workspace_slug(cwd);
    let project_id = format!("workspace-{slug}");
    let conversation_id = format!("conversation-{slug}");
    let title = "Direct agent sessions";
    store
        .create_project(&project_id, title, &slug, Some(cwd), now_ms)
        .await
        .or_else(|error| {
            if is_unique_constraint(&error) {
                Ok(())
            } else {
                Err(error)
            }
        })?;
    store
        .create_conversation(&conversation_id, &project_id, title, now_ms)
        .await
        .or_else(|error| {
            if is_unique_constraint(&error) {
                Ok(())
            } else {
                Err(error)
            }
        })?;
    Ok(conversation_id)
}

fn workspace_slug(cwd: &str) -> String {
    let mut slug = cwd
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        "workspace".into()
    } else {
        slug.chars().take(96).collect()
    }
}

fn is_unique_constraint(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains("UNIQUE constraint failed"))
}

fn provider_session_id_from_event(
    session_id: &str,
    agent: minos_domain::AgentName,
    event: &EventRow,
) -> Option<String> {
    let payload = serde_json::from_slice(event.body_inline.as_deref()?).ok()?;
    provider_session_id_from_ingest(&RawIngest {
        agent,
        session_id: session_id.to_string(),
        provider_session_id: provider_session_id_from_payload(agent, &payload),
        provider_event_id: None,
        event_type: None,
        body: minos_agent_runtime::RawBody::InlineBytes {
            bytes: serde_json::to_vec(&payload).ok()?,
            media_type: "application/json".into(),
        },
        ts_ms: event.ts_ms,
    })
}

fn provider_session_id_from_payload(
    agent: minos_domain::AgentName,
    payload: &serde_json::Value,
) -> Option<String> {
    RawIngest::from_json(agent, String::new(), payload.clone(), 0).provider_session_id
}

fn state_to_proto(state: &minos_agent_runtime::SessionState) -> ProtoSessionState {
    use minos_agent_runtime::SessionState as RtState;
    match state {
        RtState::Starting => ProtoSessionState::Starting,
        RtState::Idle => ProtoSessionState::Idle,
        RtState::Running { turn_started_at_ms } => ProtoSessionState::Running {
            turn_started_at_ms: *turn_started_at_ms,
        },
        RtState::Suspended { reason } => ProtoSessionState::Suspended {
            reason: pause_to_proto(reason),
        },
        RtState::Resuming => ProtoSessionState::Resuming,
        RtState::Closed { reason } => ProtoSessionState::Closed {
            reason: close_to_proto(reason),
        },
    }
}

fn local_event_from_manager(event: &ManagerEvent) -> LocalManagerEvent {
    match event {
        ManagerEvent::SessionAdded {
            session_id,
            workspace,
            agent,
            parent_session_id,
        } => LocalManagerEvent::SessionAdded {
            session_id: session_id.clone(),
            workspace: workspace.display().to_string(),
            agent: *agent,
            parent_session_id: parent_session_id.clone(),
        },
        ManagerEvent::SessionStateChanged {
            session_id,
            old,
            new,
            at_ms,
        } => LocalManagerEvent::SessionStateChanged {
            session_id: session_id.clone(),
            old: state_to_proto(old),
            new: state_to_proto(new),
            at_ms: *at_ms,
        },
        ManagerEvent::SessionClosed { session_id, reason } => LocalManagerEvent::SessionClosed {
            session_id: session_id.clone(),
            reason: close_to_proto(reason),
        },
        ManagerEvent::InstanceCrashed {
            workspace,
            affected_threads,
            reason,
        } => LocalManagerEvent::InstanceCrashed {
            workspace: workspace.display().to_string(),
            affected_threads: affected_threads.clone(),
            reason: pause_to_proto(reason),
        },
    }
}

fn publish_conversation_message_appended(
    tx: &broadcast::Sender<LocalConversationEvent>,
    conversation_id: &str,
    message_seq: i64,
) {
    let _ = tx.send(LocalConversationEvent::ConversationMessageAppended {
        conversation_id: conversation_id.to_owned(),
        message_seq,
    });
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
        TestGlue {
            glue: AgentGlue::new(
                tmp.path().join("workspaces"),
                Arc::new(std::collections::HashMap::new()),
                store,
            ),
            _tmp: tmp,
        }
    }

    async fn seed_thread(
        glue: &AgentGlue,
        session_id: &str,
        agent: &str,
        started_at: i64,
        last_activity_at: i64,
    ) {
        glue.store.upsert_workspace("/w", started_at).await.unwrap();
        sqlx::query(
            "INSERT OR IGNORE INTO projects(project_id, name, workspace_slug, workspace_path, created_at, updated_at) \
             VALUES ('p-test', 'Test', 'test', '/w', 0, 0)",
        )
        .execute(glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations(conversation_id, project_id, title, created_at_ms, updated_at_ms) \
             VALUES (?, 'p-test', 'Test', ?, ?)",
        )
        .bind(format!("c-{session_id}"))
        .bind(started_at)
        .bind(last_activity_at)
        .execute(glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES (?, ?, '/w', ?, 'idle', 3, ?, ?)",
        )
        .bind(session_id)
        .bind(format!("c-{session_id}"))
        .bind(agent)
        .bind(started_at)
        .bind(last_activity_at)
        .execute(glue.store.pool())
        .await
        .unwrap();
    }

    async fn seed_conversation(glue: &AgentGlue, conversation_id: &str) {
        glue.store
            .create_project("p-test", "Test", "test", Some("/w"), 0)
            .await
            .unwrap();
        glue.store
            .create_conversation(conversation_id, "p-test", "Test", 0)
            .await
            .unwrap();
    }

    async fn seed_conversation_with_agents(
        glue: &AgentGlue,
        conversation_id: &str,
        agents: &[&str],
    ) {
        seed_conversation(glue, conversation_id).await;
        let members = agents.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        glue.store
            .set_conversation_agent_members(conversation_id, &members, 1)
            .await
            .unwrap();
    }

    #[test]
    fn validate_agent_profile_name_accepts_clean_tokens() {
        assert_eq!(
            validate_agent_profile_name("ResearchGrok", "create_agent_profile").unwrap(),
            "ResearchGrok"
        );
        assert_eq!(
            validate_agent_profile_name("  Helper  ", "create_agent_profile").unwrap(),
            "Helper"
        );
        assert_eq!(
            validate_agent_profile_name("my-agent_1", "update_agent_profile").unwrap(),
            "my-agent_1"
        );
    }

    #[test]
    fn validate_agent_profile_name_rejects_whitespace_hash_at() {
        for bad in ["has space", "has\ttab", "hash#name", "at@name", "a b#c", ""] {
            let err = validate_agent_profile_name(bad, "create_agent_profile")
                .expect_err("should reject");
            let msg = err.to_string();
            assert!(
                msg.contains("required")
                    || msg.contains("whitespace")
                    || msg.contains('#')
                    || msg.contains('@'),
                "unexpected error for {bad:?}: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn create_agent_profile_rejects_invalid_name() {
        let test = test_glue().await;
        let err = test
            .glue
            .create_agent_profile(minos_protocol::CreateAgentProfileRequest {
                name: "bad name".into(),
                description: String::new(),
                runtime_agent: AgentName::Grok,
                model: "grok-4".into(),
                reasoning_effort: String::new(),
                instructions: String::new(),
            })
            .await
            .expect_err("whitespace name");
        assert!(
            err.to_string().contains("whitespace") || err.to_string().contains('#'),
            "unexpected: {err}"
        );

        let err = test
            .glue
            .create_agent_profile(minos_protocol::CreateAgentProfileRequest {
                name: "hash#x".into(),
                description: String::new(),
                runtime_agent: AgentName::Grok,
                model: "grok-4".into(),
                reasoning_effort: String::new(),
                instructions: String::new(),
            })
            .await
            .expect_err("hash name");
        assert!(err.to_string().contains('#'), "unexpected: {err}");

        let err = test
            .glue
            .create_agent_profile(minos_protocol::CreateAgentProfileRequest {
                name: "at@x".into(),
                description: String::new(),
                runtime_agent: AgentName::Grok,
                model: "grok-4".into(),
                reasoning_effort: String::new(),
                instructions: String::new(),
            })
            .await
            .expect_err("at name");
        assert!(err.to_string().contains('@'), "unexpected: {err}");
    }

    #[tokio::test]
    async fn update_agent_profile_rejects_invalid_name() {
        let test = test_glue().await;
        let created = test
            .glue
            .create_agent_profile(minos_protocol::CreateAgentProfileRequest {
                name: "ValidName".into(),
                description: String::new(),
                runtime_agent: AgentName::Codex,
                model: "gpt-5".into(),
                reasoning_effort: String::new(),
                instructions: String::new(),
            })
            .await
            .expect("create ok");

        let err = test
            .glue
            .update_agent_profile(minos_protocol::UpdateAgentProfileRequest {
                id: created.id.clone(),
                name: "not valid".into(),
                description: String::new(),
                instructions: String::new(),
            })
            .await
            .expect_err("whitespace update name");
        assert!(err.to_string().contains("whitespace"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn resolve_launch_options_applies_profile_fields() {
        let test = test_glue().await;
        test.glue
            .store
            .create_agent_profile(
                "profile-research",
                "Research",
                "deep",
                "grok",
                "grok-4",
                "high",
                "You are a researcher.",
                1,
            )
            .await
            .unwrap();

        let launch = test
            .glue
            .resolve_launch_options(AgentName::Grok, Some("profile-research"), None, None, None)
            .await
            .unwrap()
            .expect("profile should yield launch options");
        assert_eq!(launch.model.as_deref(), Some("grok-4"));
        assert_eq!(launch.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(
            launch.instructions.as_deref(),
            Some("You are a researcher.")
        );
    }

    #[tokio::test]
    async fn resolve_launch_options_rejects_agent_mismatch() {
        let test = test_glue().await;
        test.glue
            .store
            .create_agent_profile(
                "profile-codex",
                "Coder",
                "",
                "codex",
                "gpt-5",
                "medium",
                "",
                1,
            )
            .await
            .unwrap();

        let err = test
            .glue
            .resolve_launch_options(AgentName::Grok, Some("profile-codex"), None, None, None)
            .await
            .expect_err("agent must match profile runtime");
        let msg = err.to_string();
        assert!(
            msg.contains("agent mismatch") || msg.contains("mismatch"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn resolve_launch_options_explicit_overrides_profile() {
        let test = test_glue().await;
        test.glue
            .store
            .create_agent_profile(
                "profile-base",
                "Base",
                "",
                "claude",
                "claude-sonnet",
                "low",
                "base instructions",
                1,
            )
            .await
            .unwrap();

        let launch = test
            .glue
            .resolve_launch_options(
                AgentName::Claude,
                Some("profile-base"),
                Some("claude-opus".into()),
                None,
                Some("override instructions".into()),
            )
            .await
            .unwrap()
            .expect("launch options");
        assert_eq!(launch.model.as_deref(), Some("claude-opus"));
        // effort not overridden → profile value
        assert_eq!(launch.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(
            launch.instructions.as_deref(),
            Some("override instructions")
        );
    }

    #[tokio::test]
    async fn resolve_delegate_launch_target_newest_profile_for_runtime() {
        let test = test_glue().await;
        test.glue
            .store
            .create_agent_profile(
                "profile-old",
                "Old",
                "",
                "grok",
                "grok-old",
                "low",
                "old",
                10,
            )
            .await
            .unwrap();
        test.glue
            .store
            .create_agent_profile(
                "profile-new",
                "New",
                "",
                "grok",
                "grok-new",
                "high",
                "new",
                99,
            )
            .await
            .unwrap();

        let (agent, profile_id, launch) =
            resolve_delegate_launch_target(&test.glue.store, Some("grok"), None, None)
                .await
                .unwrap();
        assert_eq!(agent, AgentName::Grok);
        assert_eq!(profile_id.as_deref(), Some("profile-new"));
        let launch = launch.expect("newest profile should yield launch options");
        assert_eq!(launch.model.as_deref(), Some("grok-new"));
        assert_eq!(launch.reasoning_effort.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn resolve_delegate_launch_target_by_profile_name() {
        let test = test_glue().await;
        test.glue
            .store
            .create_agent_profile(
                "profile-research",
                "ResearchGrok",
                "",
                "grok",
                "grok-4",
                "high",
                "research",
                1,
            )
            .await
            .unwrap();

        let (agent, profile_id, launch) =
            resolve_delegate_launch_target(&test.glue.store, None, None, Some("researchgrok"))
                .await
                .unwrap();
        assert_eq!(agent, AgentName::Grok);
        assert_eq!(profile_id.as_deref(), Some("profile-research"));
        assert_eq!(launch.and_then(|l| l.model).as_deref(), Some("grok-4"));
    }

    #[tokio::test]
    async fn resolve_delegate_launch_target_by_profile_id_only() {
        let test = test_glue().await;
        test.glue
            .store
            .create_agent_profile(
                "profile-solo",
                "Solo",
                "",
                "claude",
                "claude-opus",
                "medium",
                "solo instructions",
                1,
            )
            .await
            .unwrap();

        let (agent, profile_id, launch) =
            resolve_delegate_launch_target(&test.glue.store, None, Some("profile-solo"), None)
                .await
                .unwrap();
        assert_eq!(agent, AgentName::Claude);
        assert_eq!(profile_id.as_deref(), Some("profile-solo"));
        let launch = launch.expect("profile_id should yield launch options");
        assert_eq!(launch.model.as_deref(), Some("claude-opus"));
        assert_eq!(launch.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(launch.instructions.as_deref(), Some("solo instructions"));
    }

    #[tokio::test]
    async fn resolve_delegate_launch_target_rejects_agent_mismatch() {
        let test = test_glue().await;
        test.glue
            .store
            .create_agent_profile(
                "profile-codex",
                "Coder",
                "",
                "codex",
                "gpt-5",
                "high",
                "",
                1,
            )
            .await
            .unwrap();

        let err = resolve_delegate_launch_target(
            &test.glue.store,
            Some("grok"),
            Some("profile-codex"),
            None,
        )
        .await
        .expect_err("request agent must match profile runtime");
        let msg = err.to_string();
        assert!(
            msg.contains("agent mismatch") || msg.contains("mismatch"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn resolve_launch_options_without_profile_uses_explicit_only() {
        let test = test_glue().await;
        let launch = test
            .glue
            .resolve_launch_options(
                AgentName::Codex,
                None,
                Some("gpt-5".into()),
                Some("high".into()),
                None,
            )
            .await
            .unwrap()
            .expect("explicit fields");
        assert_eq!(launch.model.as_deref(), Some("gpt-5"));
        assert_eq!(launch.reasoning_effort.as_deref(), Some("high"));
        assert!(launch.instructions.is_none());
    }

    #[tokio::test]
    async fn resolve_launch_options_missing_profile_errors() {
        let test = test_glue().await;
        let err = test
            .glue
            .resolve_launch_options(AgentName::Grok, Some("no-such-profile"), None, None, None)
            .await
            .expect_err("missing profile");
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn daemon_mcp_post_conversation_update_appends_and_emits_local_event() {
        let test = test_glue().await;
        seed_conversation_with_agents(&test.glue, "conversation-mcp", &["codex"]).await;
        test.glue.store.upsert_workspace("/w", 1).await.unwrap();
        test.glue
            .store
            .insert_session_in_conversation(
                "thread-codex-1234",
                "conversation-mcp",
                "/w",
                "codex",
                Some("thread-codex-1234"),
                None,
                "idle",
                1,
                true,
            )
            .await
            .unwrap();
        let (event_tx, mut event_rx) = broadcast::channel(4);

        let response = handle_daemon_mcp_request(
            test.glue.manager.clone(),
            test.glue.store.clone(),
            PathBuf::from("unused-teamwork.sqlite"),
            test.glue.default_workspace.clone(),
            event_tx,
            SocketRequest::PostConversationUpdate {
                conversation_id: "conversation-mcp".into(),
                source_agent: Some("codex".into()),
                source_session_id: Some("thread-codex-1234".into()),
                message: "review posted".into(),
            },
        )
        .await
        .unwrap();

        assert!(matches!(response, SocketResponse::Ok { .. }));
        let rows = test
            .glue
            .store
            .list_conversation_messages("conversation-mcp", None, Some(10))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sender_role, "agent");
        assert_eq!(rows[0].agent.as_deref(), Some("codex"));
        assert_eq!(rows[0].session_id.as_deref(), Some("thread-codex-1234"));
        assert_eq!(rows[0].body, "review posted");

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .expect("conversation append event should be emitted")
            .expect("event channel should stay open");
        assert_eq!(
            event,
            LocalConversationEvent::ConversationMessageAppended {
                conversation_id: "conversation-mcp".into(),
                message_seq: rows[0].message_seq,
            }
        );
    }

    #[tokio::test]
    async fn daemon_mcp_delegate_blocks_third_agent_from_delegated_thread() {
        let test = test_glue().await;
        seed_conversation_with_agents(
            &test.glue,
            "conversation-mcp",
            &["codex", "opencode", "gemini"],
        )
        .await;
        test.glue.store.upsert_workspace("/w", 1).await.unwrap();
        test.glue
            .store
            .insert_session_in_conversation(
                "thread-opencode-1234",
                "conversation-mcp",
                "/w",
                "opencode",
                Some("thread-opencode-1234"),
                None,
                "idle",
                1,
                true,
            )
            .await
            .unwrap();
        let teamwork_db = test._tmp.path().join("teamwork.sqlite");
        let teamwork_store = minos_chat_store::TeamworkStore::open(&teamwork_db)
            .await
            .unwrap();
        teamwork_store
            .ensure_conversation("conversation-mcp", "main", "/w")
            .await
            .unwrap();
        teamwork_store
            .create_delegation(
                "conversation-mcp",
                Some(AgentName::Codex),
                Some("thread-codex-1234".into()),
                AgentName::Opencode,
                "check this".into(),
                Some("thread-opencode-1234".into()),
            )
            .await
            .unwrap();
        let (event_tx, _) = broadcast::channel(4);

        let error = handle_daemon_mcp_request(
            test.glue.manager.clone(),
            test.glue.store.clone(),
            teamwork_db,
            test.glue.default_workspace.clone(),
            event_tx,
            SocketRequest::DelegateToAgent {
                conversation_id: "conversation-mcp".into(),
                source_agent: Some("opencode".into()),
                source_session_id: Some("thread-opencode-1234".into()),
                target_agent: Some("gemini".into()),
                profile_id: None,
                target_profile: None,
                prompt: "say hi".into(),
            },
        )
        .await
        .expect_err("third-agent delegation should be rejected");

        assert!(error
            .to_string()
            .contains("may only delegate back to codex"));
        assert!(test
            .glue
            .store
            .list_conversation_messages("conversation-mcp", None, Some(10))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn remove_conversation_agent_closes_sessions_and_cancels_delegations() {
        let test = test_glue().await;
        seed_conversation_with_agents(&test.glue, "conversation-roster", &["codex", "claude"])
            .await;
        test.glue.store.upsert_workspace("/w", 1).await.unwrap();
        test.glue
            .store
            .insert_session_in_conversation(
                "thread-claude-1",
                "conversation-roster",
                "/w",
                "claude",
                Some("thread-claude-1"),
                None,
                "running",
                1,
                true,
            )
            .await
            .unwrap();
        test.glue
            .store
            .insert_session_in_conversation(
                "thread-codex-1",
                "conversation-roster",
                "/w",
                "codex",
                Some("thread-codex-1"),
                None,
                "idle",
                1,
                true,
            )
            .await
            .unwrap();

        let teamwork = minos_chat_store::TeamworkStore::open(test.glue.store.db_path())
            .await
            .unwrap();
        teamwork
            .ensure_conversation("conversation-roster", "roster", "/w")
            .await
            .unwrap();
        let delegation = teamwork
            .create_delegation(
                "conversation-roster",
                Some(AgentName::Codex),
                Some("thread-codex-1".into()),
                AgentName::Claude,
                "do work".into(),
                Some("thread-claude-1".into()),
            )
            .await
            .unwrap();

        let response = test
            .glue
            .remove_conversation_agent(minos_protocol::RemoveConversationAgentParams {
                conversation_id: "conversation-roster".into(),
                agent: "claude".into(),
            })
            .await
            .unwrap();

        assert!(!response
            .conversation
            .participating_agents
            .contains(&AgentName::Claude));
        assert!(response
            .conversation
            .participating_agents
            .contains(&AgentName::Codex));
        assert_eq!(
            response.closed_session_ids,
            vec!["thread-claude-1".to_owned()]
        );
        assert_eq!(
            response.cancelled_delegation_ids,
            vec![delegation.delegation_id.clone()]
        );

        let claude_row = test
            .glue
            .store
            .get_session("thread-claude-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claude_row.status, "closed");
        assert_eq!(
            claude_row.last_close_reason.as_deref(),
            Some("roster_removed")
        );

        let codex_row = test
            .glue
            .store
            .get_session("thread-codex-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(codex_row.status, "idle");

        let cancelled = teamwork
            .get_delegation("conversation-roster", &delegation.delegation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            cancelled.status,
            minos_chat_store::TeamworkDelegationStatus::Cancelled
        );

        // MCP from the removed agent session must fail with a membership error.
        let (event_tx, _) = broadcast::channel(4);
        let error = handle_daemon_mcp_request(
            test.glue.manager.clone(),
            test.glue.store.clone(),
            test.glue.store.db_path().to_path_buf(),
            test.glue.default_workspace.clone(),
            event_tx,
            SocketRequest::PostConversationUpdate {
                conversation_id: "conversation-roster".into(),
                source_agent: Some("claude".into()),
                source_session_id: Some("thread-claude-1".into()),
                message: "still here".into(),
            },
        )
        .await
        .expect_err("removed agent MCP should be rejected");
        let message = error.to_string();
        assert!(
            message.contains("closed") || message.contains("no longer a member"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn list_sessions_reads_persisted_rows_and_filters_agent() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-a", "codex", 10, 20).await;
        seed_thread(&test.glue, "thr-b", "claude", 30, 40).await;

        let response = test
            .glue
            .list_sessions(ListSessionsParams {
                limit: 50,
                before_ts_ms: None,
                agent: Some(minos_domain::AgentName::Claude),
            })
            .await
            .unwrap();

        assert_eq!(response.sessions.len(), 1);
        assert_eq!(response.sessions[0].session_id, "thr-b");
        assert_eq!(response.sessions[0].agent, minos_domain::AgentName::Claude);
        assert_eq!(response.sessions[0].message_count, 3);
        assert_eq!(response.sessions[0].first_ts_ms, 30);
        assert_eq!(response.sessions[0].last_ts_ms, 40);
    }

    #[tokio::test]
    async fn persisted_ingest_stream_emits_committed_event_seq() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-live", "codex", 10, 20).await;
        test.glue
            .manager
            .register_persisted_thread(
                "thr-live".into(),
                PathBuf::from("/w"),
                AgentName::Codex,
                None,
                None,
                Some("c-live".into()),
                SessionState::Idle,
                3,
            )
            .await
            .unwrap();

        let mut rx = test.glue.persisted_ingest_stream();
        let _ = test
            .glue
            .manager
            .send_user_message("thr-live", "hello".into())
            .await;
        let frame = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("persisted ingest should be emitted")
            .expect("persisted ingest channel should stay open");

        assert_eq!(frame.session_id, "thr-live");
        assert_eq!(frame.seq, 4);
        assert_eq!(frame.agent, AgentName::Codex);
        assert!(frame.ui_events.iter().any(|event| matches!(
            event,
            minos_ui_protocol::UiEventMessage::MessageStarted { .. }
        )));
    }

    #[tokio::test]
    async fn get_session_uses_persisted_suspended_state() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-s", "codex", 10, 20).await;
        sqlx::query(
            "UPDATE sessions SET status = 'suspended', last_pause_reason = 'daemon_restart' WHERE session_id = 'thr-s'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();

        let response = test
            .glue
            .get_session(GetSessionParams {
                session_id: "thr-s".into(),
            })
            .await
            .unwrap();

        assert_eq!(response.thread.session_id, "thr-s");
        assert_eq!(
            response.state,
            ProtoSessionState::Suspended {
                reason: ProtoPauseReason::DaemonRestart,
            }
        );
    }

    #[tokio::test]
    async fn get_session_maps_closed_reason_from_store() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-c", "codex", 10, 20).await;
        test.glue
            .store
            .close_session_row("thr-c", "user_close", 55)
            .await
            .unwrap();

        let response = test
            .glue
            .get_session(GetSessionParams {
                session_id: "thr-c".into(),
            })
            .await
            .unwrap();

        assert_eq!(response.thread.ended_at_ms, Some(55));
        assert_eq!(
            response.thread.end_reason,
            Some(SessionEndReason::UserStopped)
        );
        assert_eq!(
            response.state,
            ProtoSessionState::Closed {
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
            "INSERT INTO projects(project_id, name, workspace_slug, workspace_path, created_at, updated_at) \
             VALUES ('p-live', 'Live', 'live', ?, 10, 10)",
        )
        .bind(&workspace_root)
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations(conversation_id, project_id, title, created_at_ms, updated_at_ms) \
             VALUES ('c-live', 'p-live', 'Live', 10, 10)",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES ('thr-live', 'c-live', ?, 'codex', 'idle', 0, 10, 10)",
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
                None,
                Some("c-live".into()),
                SessionState::Idle,
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
            .get_session("thr-live")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "running");
        assert!(matches!(
            test.glue.current_state(),
            SessionState::Running { .. }
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
            "INSERT INTO projects(project_id, name, workspace_slug, workspace_path, created_at, updated_at) \
             VALUES ('p-snapshot', 'Snapshot', 'snapshot', ?, 10, 10)",
        )
        .bind(&workspace_root)
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations(conversation_id, project_id, title, created_at_ms, updated_at_ms) \
             VALUES ('c-snapshot', 'p-snapshot', 'Snapshot', 10, 20)",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions(session_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES ('thr-snapshot', 'c-snapshot', ?, 'codex', 'idle', 0, 10, 20)",
        )
        .bind(&workspace_root)
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        let state = SessionState::Running {
            turn_started_at_ms: 99,
        };
        test.glue
            .manager
            .register_persisted_thread(
                "thr-snapshot".into(),
                workspace,
                minos_domain::AgentName::Codex,
                Some("thr-snapshot".into()),
                None,
                Some("c-live".into()),
                state.clone(),
                0,
            )
            .await
            .unwrap();

        let snapshot = test
            .glue
            .current_agent_session()
            .await
            .unwrap()
            .expect("live thread snapshot");

        assert_eq!(snapshot.session_id, "thr-snapshot");
        assert_eq!(snapshot.workspace_root, workspace_root);
        assert_eq!(snapshot.state, state);
    }

    #[tokio::test]
    async fn resume_session_registers_persisted_thread_and_returns_workspace() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-r", "codex", 10, 20).await;
        sqlx::query(
            "UPDATE sessions SET status = 'suspended', last_pause_reason = 'daemon_restart', provider_session_id = 'thr-r' WHERE session_id = 'thr-r'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();

        let response = test.glue.resume_session("thr-r", false).await.unwrap();
        assert_eq!(response.session_id, "thr-r");
        assert_eq!(response.cwd, "/w");
        assert!(test.glue.manager.has_thread("thr-r").await);
    }

    #[tokio::test]
    async fn shutdown_keeps_idle_threads_resumable_without_paused() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-stop", "codex", 10, 20).await;
        test.glue
            .manager
            .register_persisted_thread(
                "thr-stop".into(),
                PathBuf::from("/w"),
                minos_domain::AgentName::Codex,
                Some("thr-stop".into()),
                None,
                Some("c-thr-stop".into()),
                SessionState::Idle,
                0,
            )
            .await
            .unwrap();

        test.glue.shutdown().await.unwrap();

        let row = test
            .glue
            .store
            .get_session("thr-stop")
            .await
            .unwrap()
            .unwrap();
        // Idle between turns must not become user-visible Paused after stop.
        assert_eq!(row.status, "idle");
        assert!(row.last_pause_reason.is_none());
        assert!(!row.needs_continue);
        // In-process manager still parks as Suspended so children tear down cleanly.
        assert!(matches!(
            test.glue.manager.list_sessions().await[0].state,
            SessionState::Suspended {
                reason: minos_agent_runtime::PauseReason::DaemonRestart
            }
        ));
    }

    #[tokio::test]
    async fn resume_session_recovers_non_codex_provider_session_id_from_events() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-g", "gemini", 10, 20).await;
        sqlx::query(
            "UPDATE sessions SET status = 'suspended', last_pause_reason = 'daemon_restart', provider_session_id = 'thr-g', last_seq = 1 WHERE session_id = 'thr-g'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO events(session_id, seq, body_kind, body_inline, projection_json, ts_ms, source) VALUES (?, 1, 'inline', ?, ?, 25, 'live')",
        )
        .bind("thr-g")
        .bind(
            serde_json::to_vec(&serde_json::json!({
                "kind":"acp_notification",
                "params":{"sessionId":"gemini-provider-session"}
            }))
            .unwrap(),
        )
        .bind(b"[]".as_slice())
        .execute(test.glue.store.pool())
        .await
        .unwrap();

        let response = test.glue.resume_session("thr-g", false).await.unwrap();

        assert_eq!(response.session_id, "thr-g");
        assert_eq!(
            test.glue
                .manager
                .session_provider_session_id("thr-g")
                .await
                .as_deref(),
            Some("gemini-provider-session")
        );
    }
}
