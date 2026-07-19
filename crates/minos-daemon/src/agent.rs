use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use minos_agent_runtime::{
    AgentLaunchMode, AgentManager, AgentRuntimeConfig, InstanceCaps, ManagerEvent, RawIngest,
    SessionPolicies, ThreadState,
};
use minos_chat_store::mcp_socket::{SocketRequest, SocketResponse};
use minos_codex_protocol::SkillsListResponse as CodexSkillsListResponse;
use minos_domain::{AgentName, MinosError};
use minos_protocol::{
    AgentDispatchRequest, AgentDispatchResponse, AgentLaunchMode as ProtoAgentLaunchMode,
    ApprovalDecisionRequest, CloseReason as ProtoCloseReason, CloseThreadRequest, GetThreadParams,
    GetThreadResponse, HostSkillError, HostSkillSummary, HostSkillsEntry, InterruptThreadRequest,
    ListHostSkillsRequest, ListHostSkillsResponse, ListHostWorkspacesRequest,
    ListHostWorkspacesResponse, ListThreadsParams, ListThreadsResponse, LocalConversationEvent,
    LocalIngestFrame, LocalManagerEvent, PauseReason as ProtoPauseReason, SendUserMessageRequest,
    StartAgentRequest, StartAgentResponse, ThreadState as ProtoThreadState, ThreadSummary,
    WriteHostSkillConfigRequest, WriteHostSkillConfigResponse,
};
use minos_ui_protocol::ThreadEndReason;
use tokio::sync::{broadcast, watch};

use crate::store::event_writer::{provider_session_id_from_ingest, EventWriter};
use crate::store::{ChatMessageRow, ConversationRow, EventRow, LocalStore, ThreadRow};
use crate::subscription::{AgentStateObserver, Subscription};
use crate::{ingest_coalescer::IngestCoalescer, ingest_sync::IngestSyncHandle};

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
                let thread_id = ingest.thread_id.clone();
                let agent = ingest.agent;
                let ts_ms = ingest.ts_ms;
                let payload_bytes = ingest.body_len();
                let chunk = match coalescer_clone.coalesce(ingest).await {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        tracing::error!(
                            target: "minos_daemon::agent",
                            error = %error,
                            thread_id = %thread_id,
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
                            .on_ingest_frame(&thread_id, agent, &ui_events)
                            .await;
                        let _ = persisted_ingest_tx_clone.send(LocalIngestFrame {
                            thread_id: thread_id.clone(),
                            seq,
                            agent,
                            ui_events,
                            ts_ms,
                        });
                        tracing::info!(
                            target: "minos_daemon::agent",
                            thread_id = %thread_id,
                            seq,
                            bytes = payload_bytes,
                            "ingest event committed",
                        );
                    }
                    Err(e) => tracing::error!(
                        target: "minos_daemon::agent",
                        error = %e,
                        thread_id = %thread_id,
                        "EventWriter.write_chunk failed; event not persisted locally",
                    ),
                }
            }
        });

        let (state_tx, state_rx) = watch::channel(ThreadState::Idle);
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
                            ManagerEvent::ThreadAdded {
                                thread_id,
                                workspace,
                                agent,
                                parent_thread_id,
                            } => {
                                let cwd = workspace.display().to_string();
                                let now_ms = current_unix_ms();
                                if let Err(e) = store_clone.upsert_workspace(&cwd, now_ms).await {
                                    tracing::warn!(
                                        target: "minos_daemon::agent",
                                        error = %e,
                                        thread_id = %thread_id,
                                        agent = %agent_label(agent),
                                        workspace = %cwd,
                                        "store.upsert_workspace failed for ThreadAdded",
                                    );
                                }
                                if let Some(parent_thread_id) = parent_thread_id {
                                    persist_subagent_thread_parent_row(
                                        &store_clone,
                                        &thread_id,
                                        &parent_thread_id,
                                        &cwd,
                                        agent,
                                        now_ms,
                                    )
                                    .await;
                                }
                            }
                            ManagerEvent::ThreadStateChanged {
                                thread_id,
                                new,
                                at_ms,
                                ..
                            } => {
                                persist_runtime_state_inner(&store_clone, &thread_id, &new, at_ms)
                                    .await;
                                completion_for_state.on_thread_state(&thread_id, &new).await;
                                let _ = state_tx_clone.send(new);
                            }
                            ManagerEvent::ThreadClosed { thread_id, reason } => {
                                let state = ThreadState::Closed { reason };
                                let at_ms = current_unix_ms();
                                persist_runtime_state_inner(
                                    &store_clone,
                                    &thread_id,
                                    &state,
                                    at_ms,
                                )
                                .await;
                                completion_for_state
                                    .on_thread_state(&thread_id, &state)
                                    .await;
                                let _ = state_tx_clone.send(state);
                            }
                            ManagerEvent::InstanceCrashed {
                                affected_threads,
                                reason,
                                ..
                            } => {
                                let state = ThreadState::Suspended { reason };
                                let at_ms = current_unix_ms();
                                for thread_id in affected_threads {
                                    persist_runtime_state_inner(
                                        &store_clone,
                                        &thread_id,
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
            let ui_events: Vec<minos_ui_protocol::UiEventMessage> =
                serde_json::from_slice(&event.projection_json).map_err(|e| {
                    MinosError::CodexProtocolError {
                        method: "read_thread_raw_history".into(),
                        message: e.to_string(),
                    }
                })?;
            events.push(minos_protocol::LocalIngestFrame {
                thread_id: thread_id.to_owned(),
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
        let launch = minos_agent_runtime::AgentLaunchOptions::from_parts_full(
            req.model.clone(),
            req.reasoning_effort.clone(),
            req.instructions.clone(),
        );
        let outcome = self
            .manager
            .start_agent_with_policies(req.agent, workspace, None, launch)
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
        let launch = minos_agent_runtime::AgentLaunchOptions::from_parts_full(
            req.model.clone(),
            req.reasoning_effort.clone(),
            req.instructions.clone(),
        );
        let outcome = self
            .manager
            .start_agent_with_thread_id_and_options(
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

        let _ = self.state_tx.send(ThreadState::Idle);
        Ok(StartAgentResponse { session_id, cwd })
    }

    pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError> {
        // User text always wins over a pending auto-continue: claim the flag
        // so open-time inject cannot race a second CONTINUE turn.
        if let Err(e) = self.store.take_needs_continue(&req.session_id).await {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                thread_id = %req.session_id,
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

    pub async fn respond_opencode_question(
        &self,
        req: minos_protocol::RespondOpencodeQuestionRequest,
    ) -> Result<(), MinosError> {
        self.manager
            .respond_opencode_question(&req.thread_id, &req.question_id, req.answers)
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
        thread_id: &str,
        cwd: &str,
        agent: minos_domain::AgentName,
        provider_session_id: Option<&str>,
    ) {
        persist_thread_parent_rows_inner(
            &self.store,
            thread_id,
            cwd,
            agent,
            provider_session_id,
            None,
        )
        .await;
    }

    async fn persist_thread_parent_rows_in_conversation(
        &self,
        thread_id: &str,
        conversation_id: &str,
        cwd: &str,
        agent: minos_domain::AgentName,
        provider_session_id: Option<&str>,
    ) {
        persist_thread_parent_rows_inner(
            &self.store,
            thread_id,
            cwd,
            agent,
            provider_session_id,
            Some(conversation_id),
        )
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
            return Ok(row.provider_session_id.clone());
        }

        if let Some(session_id) = row
            .provider_session_id
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
                row.parent_thread_id.clone(),
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
    pub async fn resume_thread(
        &self,
        thread_id: &str,
        auto_continue: bool,
    ) -> Result<StartAgentResponse, MinosError> {
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
        // Register as Suspended when DB says so so reattach can run; live
        // Idle/Running rows also register with their persisted state.
        let register_state = match row_state_to_runtime(&row)? {
            minos_agent_runtime::ThreadState::Closed { reason } => {
                minos_agent_runtime::ThreadState::Closed { reason }
            }
            // Prefer Suspended for rehydrate so reattach path is used even if
            // status was idle in an older partial write (defensive).
            other
                if matches!(
                    other,
                    minos_agent_runtime::ThreadState::Idle
                        | minos_agent_runtime::ThreadState::Running { .. }
                        | minos_agent_runtime::ThreadState::Starting
                        | minos_agent_runtime::ThreadState::Resuming
                ) && !self.manager.has_thread(thread_id).await =>
            {
                // Not live yet after daemon restart — treat as suspended rehydrate.
                minos_agent_runtime::ThreadState::Suspended {
                    reason: minos_agent_runtime::PauseReason::DaemonRestart,
                }
            }
            other => other,
        };
        self.manager
            .register_persisted_thread(
                row.thread_id.clone(),
                PathBuf::from(&row.workspace_root),
                agent,
                provider_session_id,
                row.parent_thread_id.clone(),
                Some(row.conversation_id.clone()),
                register_state,
                u64::try_from(row.last_seq.max(0)).unwrap_or(u64::MAX),
            )
            .await
            .map_err(map_anyhow)?;

        // Idle/Running already live → no-op. Suspended → provider reattach → Idle.
        // Provider spawn may fail (missing CLI / no fake server in unit tests);
        // keep the row registered so a later send can re-try reattach.
        if let Err(e) = self.manager.reattach_suspended_thread(thread_id).await {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %e,
                thread_id = %thread_id,
                auto_continue,
                "reattach_suspended_thread failed; thread registered, reattach deferred",
            );
            if auto_continue {
                // Cannot continue without a live provider session.
                return Err(map_anyhow(e));
            }
        }

        if auto_continue {
            match self.store.take_needs_continue(thread_id).await {
                Ok(true) => {
                    if let Err(e) = self.manager.inject_continue_prompt(thread_id).await {
                        // Restore flag so a later open/send can retry.
                        let _ = self.store.set_needs_continue(thread_id, true).await;
                        tracing::warn!(
                            target: "minos_daemon::agent",
                            error = %e,
                            thread_id = %thread_id,
                            "inject_continue_prompt failed; needs_continue restored",
                        );
                        return Err(map_anyhow(e));
                    }
                    self.persist_current_provider_session_id(thread_id).await;
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %e,
                        thread_id = %thread_id,
                        "take_needs_continue failed during resume",
                    );
                }
            }
        }

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
        let snap = self.manager.list_threads().await;
        let now_ms = current_unix_ms();
        for s in snap {
            match self.manager.suspend_for_daemon_stop(&s.thread_id).await {
                Ok(needs_continue) => {
                    if let Err(e) = self
                        .store
                        .suspend_thread_for_daemon_restart(&s.thread_id, needs_continue, now_ms)
                        .await
                    {
                        tracing::warn!(
                            target: "minos_daemon::agent",
                            error = %e,
                            thread_id = %s.thread_id,
                            "suspend_thread_for_daemon_restart failed during shutdown",
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %e,
                        thread_id = %s.thread_id,
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
        let conversation_id = uuid::Uuid::new_v4().to_string();
        let now_ms = current_unix_ms();

        // Snapshot git context from the project workspace at create time.
        let (branch, worktree_path) = project
            .workspace_path
            .as_deref()
            .map(|p| crate::git_snapshot::detect_git_snapshot(std::path::Path::new(p)))
            .unwrap_or((None, None));
        let meta = crate::store::ConversationCreateMeta {
            priority: None,
            progress: Some("todo".into()),
            branch,
            worktree_path,
        };

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
        tracing::info!(
            target: "minos_daemon::agent",
            project_id = %project.project_id,
            conversation_id = %conversation_id,
            branch = ?meta.branch,
            "conversation created",
        );
        let row = self
            .store
            .get_conversation(&conversation_id)
            .await
            .map_err(|e| map_store_error("create_conversation.reload", e))?
            .expect("conversation inserted above");
        Ok(minos_protocol::CreateConversationResponse {
            conversation: conversation_summary_from_row(row, Vec::new())?,
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
        let messages = rows
            .into_iter()
            .take(requested_limit as usize)
            .map(local_conversation_message_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::ListConversationMessagesResponse { messages, has_more })
    }

    pub async fn list_conversation_agent_sessions(
        &self,
        req: minos_protocol::ListConversationAgentSessionsParams,
    ) -> Result<minos_protocol::ListConversationAgentSessionsResponse, MinosError> {
        let live_states: HashMap<String, ProtoThreadState> = self
            .manager
            .list_threads()
            .await
            .into_iter()
            .map(|snapshot| (snapshot.thread_id.clone(), state_to_proto(&snapshot.state)))
            .collect();
        let threads = self
            .store
            .list_threads_by_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("list_conversation_agent_sessions", e))?
            .into_iter()
            .map(|row| {
                let mut summary = thread_summary_from_row(row.clone())?;
                if let Some(state) = live_states.get(&summary.thread_id) {
                    summary.state = state.clone();
                }
                Ok(summary)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(minos_protocol::ListConversationAgentSessionsResponse { threads })
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
                req.thread_id.as_deref(),
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
        let launch = minos_agent_runtime::AgentLaunchOptions::from_parts_full(
            req.model.clone(),
            req.reasoning_effort.clone(),
            req.instructions.clone(),
        );
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
            &outcome.thread_id,
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
        let _ = self.state_tx.send(ThreadState::Idle);
        tracing::info!(
            target: "minos_daemon::agent",
            conversation_id = %req.conversation_id,
            thread_id = %outcome.thread_id,
            agent = %agent_label(req.agent),
            workspace = %cwd,
            "agent session started in conversation",
        );
        Ok(StartAgentResponse {
            session_id: outcome.thread_id,
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
        let name = req.name.trim();
        if name.is_empty() {
            return Err(MinosError::CodexProtocolError {
                method: "create_agent_profile".into(),
                message: "name is required".into(),
            });
        }
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
        let name = req.name.trim();
        if name.is_empty() {
            return Err(MinosError::CodexProtocolError {
                method: "update_agent_profile".into(),
                message: "name is required".into(),
            });
        }
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

fn thread_summary_from_row(row: crate::store::ThreadRow) -> Result<ThreadSummary, MinosError> {
    let end_reason = row_end_reason(&row);
    let state = row_state_to_proto(&row)?;
    Ok(ThreadSummary {
        thread_id: row.thread_id,
        agent: parse_agent_label(&row.agent)?,
        title: None,
        first_ts_ms: row.started_at,
        last_ts_ms: row.last_activity_at,
        message_count: u32::try_from(row.last_seq.max(0)).unwrap_or(u32::MAX),
        ended_at_ms: row.ended_at,
        end_reason,
        parent_thread_id: row.parent_thread_id,
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
        thread_id: row.thread_id,
        created_at_ms: row.created_at_ms,
        sender_role: row.sender_role,
        agent: row.agent.as_deref().map(parse_agent_label).transpose()?,
        body: row.body,
        reply_to_message_id: row.reply_to_message_id,
        delegation_id: row.delegation_id,
        mentions,
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
            let messages = store
                .list_conversation_messages(
                    &conversation_id,
                    before_seq.map(|seq| i64::try_from(seq).unwrap_or(i64::MAX)),
                    limit,
                )
                .await?
                .into_iter()
                .map(local_conversation_message_from_row)
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
            source_thread_id,
            target_agent,
            prompt,
        } => {
            let target_agent = parse_socket_agent(&target_agent)?;
            let source_agent = source_agent
                .as_deref()
                .map(parse_socket_agent)
                .transpose()?;
            let prompt = prompt.trim().to_owned();
            anyhow::ensure!(!prompt.is_empty(), "delegate_to_agent prompt is empty");
            validate_mcp_source_thread(
                &store,
                &conversation_id,
                source_agent,
                source_thread_id.as_deref(),
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
                    source_thread_id.as_deref(),
                    target_agent,
                )
                .await?;
            let workspace =
                workspace_for_mcp_conversation(&store, &conversation_id, &default_workspace)
                    .await?;
            let outcome = manager
                .start_agent_in_conversation(
                    target_agent,
                    workspace.clone(),
                    conversation_id.clone(),
                )
                .await?;
            persist_thread_parent_rows_inner(
                &store,
                &outcome.thread_id,
                &outcome.cwd.display().to_string(),
                target_agent,
                outcome.provider_session_id.as_deref(),
                Some(&conversation_id),
            )
            .await;
            manager
                .send_user_message(&outcome.thread_id, prompt.clone())
                .await?;
            let delegation = teamwork_store
                .create_delegation(
                    &conversation_id,
                    source_agent,
                    source_thread_id.clone(),
                    target_agent,
                    prompt,
                    Some(outcome.thread_id.clone()),
                )
                .await?;
            let short_target = short_mcp_thread_id(&outcome.thread_id);
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
                thread_id: Some(outcome.thread_id.clone()),
                thread_short_id: Some(short_target),
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
                    source_thread_id.as_deref(),
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
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({
                    "accepted": true,
                    "target_agent": target_agent.bin_name(),
                    "thread_id": outcome.thread_id,
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
            let poll = std::time::Duration::from_millis(200);
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
            if let Some(thread_id) = delegation.thread_id.as_deref() {
                let _ = manager.interrupt_thread(thread_id).await;
            }
            Ok(SocketResponse::Ok {
                data: Some(serde_json::to_value(delegation)?),
            })
        }
        SocketRequest::PostConversationUpdate {
            conversation_id,
            source_agent,
            source_thread_id,
            message,
        } => {
            let source_agent = source_agent
                .as_deref()
                .map(parse_socket_agent)
                .transpose()?;
            validate_mcp_source_thread(
                &store,
                &conversation_id,
                source_agent,
                source_thread_id.as_deref(),
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
                    source_thread_id.as_deref(),
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

async fn validate_mcp_source_thread(
    store: &LocalStore,
    conversation_id: &str,
    source_agent: Option<AgentName>,
    source_thread_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        source_agent.is_none() || source_thread_id.is_some(),
        "MCP source_thread_id is required when source_agent is set"
    );
    let Some(source_thread_id) = source_thread_id else {
        return Ok(());
    };
    let rows = store.list_threads_by_conversation(conversation_id).await?;
    let Some(row) = rows.iter().find(|row| row.thread_id == source_thread_id) else {
        anyhow::bail!(
            "MCP source thread {source_thread_id} does not belong to conversation {conversation_id}"
        );
    };
    if let Some(source_agent) = source_agent {
        let actual = parse_socket_agent(&row.agent)?;
        anyhow::ensure!(
            actual == source_agent,
            "MCP source thread {source_thread_id} belongs to {}, not {}",
            actual.bin_name(),
            source_agent.bin_name()
        );
    }
    Ok(())
}

async fn deliver_daemon_post_update_target(
    manager: Arc<AgentManager>,
    store: Arc<LocalStore>,
    conversation_id: &str,
    default_workspace: &Path,
    body: &str,
) -> anyhow::Result<String> {
    let Some((target_agent, thread_short_id, prompt)) = parse_mcp_agent_routing(body) else {
        return Ok(body.to_owned());
    };
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return Ok(body.to_owned());
    }
    if let Some(thread_short_id) = thread_short_id {
        let thread_id = mcp_thread_id_for_agent_short_id(
            &manager,
            &store,
            conversation_id,
            target_agent,
            &thread_short_id,
        )
        .await?;
        manager.send_user_message(&thread_id, prompt).await?;
        return Ok(body.to_owned());
    }

    let workspace =
        workspace_for_mcp_conversation(&store, conversation_id, default_workspace).await?;
    let outcome = manager
        .start_agent_in_conversation(target_agent, workspace.clone(), conversation_id.to_owned())
        .await?;
    persist_thread_parent_rows_inner(
        &store,
        &outcome.thread_id,
        &outcome.cwd.display().to_string(),
        target_agent,
        outcome.provider_session_id.as_deref(),
        Some(conversation_id),
    )
    .await;
    manager
        .send_user_message(&outcome.thread_id, prompt.clone())
        .await?;
    Ok(format!(
        "@{}#{} {}",
        target_agent.bin_name(),
        short_mcp_thread_id(&outcome.thread_id),
        prompt
    ))
}

async fn mcp_thread_id_for_agent_short_id(
    manager: &AgentManager,
    store: &LocalStore,
    conversation_id: &str,
    agent: AgentName,
    thread_short_id: &str,
) -> anyhow::Result<String> {
    let short_id = thread_short_id.to_ascii_lowercase();
    let rows = store.list_threads_by_conversation(conversation_id).await?;
    let Some(row) = rows.into_iter().find(|row| {
        row.parent_thread_id.is_none()
            && row.agent == agent.bin_name()
            && (short_mcp_thread_id(&row.thread_id).to_ascii_lowercase() == short_id
                || row.thread_id.to_ascii_lowercase().starts_with(&short_id))
    }) else {
        anyhow::bail!(
            "No existing {} session matches #{}",
            agent.bin_name(),
            thread_short_id
        );
    };
    let state = row_state_to_runtime(&row)?;
    anyhow::ensure!(
        !matches!(state, ThreadState::Closed { .. }),
        "{} session #{} is closed",
        agent.bin_name(),
        short_mcp_thread_id(&row.thread_id)
    );
    if !manager.has_thread(&row.thread_id).await {
        manager
            .register_persisted_thread(
                row.thread_id.clone(),
                PathBuf::from(&row.workspace_root),
                agent,
                row.provider_session_id.clone(),
                row.parent_thread_id.clone(),
                Some(row.conversation_id.clone()),
                state,
                u64::try_from(row.last_seq.max(0)).unwrap_or(u64::MAX),
            )
            .await?;
    }
    Ok(row.thread_id)
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
    let (agent, thread_short_id) = match target.split_once('#') {
        Some((agent, thread_short_id)) if !thread_short_id.is_empty() => (
            parse_socket_agent(agent).ok()?,
            Some(thread_short_id.to_owned()),
        ),
        Some(_) => return None,
        None => (parse_socket_agent(target).ok()?, None),
    };
    Some((agent, thread_short_id, body))
}

fn parse_socket_agent(value: &str) -> anyhow::Result<AgentName> {
    let normalized = value.trim().to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {value}"))
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
            thread_id: None,
            thread_short_id: short_id,
        });
    }
    mentions
}

fn short_mcp_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
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
                    thread_id = %thread_id,
                    workspace = %cwd,
                    "ensure_workspace_conversation failed; events FK may reject ingest",
                );
                return;
            }
        },
    };
    if let Err(e) = store
        .insert_thread_in_conversation(
            thread_id,
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

async fn persist_subagent_thread_parent_row(
    store: &LocalStore,
    thread_id: &str,
    parent_thread_id: &str,
    cwd: &str,
    agent: minos_domain::AgentName,
    now_ms: i64,
) {
    let parent = match store.get_thread(parent_thread_id).await {
        Ok(Some(parent)) => parent,
        Ok(None) => {
            tracing::warn!(
                target: "minos_daemon::agent",
                thread_id,
                parent_thread_id,
                workspace = %cwd,
                "subagent parent thread missing; events FK may reject ingest",
            );
            return;
        }
        Err(error) => {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %error,
                thread_id,
                parent_thread_id,
                "store.get_thread failed for subagent parent",
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
        .insert_thread_in_conversation(
            thread_id,
            &parent.conversation_id,
            workspace_root,
            agent_label(agent),
            None,
            Some(parent_thread_id),
            "idle",
            now_ms,
            false,
        )
        .await
    {
        tracing::warn!(
            target: "minos_daemon::agent",
            error = %error,
            thread_id,
            parent_thread_id,
            "store.insert_thread failed for subagent",
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
    thread_id: &str,
    agent: minos_domain::AgentName,
    event: &EventRow,
) -> Option<String> {
    let payload = serde_json::from_slice(event.body_inline.as_deref()?).ok()?;
    provider_session_id_from_ingest(&RawIngest {
        agent,
        thread_id: thread_id.to_string(),
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

fn local_event_from_manager(event: &ManagerEvent) -> LocalManagerEvent {
    match event {
        ManagerEvent::ThreadAdded {
            thread_id,
            workspace,
            agent,
            parent_thread_id,
        } => LocalManagerEvent::ThreadAdded {
            thread_id: thread_id.clone(),
            workspace: workspace.display().to_string(),
            agent: *agent,
            parent_thread_id: parent_thread_id.clone(),
        },
        ManagerEvent::ThreadStateChanged {
            thread_id,
            old,
            new,
            at_ms,
        } => LocalManagerEvent::ThreadStateChanged {
            thread_id: thread_id.clone(),
            old: state_to_proto(old),
            new: state_to_proto(new),
            at_ms: *at_ms,
        },
        ManagerEvent::ThreadClosed { thread_id, reason } => LocalManagerEvent::ThreadClosed {
            thread_id: thread_id.clone(),
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
        thread_id: &str,
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
        .bind(format!("c-{thread_id}"))
        .bind(started_at)
        .bind(last_activity_at)
        .execute(glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO threads(thread_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES (?, ?, '/w', ?, 'idle', 3, ?, ?)",
        )
        .bind(thread_id)
        .bind(format!("c-{thread_id}"))
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

    #[tokio::test]
    async fn daemon_mcp_post_conversation_update_appends_and_emits_local_event() {
        let test = test_glue().await;
        seed_conversation(&test.glue, "conversation-mcp").await;
        test.glue.store.upsert_workspace("/w", 1).await.unwrap();
        test.glue
            .store
            .insert_thread_in_conversation(
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
                source_thread_id: Some("thread-codex-1234".into()),
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
        assert_eq!(rows[0].thread_id.as_deref(), Some("thread-codex-1234"));
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
        seed_conversation(&test.glue, "conversation-mcp").await;
        test.glue.store.upsert_workspace("/w", 1).await.unwrap();
        test.glue
            .store
            .insert_thread_in_conversation(
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
                source_thread_id: Some("thread-opencode-1234".into()),
                target_agent: "gemini".into(),
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
                ThreadState::Idle,
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

        assert_eq!(frame.thread_id, "thr-live");
        assert_eq!(frame.seq, 4);
        assert_eq!(frame.agent, AgentName::Codex);
        assert!(frame.ui_events.iter().any(|event| matches!(
            event,
            minos_ui_protocol::UiEventMessage::MessageStarted { .. }
        )));
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
            "INSERT INTO threads(thread_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
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
            "INSERT INTO threads(thread_id, conversation_id, workspace_root, agent, status, last_seq, started_at, last_activity_at) \
             VALUES ('thr-snapshot', 'c-snapshot', ?, 'codex', 'idle', 0, 10, 20)",
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
                None,
                Some("c-live".into()),
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
            "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart', provider_session_id = 'thr-r' WHERE thread_id = 'thr-r'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();

        let response = test.glue.resume_thread("thr-r", false).await.unwrap();
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
                ThreadState::Idle,
                0,
            )
            .await
            .unwrap();

        test.glue.shutdown().await.unwrap();

        let row = test
            .glue
            .store
            .get_thread("thr-stop")
            .await
            .unwrap()
            .unwrap();
        // Idle between turns must not become user-visible Paused after stop.
        assert_eq!(row.status, "idle");
        assert!(row.last_pause_reason.is_none());
        assert!(!row.needs_continue);
        // In-process manager still parks as Suspended so children tear down cleanly.
        assert!(matches!(
            test.glue.manager.list_threads().await[0].state,
            ThreadState::Suspended {
                reason: minos_agent_runtime::PauseReason::DaemonRestart
            }
        ));
    }

    #[tokio::test]
    async fn resume_thread_recovers_non_codex_provider_session_id_from_events() {
        let test = test_glue().await;
        seed_thread(&test.glue, "thr-g", "gemini", 10, 20).await;
        sqlx::query(
            "UPDATE threads SET status = 'suspended', last_pause_reason = 'daemon_restart', provider_session_id = 'thr-g', last_seq = 1 WHERE thread_id = 'thr-g'",
        )
        .execute(test.glue.store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO events(thread_id, seq, body_kind, body_inline, projection_json, ts_ms, source) VALUES (?, 1, 'inline', ?, ?, 25, 'live')",
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

        let response = test.glue.resume_thread("thr-g", false).await.unwrap();

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
