use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use minos_agent_runtime::{
    AgentManager, AgentRuntimeConfig, InstanceCaps, ManagerEvent, RawIngest, SessionPolicies,
    SessionState,
};
use minos_chat_store::mcp_socket::{SocketRequest, SocketResponse};
use minos_codex_protocol::SkillsListResponse as CodexSkillsListResponse;
use minos_domain::{AgentName, MinosError};
use minos_protocol::{
    AgentDispatchRequest, AgentDispatchResponse, ApprovalDecisionRequest,
    CloseReason as ProtoCloseReason, CloseSessionRequest, GetSessionParams, GetSessionResponse,
    HostSkillError, HostSkillSummary, HostSkillsEntry, InterruptSessionRequest,
    ListHostSkillsRequest, ListHostSkillsResponse, ListHostWorkspacesRequest,
    ListHostWorkspacesResponse, ListSessionsParams, ListSessionsResponse, LocalConversationEvent,
    LocalIngestFrame, LocalManagerEvent, PauseReason as ProtoPauseReason, SendUserMessageRequest,
    SessionState as ProtoSessionState, SessionSummary, StartAgentRequest, StartAgentResponse,
    WriteHostSkillConfigRequest, WriteHostSkillConfigResponse,
};
use minos_ui_protocol::SessionEndReason;
use tokio::sync::{broadcast, watch};

use crate::ingest_chunk::IngestChunk;
use crate::ingest_coalescer::{IngestCoalescer, PreparedIngest};
use crate::ingest_sync::IngestSyncHandle;
use crate::store::event_writer::{provider_session_id_from_ingest, EventWriter};
use crate::store::{ChatMessageRow, ConversationRow, EventRow, LocalStore, SessionRow};
use crate::subscription::{AgentStateObserver, Subscription};

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
/// The single-session `AgentRuntime` was retired; the existing daemon
/// FFI surface (`StartAgentRequest` / `SendUserMessageRequest` / `stop_agent`
/// / `state_stream`) is preserved here as a thin shim over multi-session
/// `AgentManager`.
pub struct AgentGlue {
    pub manager: Arc<AgentManager>,
    pub writer: Arc<EventWriter>,
    /// Local SQLite store. Owned so `start_agent` / `close_session` can keep
    /// the parent `sessions` / `workspaces` rows in sync with the in-memory
    /// `AgentManager`. Without these the events FK fails the
    /// moment codex emits its first ingest frame.
    store: Arc<LocalStore>,
    /// Watch channel mirroring the most recently observed session state. The
    /// FFI surface exposes a single `state_stream()`. Multi-thread fan-out is not yet wired.
    state_tx: Arc<watch::Sender<SessionState>>,
    state_rx: watch::Receiver<SessionState>,
    persisted_ingest_tx: broadcast::Sender<LocalIngestFrame>,
    local_manager_event_tx: broadcast::Sender<LocalManagerEvent>,
    local_conversation_event_tx: broadcast::Sender<LocalConversationEvent>,
    ingest_sync: Arc<StdMutex<Option<IngestSyncHandle>>>,
    /// Default workspace dir used when `start_agent` is invoked without a
    /// workspace param. Resolved once at construction time.
    default_workspace: PathBuf,
    /// Conversation completion projector (local agent-result writeback).
    completion: crate::conversation_completion::ConversationCompletion,
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
        // Spawn the bridge: every durable RawIngest from the manager is
        // forwarded to the EventWriter (persist + formal host realtime uplink).
        //
        // Each ingest gets one info-level log line so the daemon log shows
        // the codex → host event stream. Volume is bounded by codex's own
        // emit rate (~tens/s/thread).
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
        let manager_for_ingest = manager.clone();
        // Commit-then-live-upload, seq assigned only inside SQLite, parent-missing
        // frames buffered (never silently dropped after a burned seq).
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    maybe = rx.recv() => {
                        let Some(ingest) = maybe else { break; };
                        // A failed older write owns the head of the commit lane.
                        // Never let a newly received provider frame allocate seq first.
                        let commit_lane_ready = Box::pin(drain_prepared_queue(
                            &writer_clone,
                            &coalescer_clone,
                            &ingest_sync_clone,
                            &persisted_ingest_tx_clone,
                            &completion_for_ingest,
                            &manager_for_ingest,
                        ))
                        .await;
                        match coalescer_clone.admit(ingest).await {
                            Ok(Some(prepared)) => {
                                if commit_lane_ready {
                                    if !commit_prepared_ingest(
                                        &writer_clone,
                                        &ingest_sync_clone,
                                        &persisted_ingest_tx_clone,
                                        &completion_for_ingest,
                                        prepared.clone(),
                                    )
                                    .await
                                    {
                                        if let Err(full) =
                                            coalescer_clone.requeue_write_failure(prepared).await
                                        {
                                            fail_session_ingest_queue_full(
                                                &manager_for_ingest,
                                                &full,
                                            )
                                            .await;
                                        }
                                    }
                                } else if let Err(full) =
                                    coalescer_clone.requeue_write_failure(prepared).await
                                {
                                    fail_session_ingest_queue_full(&manager_for_ingest, &full)
                                        .await;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                if let Some(full) =
                                    error.downcast_ref::<crate::ingest_coalescer::IngestQueueFull>()
                                {
                                    fail_session_ingest_queue_full(
                                        &manager_for_ingest,
                                        full,
                                    )
                                    .await;
                                } else {
                                    tracing::error!(
                                        target: "minos_daemon::agent",
                                        error = %error,
                                        "failed to prepare ingest event",
                                    );
                                }
                            }
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                        Box::pin(drain_prepared_queue(
                            &writer_clone,
                            &coalescer_clone,
                            &ingest_sync_clone,
                            &persisted_ingest_tx_clone,
                            &completion_for_ingest,
                            &manager_for_ingest,
                        ))
                        .await;
                    }
                }
            }
        });

        let (state_tx, state_rx) = watch::channel(SessionState::Idle);
        let state_tx = Arc::new(state_tx);
        let mut manager_events = manager.manager_event_stream();
        let store_clone = store.clone();
        let manager_for_lifecycle = manager.clone();
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
                                    // The ingest actor observes the new parent on its next
                                    // poll and remains the sole owner of commit ordering.
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
                                // Suspend affected threads and run completion flush for
                                // pending source deliveries. Product: no partial
                                // agent-result write on instance death (Suspended arm).
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
                                    completion_for_state
                                        .on_session_state(&session_id, &state)
                                        .await;
                                    let _ = state_tx_clone.send(state.clone());
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Never only log: lag drops Idle/Closed/Crash which must
                        // still drive completion + SQLite. Full reconcile from
                        // manager snapshot + active SQLite rows.
                        reconcile_manager_lifecycle_after_lag(
                            &manager_for_lifecycle,
                            &store_clone,
                            &completion_for_state,
                            &local_manager_event_tx_clone,
                            &state_tx_clone,
                            skipped,
                        )
                        .await;
                    }
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
            completion,
        }
    }

    pub fn set_ingest_sync(&self, sync: IngestSyncHandle) {
        if let Ok(mut guard) = self.ingest_sync.lock() {
            *guard = Some(sync);
        }
    }

    /// Wire Host `/ws/host` outbound so mailbox turn completion can emit
    /// `AppendBotMessage`. Call after `RelayClient` is constructed.
    pub fn set_host_outbound(
        &self,
        tx: tokio::sync::mpsc::Sender<minos_protocol::realtime::ClientFrame>,
    ) {
        self.completion.set_host_outbound(tx);
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
            req.profile_id.as_deref(),
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
        self.start_agent_with_session_id_in_conversation(
            session_id,
            req,
            initial_user_message,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
        )
        .await
    }

    /// Start agent with a fixed session id and optional Hub collaboration identity.
    ///
    /// When `conversation_id` is set (Mobile/Hub dispatch), the session is bound to
    /// **that exact id** under a resolved local project — never
    /// `ensure_workspace_conversation` / "Direct agent sessions".
    ///
    /// `origin_message_id` pins frozen agent-result id suffix for collab turns.
    /// `delivery_id` + `bot_id` pin mailbox context so completion emits AppendBotMessage.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_agent_with_session_id_in_conversation(
        &self,
        session_id: String,
        req: StartAgentRequest,
        initial_user_message: Option<String>,
        conversation_id: Option<String>,
        project_id: Option<String>,
        conversation_title: Option<String>,
        origin_message_id: Option<String>,
        attachments: Vec<minos_protocol::DispatchAttachment>,
        delivery_id: Option<String>,
        bot_id: Option<String>,
    ) -> Result<StartAgentResponse, MinosError> {
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

        let cloud_conversation_id = conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(conv_id) = cloud_conversation_id {
            if let Err(error) = ensure_hub_collaboration_conversation(
                &self.store,
                conv_id,
                project_id.as_deref(),
                conversation_title.as_deref(),
                Some(cwd.as_str()),
            )
            .await
            {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %error,
                    conversation_id = %conv_id,
                    "ensure_hub_collaboration_conversation failed; session may lack parent rows"
                );
            }
            self.persist_thread_parent_rows_in_conversation(
                &session_id,
                conv_id,
                &cwd,
                req.agent,
                bot_id.as_deref().or(req.profile_id.as_deref()),
                outcome.provider_session_id.as_deref(),
            )
            .await;
        } else {
            // True local direct-agent path (no Hub conversation).
            self.persist_thread_parent_rows(
                &session_id,
                &cwd,
                req.agent,
                bot_id.as_deref().or(req.profile_id.as_deref()),
                outcome.provider_session_id.as_deref(),
            )
            .await;
        }

        // Hub collab: require canonical origin (no message_key/t{ms} fallback).
        if cloud_conversation_id.is_some() {
            self.completion
                .note_require_canonical_origin(&session_id)
                .await;
        }
        // Pin origin before the turn runs so completion never falls back to
        // message_key/t{ms} when Hub origin is known.
        if let Some(origin) = origin_message_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.completion.note_turn_origin(&session_id, origin).await;
        }
        // Mailbox delivery context → completion emits AppendBotMessage on final text.
        if let (Some(delivery), Some(bot)) = (
            delivery_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            bot_id.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        ) {
            self.completion
                .note_mailbox_delivery(&session_id, delivery, bot)
                .await;
        }

        let paths = crate::media_materialize::materialize_attachments(
            &outcome.cwd,
            origin_message_id.as_deref(),
            &attachments,
        )
        .await;
        let prompt = crate::media_materialize::append_attachment_paths(
            initial_user_message.as_deref().unwrap_or(""),
            &paths,
        );
        if !prompt.trim().is_empty() {
            self.manager
                .send_user_message(&session_id, prompt)
                .await
                .map_err(map_anyhow)?;
        }

        let _ = self.state_tx.send(SessionState::Idle);
        tracing::info!(
            target: "minos_daemon::agent",
            profile_id = req.profile_id.as_deref().unwrap_or(""),
            agent = %agent_label(req.agent),
            session_id = %session_id,
            conversation_id = cloud_conversation_id.unwrap_or(""),
            origin_message_id = origin_message_id.as_deref().unwrap_or(""),
            delivery_id = delivery_id.as_deref().unwrap_or(""),
            bot_id = bot_id.as_deref().unwrap_or(""),
            "agent session started with fixed session_id"
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
        if let Some(origin) = req
            .origin_message_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.completion
                .note_turn_origin(&req.session_id, origin)
                .await;
        }
        if let (Some(delivery), Some(bot)) = (
            req.delivery_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            req.bot_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        ) {
            self.completion
                .note_mailbox_delivery(&req.session_id, delivery, bot)
                .await;
        }
        let cwd = self
            .manager
            .session_workspace(&req.session_id)
            .await
            .unwrap_or_else(|| self.default_workspace.clone());
        let paths = crate::media_materialize::materialize_attachments(
            &cwd,
            req.origin_message_id.as_deref(),
            &req.attachments,
        )
        .await;
        let text = crate::media_materialize::append_attachment_paths(&req.text, &paths);
        self.manager
            .send_user_message(&req.session_id, text)
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

    /// Dispatch a user message into an existing or newly created agent session.
    ///
    /// Matches [`Self::start_agent_with_session_id_in_conversation`] collab semantics:
    /// - non-empty `conversation_id` binds the session to that Hub conversation
    ///   (never invents "Direct agent sessions")
    /// - `origin_message_id` is pinned before the turn path when `session_id` is known
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
            conversation_id,
            origin_message_id,
            model,
            reasoning_effort,
            attachments,
        } = req;

        if let Some(existing_session_id) = session_id.as_deref() {
            self.ensure_thread_registered(existing_session_id).await?;
        }

        let cloud_conversation_id = conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        // Preserve an already-bound session conversation so we never invent Direct
        // agent sessions when re-dispatching into an existing collab thread.
        let prior_conversation_id = if let Some(existing_session_id) = session_id.as_deref() {
            match self.store.get_session(existing_session_id).await {
                Ok(Some(row)) => {
                    let cid = row.conversation_id.trim();
                    if cid.is_empty() {
                        None
                    } else {
                        Some(row.conversation_id)
                    }
                }
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %error,
                        session_id = %existing_session_id,
                        "store.get_session failed while resolving dispatch conversation binding",
                    );
                    None
                }
            }
        } else {
            None
        };

        let staged_origin = origin_message_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let workspace_path = resolve_workspace(&self.default_workspace, &workspace);
        let cwd_for_ensure = workspace_path.display().to_string();

        // Ensure Hub conversation exists before the turn can emit ingest events.
        if let Some(conv_id) = cloud_conversation_id.as_deref() {
            if let Err(error) = ensure_hub_collaboration_conversation(
                &self.store,
                conv_id,
                None,
                None,
                Some(cwd_for_ensure.as_str()),
            )
            .await
            {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %error,
                    conversation_id = %conv_id,
                    "ensure_hub_collaboration_conversation failed; session may lack parent rows"
                );
            }
        }

        // When the session id is already known, pin collab/origin before the turn
        // path starts so completion never races to message_key/t{ms} fallbacks.
        if let Some(existing_session_id) = session_id.as_deref() {
            if cloud_conversation_id.is_some() {
                self.completion
                    .note_require_canonical_origin(existing_session_id)
                    .await;
            }
            if let Some(origin) = staged_origin.as_deref() {
                self.completion
                    .note_turn_origin(existing_session_id, origin)
                    .await;
            }
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
        let paths = crate::media_materialize::materialize_attachments(
            &workspace_path,
            staged_origin.as_deref(),
            &attachments,
        )
        .await;
        let text = crate::media_materialize::append_attachment_paths(&text, &paths);

        // New sessions: start → parent rows + origin pin → send. Never start the
        // turn before completion knows the durable origin (collab agent-result).
        let outcome = if session_id.is_none() {
            let started = self
                .manager
                .start_agent_with_policies(
                    agent,
                    workspace_path.clone(),
                    policies.clone(),
                    launch.clone(),
                )
                .await
                .map_err(map_anyhow)?;
            let cwd = started.cwd.display().to_string();
            if let Some(conv_id) = cloud_conversation_id.as_deref() {
                self.persist_thread_parent_rows_in_conversation(
                    &started.session_id,
                    conv_id,
                    &cwd,
                    agent,
                    None,
                    started.provider_session_id.as_deref(),
                )
                .await;
                self.completion
                    .note_require_canonical_origin(&started.session_id)
                    .await;
            } else {
                self.persist_thread_parent_rows(
                    &started.session_id,
                    &cwd,
                    agent,
                    None,
                    started.provider_session_id.as_deref(),
                )
                .await;
            }
            if let Some(origin) = staged_origin.as_deref() {
                self.completion
                    .note_turn_origin(&started.session_id, origin)
                    .await;
            }
            self.manager
                .send_user_message(&started.session_id, text)
                .await
                .map_err(map_anyhow)?;
            minos_agent_runtime::DispatchOutcome {
                session_id: started.session_id,
                cwd: started.cwd,
                provider_session_id: started.provider_session_id,
            }
        } else {
            let outcome = self
                .manager
                .dispatch_message_with_options(
                    agent,
                    workspace_path,
                    session_id,
                    text,
                    policies,
                    launch,
                )
                .await
                .map_err(map_anyhow)?;
            let cwd = outcome.cwd.display().to_string();
            if let Some(conv_id) = cloud_conversation_id.as_deref() {
                self.persist_thread_parent_rows_in_conversation(
                    &outcome.session_id,
                    conv_id,
                    &cwd,
                    agent,
                    None,
                    outcome.provider_session_id.as_deref(),
                )
                .await;
            } else if let Some(conv_id) = prior_conversation_id.as_deref() {
                self.persist_thread_parent_rows_in_conversation(
                    &outcome.session_id,
                    conv_id,
                    &cwd,
                    agent,
                    None,
                    outcome.provider_session_id.as_deref(),
                )
                .await;
            } else {
                self.persist_thread_parent_rows(
                    &outcome.session_id,
                    &cwd,
                    agent,
                    None,
                    outcome.provider_session_id.as_deref(),
                )
                .await;
            }
            outcome
        };

        Ok(AgentDispatchResponse {
            session_id: outcome.session_id,
        })
    }

    async fn persist_thread_parent_rows(
        &self,
        session_id: &str,
        cwd: &str,
        agent: minos_domain::AgentName,
        bot_id: Option<&str>,
        provider_session_id: Option<&str>,
    ) {
        persist_thread_parent_rows_inner(
            &self.store,
            session_id,
            cwd,
            agent,
            bot_id,
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
        bot_id: Option<&str>,
        provider_session_id: Option<&str>,
    ) {
        persist_thread_parent_rows_inner(
            &self.store,
            session_id,
            cwd,
            agent,
            bot_id,
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
        // to `suspended { daemon_restart }` via startup recovery.
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

        let project_workspace = project
            .workspace_path
            .as_deref()
            .map(std::path::Path::new)
            .filter(|p| p.is_dir());
        let is_git = project_workspace
            .map(crate::git::exec::is_inside_work_tree)
            .unwrap_or(false);

        let requested_mode = req
            .git_mode
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase());
        let git_mode = match requested_mode.as_deref() {
            None if is_git => "worktree",
            None => "inherit",
            Some("worktree") => "worktree",
            Some("inherit") => "inherit",
            Some(other) => {
                return Err(MinosError::CodexProtocolError {
                    method: "create_conversation".into(),
                    message: format!("invalid git_mode '{other}'; expected worktree|inherit"),
                });
            }
        };

        // Resolve git binding: optional isolated worktree, else inherit snapshot.
        let mut branch = None;
        let mut worktree_path = None;
        let mut git_dirty = None;
        let mut git_head = None;
        let mut worktree_activity: Option<minos_protocol::GitActivity> = None;

        if git_mode == "worktree" {
            if let Some(ws) = project_workspace {
                if is_git {
                    match crate::git::create_conversation_worktree(ws, &conversation_id, title) {
                        Ok(wt) => {
                            branch = Some(wt.branch.clone());
                            worktree_path = Some(wt.path.to_string_lossy().into_owned());
                            if let Ok(live) = crate::git::detect_live_status(&wt.path) {
                                git_dirty = Some(live.dirty);
                                git_head = live.short_head.or(live.head);
                            }
                            worktree_activity =
                                Some(minos_protocol::GitActivity::WorktreeCreated {
                                    branch: wt.branch,
                                    worktree_path: wt.path.to_string_lossy().into_owned(),
                                    base_branch: wt.base_branch,
                                });
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "minos_daemon::agent",
                                error = %err,
                                project_id = %project.project_id,
                                "worktree create failed; falling back to inherit snapshot",
                            );
                            let (b, w) = crate::git::detect_git_snapshot(ws);
                            branch = b;
                            worktree_path = w;
                        }
                    }
                }
            }
        } else if let Some(ws) = project_workspace {
            let (b, w) = crate::git::detect_git_snapshot(ws);
            branch = b;
            worktree_path = w;
            if is_git {
                if let Ok(live) = crate::git::detect_live_status(ws) {
                    git_dirty = Some(live.dirty);
                    git_head = live.short_head.or(live.head);
                    if branch.is_none() {
                        branch = live.branch;
                    }
                }
            }
        }

        let effective_git_mode = if worktree_path.is_some() && git_mode == "worktree" {
            "worktree"
        } else {
            "inherit"
        };

        let meta = crate::store::ConversationCreateMeta {
            priority,
            progress: Some("todo".into()),
            branch,
            worktree_path,
            git_mode: Some(effective_git_mode.into()),
            git_dirty,
            git_head,
        };

        // Normalize + validate roster up front (membership gates @mention / start).
        // Wire still passes runtime labels; convert to stable local-rt bot_ids.
        let mut member_inputs: Vec<crate::store::ConversationAgentMemberInput> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for spec in &req.agents {
            let label = spec.agent.trim();
            if label.is_empty() || !seen.insert(label.to_ascii_lowercase()) {
                continue;
            }
            let agent = parse_agent_label(label).map_err(|e| MinosError::CodexProtocolError {
                method: "create_conversation".into(),
                message: format!("invalid agent member '{label}': {e}"),
            })?;
            let bot_id = self
                .store
                .ensure_local_runtime_bot(agent_label(agent), now_ms)
                .await
                .map_err(|e| map_store_error("create_conversation.ensure_bot", e))?;
            member_inputs.push(crate::store::ConversationAgentMemberInput {
                bot_id,
                brief: spec.brief.clone(),
            });
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
            .set_conversation_agent_members(&conversation_id, &member_inputs, now_ms)
            .await
            .map_err(|e| map_store_error("create_conversation.set_members", e))?;

        if let Some(activity) = worktree_activity {
            if let Err(e) = self
                .post_git_activity_message(&conversation_id, activity, None, None, now_ms)
                .await
            {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    conversation_id = %conversation_id,
                    "failed to post worktree_created activity",
                );
            }
        }

        if !member_inputs.is_empty() {
            let roster_rows = self
                .store
                .list_conversation_roster(&conversation_id)
                .await
                .unwrap_or_default();
            if let Err(e) = self
                .post_roster_system_message(
                    &conversation_id,
                    &crate::roster::format_roster_established_system_message(&roster_rows),
                    now_ms,
                )
                .await
            {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    conversation_id = %conversation_id,
                    "failed to post roster established system message",
                );
            }
            self.publish_roster_changed(&conversation_id).await;
        }

        tracing::info!(
            target: "minos_daemon::agent",
            project_id = %project.project_id,
            conversation_id = %conversation_id,
            branch = ?meta.branch,
            git_mode = %effective_git_mode,
            worktree_path = ?meta.worktree_path,
            agent_count = member_inputs.len(),
            "conversation created",
        );
        let row = self
            .store
            .get_conversation(&conversation_id)
            .await
            .map_err(|e| map_store_error("create_conversation.reload", e))?
            .expect("conversation inserted above");
        Ok(minos_protocol::CreateConversationResponse {
            conversation: self.conversation_summary_loaded(row).await?,
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
                conversation: self.conversation_summary_loaded(existing).await?,
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
        Ok(minos_protocol::UpdateConversationResponse {
            conversation: self.conversation_summary_loaded(row).await?,
        })
    }

    /// Add a runtime agent to the conversation roster (idempotent).
    /// Posts a conversation system message and injects host notices into idle
    /// top-level sessions already in the conversation.
    pub async fn add_conversation_agent(
        &self,
        req: minos_protocol::AddConversationAgentParams,
    ) -> Result<minos_protocol::AddConversationAgentResponse, MinosError> {
        let _existing = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("add_conversation_agent.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "add_conversation_agent".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;

        let agent =
            parse_agent_label(req.agent.trim()).map_err(|e| MinosError::CodexProtocolError {
                method: "add_conversation_agent".into(),
                message: format!("invalid agent '{}': {e}", req.agent),
            })?;
        let agent_label = agent_label(agent).to_owned();
        let now_ms = current_unix_ms();
        let bot_id = self
            .store
            .ensure_local_runtime_bot(&agent_label, now_ms)
            .await
            .map_err(|e| map_store_error("add_conversation_agent.ensure_bot", e))?;

        self.store
            .add_conversation_agent_member(
                &req.conversation_id,
                &bot_id,
                now_ms,
                req.brief.as_deref(),
            )
            .await
            .map_err(|e| map_store_error("add_conversation_agent.add", e))?;

        let mut brief_for_msg = req
            .brief
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if brief_for_msg.is_none() {
            if let Ok(Some(desc)) = self.store.bot_description(&bot_id).await {
                brief_for_msg = Some(desc);
            } else if let Ok(Some(desc)) = self
                .store
                .latest_profile_description_for_runtime(&agent_label)
                .await
            {
                brief_for_msg = Some(desc);
            }
        }
        let system_body = crate::roster::format_roster_joined_system_message(
            &agent_label,
            brief_for_msg.as_deref(),
        );
        let _ = self
            .post_roster_system_message(&req.conversation_id, &system_body, now_ms)
            .await;
        self.publish_roster_changed(&req.conversation_id).await;
        let change_summary = match brief_for_msg.as_deref() {
            Some(brief) => format!("Member **{agent_label}** joined ({brief})."),
            None => format!("Member **{agent_label}** joined the conversation."),
        };
        // Notify existing teammates (not the newcomer — they have no session yet).
        self.inject_roster_update_to_idle_sessions(&req.conversation_id, &change_summary, &[])
            .await;

        tracing::info!(
            target: "minos_daemon::agent",
            conversation_id = %req.conversation_id,
            agent = %agent_label,
            bot_id = %bot_id,
            "added conversation agent to roster",
        );

        let row = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("add_conversation_agent.reload", e))?
            .expect("conversation exists above");
        Ok(minos_protocol::AddConversationAgentResponse {
            conversation: self.conversation_summary_loaded(row).await?,
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

        let removed_bot_ids = self
            .store
            .remove_conversation_members_by_runtime(&req.conversation_id, &agent_label)
            .await
            .map_err(|e| map_store_error("remove_conversation_agent.remove", e))?;
        // Also try stable local-rt seed if runtime join found nothing (identity missing).
        let removed_bot_ids = if removed_bot_ids.is_empty() {
            let local_id = crate::store::local_runtime_bot_id(&agent_label);
            if self
                .store
                .remove_conversation_agent_member(&req.conversation_id, &local_id)
                .await
                .map_err(|e| map_store_error("remove_conversation_agent.remove_local", e))?
            {
                vec![local_id]
            } else {
                Vec::new()
            }
        } else {
            removed_bot_ids
        };
        if removed_bot_ids.is_empty() {
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
            let matches_bot = row
                .bot_id
                .as_deref()
                .map(|b| removed_bot_ids.iter().any(|id| id == b))
                .unwrap_or(false);
            let matches_runtime = row.agent == agent_label;
            if (!matches_bot && !matches_runtime) || row.status == "closed" {
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

        // Conversation system row (timeline) + idle-session host inject (session wire).
        let change = crate::roster::format_roster_removed_system_message(&agent_label);
        let _ = self
            .post_roster_system_message(&req.conversation_id, &change, now_ms)
            .await;
        self.publish_roster_changed(&req.conversation_id).await;
        self.inject_roster_update_to_idle_sessions(
            &req.conversation_id,
            &format!("Member **{agent_label}** left the conversation."),
            &[agent_label.as_str()],
        )
        .await;

        let row = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("remove_conversation_agent.reload", e))?
            .expect("conversation exists above");
        Ok(minos_protocol::RemoveConversationAgentResponse {
            conversation: self.conversation_summary_loaded(row).await?,
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
        let mut conversations = Vec::with_capacity(rows.len());
        for row in rows {
            conversations.push(self.conversation_summary_loaded(row).await?);
        }
        Ok(minos_protocol::ListConversationsResponse { conversations })
    }

    pub async fn list_conversation_roster(
        &self,
        req: minos_protocol::ListConversationRosterParams,
    ) -> Result<minos_protocol::ListConversationRosterResponse, MinosError> {
        let _ = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("list_conversation_roster.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "list_conversation_roster".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;
        let rows = self
            .store
            .list_conversation_roster(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("list_conversation_roster", e))?;
        let rows = self
            .store
            .enrich_roster_with_profile_briefs(rows)
            .await
            .map_err(|e| map_store_error("list_conversation_roster.enrich", e))?;
        Ok(minos_protocol::ListConversationRosterResponse {
            conversation_id: req.conversation_id,
            members: roster_members_from_rows(&rows)?,
        })
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
        // Agent rows store bot_id (identity), not runtime string.
        let bot_id = if req.sender_role == "agent" {
            if let Some(sid) = req.session_id.as_deref() {
                if let Ok(Some(session)) = self.store.get_session(sid).await {
                    if let Some(bid) = session.bot_id.filter(|s| !s.is_empty()) {
                        Some(bid)
                    } else {
                        self.store
                            .ensure_local_runtime_bot(&session.agent, now_ms)
                            .await
                            .ok()
                    }
                } else if let Some(agent) = req.agent {
                    self.store
                        .ensure_local_runtime_bot(agent_label(agent), now_ms)
                        .await
                        .ok()
                } else {
                    None
                }
            } else if let Some(agent) = req.agent {
                self.store
                    .ensure_local_runtime_bot(agent_label(agent), now_ms)
                    .await
                    .ok()
            } else {
                None
            }
        } else {
            None
        };
        let mentions_json = serde_json::to_string(&req.mentions).unwrap_or_else(|_| "[]".into());
        let message_seq = self
            .store
            .upsert_conversation_message(
                &req.conversation_id,
                &req.message_id,
                req.session_id.as_deref(),
                &req.sender_role,
                bot_id.as_deref(),
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

    // ── Git work-unit service ─────────────────────────────────────────────

    async fn resolve_git_checkout_path(
        &self,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        path: Option<&str>,
    ) -> Result<PathBuf, MinosError> {
        if let Some(raw) = path.map(str::trim).filter(|s| !s.is_empty()) {
            return Ok(PathBuf::from(raw));
        }
        if let Some(cid) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) {
            let conversation = self
                .store
                .get_conversation(cid)
                .await
                .map_err(|e| map_store_error("git.resolve_path.conversation", e))?
                .ok_or_else(|| MinosError::CodexProtocolError {
                    method: "git".into(),
                    message: format!("conversation not found: {cid}"),
                })?;
            let project = self
                .store
                .get_project(&conversation.project_id)
                .await
                .map_err(|e| map_store_error("git.resolve_path.project", e))?
                .ok_or_else(|| MinosError::CodexProtocolError {
                    method: "git".into(),
                    message: format!("project not found: {}", conversation.project_id),
                })?;
            return crate::git::resolve_work_path(
                conversation.worktree_path.as_deref(),
                project.workspace_path.as_deref(),
            )
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git".into(),
                message: format!("no usable git path for conversation {cid}"),
            });
        }
        if let Some(pid) = project_id.map(str::trim).filter(|s| !s.is_empty()) {
            let project = self
                .store
                .get_project(pid)
                .await
                .map_err(|e| map_store_error("git.resolve_path.project_only", e))?
                .ok_or_else(|| MinosError::CodexProtocolError {
                    method: "git".into(),
                    message: format!("project not found: {pid}"),
                })?;
            return project
                .workspace_path
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
                .ok_or_else(|| MinosError::CodexProtocolError {
                    method: "git".into(),
                    message: format!("project has no workspace path: {pid}"),
                });
        }
        Err(MinosError::CodexProtocolError {
            method: "git".into(),
            message: "conversation_id, project_id, or path is required".into(),
        })
    }

    async fn refresh_conversation_git_cache(
        &self,
        conversation_id: &str,
        workspace: &Path,
    ) -> Result<(), MinosError> {
        let live = crate::git::detect_live_status(workspace).map_err(|e| {
            MinosError::CodexProtocolError {
                method: "git.refresh".into(),
                message: e,
            }
        })?;
        let conversation = self
            .store
            .get_conversation(conversation_id)
            .await
            .map_err(|e| map_store_error("git.refresh.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git.refresh".into(),
                message: format!("conversation not found: {conversation_id}"),
            })?;
        let worktree_path = conversation.worktree_path.clone().or_else(|| {
            if live.is_linked_worktree {
                Some(live.path.to_string_lossy().into_owned())
            } else {
                None
            }
        });
        let now_ms = current_unix_ms();
        self.store
            .update_conversation_git_fields(
                conversation_id,
                live.branch.as_deref().or(conversation.branch.as_deref()),
                worktree_path.as_deref(),
                None,
                Some(live.dirty),
                live.short_head
                    .as_deref()
                    .or(live.head.as_deref())
                    .or(conversation.git_head.as_deref()),
                now_ms,
            )
            .await
            .map_err(|e| map_store_error("git.refresh.update", e))?;
        Ok(())
    }

    async fn post_git_activity_message(
        &self,
        conversation_id: &str,
        activity: minos_protocol::GitActivity,
        session_id: Option<&str>,
        agent: Option<AgentName>,
        now_ms: i64,
    ) -> Result<(i64, String, String), MinosError> {
        let body = crate::git::format_activity_body(&activity).map_err(|e| {
            MinosError::CodexProtocolError {
                method: "post_git_update".into(),
                message: e,
            }
        })?;
        let message_id = format!("git:{}:{}", conversation_id, uuid::Uuid::new_v4());
        let (sender_role, bot_id_opt) = if let Some(a) = agent {
            let bot_id = self
                .store
                .ensure_local_runtime_bot(agent_label(a), now_ms)
                .await
                .ok();
            ("agent", bot_id)
        } else {
            ("user", None)
        };
        // Agent rows require session_id + bot_id; fall back to user when session missing.
        let (sender_role, session_id, bot_id_opt) =
            if sender_role == "agent" && (session_id.is_none() || bot_id_opt.is_none()) {
                ("user", None, None)
            } else {
                (sender_role, session_id, bot_id_opt)
            };
        let message_seq = self
            .store
            .upsert_conversation_message(
                conversation_id,
                &message_id,
                session_id,
                sender_role,
                bot_id_opt.as_deref(),
                &body,
                now_ms,
                None,
                None,
                "[]",
            )
            .await
            .map_err(|e| map_store_error("post_git_update", e))?;
        self.publish_conversation_message_appended(conversation_id, message_seq);
        Ok((message_seq, message_id, body))
    }

    pub async fn git_get_status(
        &self,
        req: minos_protocol::GitStatusParams,
    ) -> Result<minos_protocol::GitStatusResponse, MinosError> {
        let path = self
            .resolve_git_checkout_path(
                req.conversation_id.as_deref(),
                req.project_id.as_deref(),
                req.path.as_deref(),
            )
            .await?;
        let live =
            crate::git::detect_live_status(&path).map_err(|e| MinosError::CodexProtocolError {
                method: "git_get_status".into(),
                message: e,
            })?;
        let conversation = if req.refresh_conversation {
            if let Some(cid) = req.conversation_id.as_deref() {
                self.refresh_conversation_git_cache(cid, &path).await?;
                let row = self
                    .store
                    .get_conversation(cid)
                    .await
                    .map_err(|e| map_store_error("git_get_status.reload", e))?
                    .ok_or_else(|| MinosError::CodexProtocolError {
                        method: "git_get_status".into(),
                        message: format!("conversation not found: {cid}"),
                    })?;
                Some(self.conversation_summary_loaded(row).await?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(minos_protocol::GitStatusResponse {
            path: live.path.to_string_lossy().into_owned(),
            branch: live.branch,
            head: live.head,
            short_head: live.short_head,
            dirty: live.dirty,
            has_untracked: live.has_untracked,
            ahead_count: live.ahead_count,
            behind_count: live.behind_count,
            upstream: live.upstream,
            is_linked_worktree: live.is_linked_worktree,
            conversation,
        })
    }

    pub async fn git_get_diff(
        &self,
        req: minos_protocol::GitDiffParams,
    ) -> Result<minos_protocol::GitDiffResponse, MinosError> {
        let path = self
            .resolve_git_checkout_path(
                req.conversation_id.as_deref(),
                req.project_id.as_deref(),
                req.path.as_deref(),
            )
            .await?;
        let diff =
            crate::git::get_diff(&path, req.base.as_deref(), req.head.as_deref()).map_err(|e| {
                MinosError::CodexProtocolError {
                    method: "git_get_diff".into(),
                    message: e,
                }
            })?;
        Ok(minos_protocol::GitDiffResponse {
            path: path.to_string_lossy().into_owned(),
            base: diff.base,
            head: diff.head,
            files: diff
                .files
                .into_iter()
                .map(|f| minos_protocol::GitDiffFile {
                    path: f.path,
                    status: f.status,
                    patch: f.patch,
                    truncated: f.truncated,
                })
                .collect(),
            patch: diff.patch,
            truncated: diff.truncated,
            file_count: diff.file_count,
        })
    }

    pub async fn git_create_worktree(
        &self,
        req: minos_protocol::GitCreateWorktreeParams,
    ) -> Result<minos_protocol::GitCreateWorktreeResponse, MinosError> {
        let conversation = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("git_create_worktree.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git_create_worktree".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;
        if conversation.worktree_path.is_some() && !req.force {
            return Err(MinosError::CodexProtocolError {
                method: "git_create_worktree".into(),
                message: "conversation already has a worktree; pass force=true to replace".into(),
            });
        }
        let project = self
            .store
            .get_project(&conversation.project_id)
            .await
            .map_err(|e| map_store_error("git_create_worktree.project", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git_create_worktree".into(),
                message: format!("project not found: {}", conversation.project_id),
            })?;
        let ws = project
            .workspace_path
            .as_deref()
            .map(Path::new)
            .filter(|p| p.is_dir())
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git_create_worktree".into(),
                message: "project has no workspace path".into(),
            })?;
        let wt =
            crate::git::create_conversation_worktree(ws, &req.conversation_id, &conversation.title)
                .map_err(|e| MinosError::CodexProtocolError {
                    method: "git_create_worktree".into(),
                    message: e,
                })?;
        let live = crate::git::detect_live_status(&wt.path).ok();
        let now_ms = current_unix_ms();
        self.store
            .update_conversation_git_fields(
                &req.conversation_id,
                Some(&wt.branch),
                Some(&wt.path.to_string_lossy()),
                Some("worktree"),
                live.as_ref().map(|l| l.dirty),
                live.as_ref()
                    .and_then(|l| l.short_head.as_deref().or(l.head.as_deref())),
                now_ms,
            )
            .await
            .map_err(|e| map_store_error("git_create_worktree.update", e))?;
        let activity = minos_protocol::GitActivity::WorktreeCreated {
            branch: wt.branch.clone(),
            worktree_path: wt.path.to_string_lossy().into_owned(),
            base_branch: wt.base_branch.clone(),
        };
        let _ = self
            .post_git_activity_message(&req.conversation_id, activity, None, None, now_ms)
            .await;
        let row = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("git_create_worktree.reload", e))?
            .expect("updated above");
        Ok(minos_protocol::GitCreateWorktreeResponse {
            conversation: self.conversation_summary_loaded(row).await?,
            created: wt.created,
            branch: wt.branch,
            worktree_path: wt.path.to_string_lossy().into_owned(),
        })
    }

    pub async fn git_remove_worktree(
        &self,
        req: minos_protocol::GitRemoveWorktreeParams,
    ) -> Result<minos_protocol::GitRemoveWorktreeResponse, MinosError> {
        let conversation = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("git_remove_worktree.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git_remove_worktree".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;
        let project = self
            .store
            .get_project(&conversation.project_id)
            .await
            .map_err(|e| map_store_error("git_remove_worktree.project", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git_remove_worktree".into(),
                message: format!("project not found: {}", conversation.project_id),
            })?;
        if req.delete_files {
            if let Some(wt) = conversation.worktree_path.as_deref() {
                if let Some(ws) = project.workspace_path.as_deref() {
                    if let Err(e) =
                        crate::git::remove_conversation_worktree(Path::new(ws), Path::new(wt))
                    {
                        tracing::warn!(
                            target: "minos_daemon::agent",
                            error = %e,
                            conversation_id = %req.conversation_id,
                            "worktree remove failed",
                        );
                    }
                }
            }
        }
        let now_ms = current_unix_ms();
        // Fall back to project workspace snapshot after detach.
        let (branch, _) = project
            .workspace_path
            .as_deref()
            .map(|p| crate::git::detect_git_snapshot(Path::new(p)))
            .unwrap_or((None, None));
        self.store
            .update_conversation_git_fields(
                &req.conversation_id,
                branch.as_deref(),
                None,
                Some("inherit"),
                None,
                None,
                now_ms,
            )
            .await
            .map_err(|e| map_store_error("git_remove_worktree.update", e))?;
        let row = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("git_remove_worktree.reload", e))?
            .expect("updated");
        Ok(minos_protocol::GitRemoveWorktreeResponse {
            conversation: self.conversation_summary_loaded(row).await?,
        })
    }

    pub async fn git_ensure_identity(
        &self,
        req: minos_protocol::GitEnsureIdentityParams,
    ) -> Result<minos_protocol::GitEnsureIdentityResponse, MinosError> {
        let path = self
            .resolve_git_checkout_path(
                req.conversation_id.as_deref(),
                req.project_id.as_deref(),
                req.path.as_deref(),
            )
            .await?;
        let id = crate::git::read_identity(&path);
        Ok(minos_protocol::GitEnsureIdentityResponse {
            path: path.to_string_lossy().into_owned(),
            name: id.name.clone(),
            email: id.email.clone(),
            complete: id.is_complete(),
        })
    }

    pub async fn git_push_branch(
        &self,
        req: minos_protocol::GitPushBranchParams,
    ) -> Result<minos_protocol::GitPushBranchResponse, MinosError> {
        let path = self
            .resolve_git_checkout_path(Some(&req.conversation_id), None, None)
            .await?;
        let id = crate::git::read_identity(&path);
        id.ensure_complete()
            .map_err(|e| MinosError::CodexProtocolError {
                method: "git_push_branch".into(),
                message: e,
            })?;
        let live =
            crate::git::detect_live_status(&path).map_err(|e| MinosError::CodexProtocolError {
                method: "git_push_branch".into(),
                message: e,
            })?;
        if live.dirty {
            return Err(MinosError::CodexProtocolError {
                method: "git_push_branch".into(),
                message: "working tree has uncommitted changes; commit or stash before push".into(),
            });
        }
        let branch = live.branch.ok_or_else(|| MinosError::CodexProtocolError {
            method: "git_push_branch".into(),
            message: "detached HEAD cannot be pushed as a branch".into(),
        })?;
        let remote = req
            .remote
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("origin");
        validate_git_remote_name(remote).map_err(|e| MinosError::CodexProtocolError {
            method: "git_push_branch".into(),
            message: e,
        })?;
        let mut args = vec!["push"];
        if req.set_upstream {
            args.push("-u");
        }
        args.push(remote);
        args.push(&branch);
        crate::git::exec::run_git(&path, &args).map_err(|e| MinosError::CodexProtocolError {
            method: "git_push_branch".into(),
            message: e,
        })?;
        let _ = self
            .refresh_conversation_git_cache(&req.conversation_id, &path)
            .await;
        Ok(minos_protocol::GitPushBranchResponse {
            branch,
            remote: remote.to_owned(),
            head: live.short_head.or(live.head),
            message: "pushed".into(),
        })
    }

    pub async fn git_open_pull_request(
        &self,
        req: minos_protocol::GitOpenPullRequestParams,
    ) -> Result<minos_protocol::GitOpenPullRequestResponse, MinosError> {
        let path = self
            .resolve_git_checkout_path(Some(&req.conversation_id), None, None)
            .await?;
        let conversation = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("git_open_pull_request.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git_open_pull_request".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;
        let live =
            crate::git::detect_live_status(&path).map_err(|e| MinosError::CodexProtocolError {
                method: "git_open_pull_request".into(),
                message: e,
            })?;
        let branch = live
            .branch
            .clone()
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "git_open_pull_request".into(),
                message: "detached HEAD cannot open a pull request".into(),
            })?;
        let title = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(conversation.title.as_str());
        let body = req.body.as_deref().unwrap_or("");
        let mut args = vec![
            "pr".to_owned(),
            "create".to_owned(),
            "--title".to_owned(),
            title.to_owned(),
            "--body".to_owned(),
            body.to_owned(),
            "--head".to_owned(),
            branch.clone(),
        ];
        if let Some(base) = req.base.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            args.push("--base".into());
            args.push(base.to_owned());
        }
        if req.draft {
            args.push("--draft".into());
        }
        // Prefer GitHub CLI when available.
        let output = std::process::Command::new("gh")
            .args(&args)
            .current_dir(&path)
            .output()
            .map_err(|e| MinosError::CodexProtocolError {
                method: "git_open_pull_request".into(),
                message: format!("failed to spawn gh: {e}"),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(MinosError::CodexProtocolError {
                method: "git_open_pull_request".into(),
                message: if stderr.is_empty() {
                    "gh pr create failed".into()
                } else {
                    stderr
                },
            });
        }
        let url = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("http"))
            .unwrap_or("")
            .to_owned();
        if url.is_empty() {
            return Err(MinosError::CodexProtocolError {
                method: "git_open_pull_request".into(),
                message: "gh pr create succeeded but returned no URL".into(),
            });
        }
        let number = url.rsplit('/').next().and_then(|s| s.parse::<u32>().ok());
        let now_ms = current_unix_ms();
        let activity = minos_protocol::GitActivity::PrOpened {
            url: url.clone(),
            number,
            title: Some(title.to_owned()),
        };
        let _ = self
            .post_git_activity_message(&req.conversation_id, activity, None, None, now_ms)
            .await;
        Ok(minos_protocol::GitOpenPullRequestResponse {
            url,
            number,
            branch,
            base: req.base,
        })
    }

    pub async fn post_git_update(
        &self,
        req: minos_protocol::PostGitUpdateParams,
    ) -> Result<minos_protocol::PostGitUpdateResponse, MinosError> {
        let _ = self
            .store
            .get_conversation(&req.conversation_id)
            .await
            .map_err(|e| map_store_error("post_git_update.get", e))?
            .ok_or_else(|| MinosError::CodexProtocolError {
                method: "post_git_update".into(),
                message: format!("conversation not found: {}", req.conversation_id),
            })?;
        let now_ms = current_unix_ms();
        // Best-effort: refresh dirty/head when activity implies local commits.
        if matches!(
            req.activity,
            minos_protocol::GitActivity::CommitsMade { .. }
                | minos_protocol::GitActivity::ReadyForReview { .. }
        ) {
            if let Ok(path) = self
                .resolve_git_checkout_path(Some(&req.conversation_id), None, None)
                .await
            {
                let _ = self
                    .refresh_conversation_git_cache(&req.conversation_id, &path)
                    .await;
            }
        }
        let (message_seq, message_id, body) = self
            .post_git_activity_message(
                &req.conversation_id,
                req.activity,
                req.session_id.as_deref(),
                req.agent,
                now_ms,
            )
            .await?;
        Ok(minos_protocol::PostGitUpdateResponse {
            message_seq,
            message_id,
            body,
        })
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
        let now_ms = current_unix_ms();
        // Resolve bot identity: profile_id is bot_id when set; else local-rt seed.
        let bot_id = if let Some(pid) = req
            .profile_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            // Ensure identity exists for launch resolution.
            if self
                .store
                .get_bot_identity(pid)
                .await
                .map_err(|e| map_store_error("start_agent_in_conversation.get_bot", e))?
                .is_none()
            {
                return Err(MinosError::CodexProtocolError {
                    method: "start_agent_in_conversation".into(),
                    message: format!("bot identity not found: {pid}"),
                });
            }
            pid.to_owned()
        } else {
            self.store
                .ensure_local_runtime_bot(agent_name, now_ms)
                .await
                .map_err(|e| map_store_error("start_agent_in_conversation.ensure_bot", e))?
        };
        let is_member = self
            .store
            .is_conversation_agent_member(&req.conversation_id, &bot_id)
            .await
            .map_err(|e| map_store_error("start_agent_in_conversation.is_member", e))?;
        let is_member = if is_member {
            true
        } else {
            // Offline convenience: membership may be the runtime seed while start
            // carries a named profile of the same runtime.
            self.store
                .is_member_by_runtime(&req.conversation_id, agent_name)
                .await
                .map_err(|e| map_store_error("start_agent_in_conversation.is_member_runtime", e))?
        };
        if !is_member {
            return Err(MinosError::CodexProtocolError {
                method: "start_agent_in_conversation".into(),
                message: format!(
                    "agent '{agent_name}' (bot_id={bot_id}) is not a member of this conversation; \
                     add it when creating the conversation"
                ),
            });
        }
        // Prefer conversation worktree when present; else explicit req; else project workspace.
        let workspace = if !req.workspace.trim().is_empty() {
            PathBuf::from(req.workspace.trim())
        } else if let Some(wt) = conversation
            .worktree_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
        {
            wt
        } else {
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
        // Refresh live git cache so UI dirty/branch stay current when work starts.
        if let Err(e) = self
            .refresh_conversation_git_cache(&req.conversation_id, &workspace)
            .await
        {
            tracing::debug!(
                target: "minos_daemon::agent",
                error = %e,
                conversation_id = %req.conversation_id,
                "git cache refresh skipped",
            );
        }
        let mut launch = self
            .resolve_launch_options(
                req.agent,
                req.profile_id.as_deref(),
                req.model.clone(),
                req.reasoning_effort.clone(),
                req.instructions.clone(),
            )
            .await?;
        // Inject conversation roster briefing into session-start instructions
        // (developer / append-system-prompt / rules — not a conversation user row).
        // Empty member briefs fall back to newest host profile description.
        if let Ok(rows) = self
            .store
            .list_conversation_roster(&req.conversation_id)
            .await
        {
            let rows = match self.store.enrich_roster_with_profile_briefs(rows).await {
                Ok(enriched) => enriched,
                Err(e) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %e,
                        conversation_id = %req.conversation_id,
                        "roster briefing: profile brief enrich failed; using raw roster",
                    );
                    // Re-fetch raw roster (previous vec was moved into enrich).
                    self.store
                        .list_conversation_roster(&req.conversation_id)
                        .await
                        .unwrap_or_default()
                }
            };
            let self_label = self
                .store
                .get_bot_identity(&bot_id)
                .await
                .ok()
                .flatten()
                .map(|b| b.display_name)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| agent_name.to_owned());
            let briefing = crate::roster::format_roster_briefing(&bot_id, &self_label, &rows);
            if let Some(merged) = crate::roster::merge_launch_instructions(
                launch.as_ref().and_then(|l| l.instructions.clone()),
                &briefing,
            ) {
                let mut opts = launch.unwrap_or_default();
                opts.instructions = Some(merged);
                launch = Some(opts);
            }
        }
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
            Some(bot_id.as_str()),
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
        // Profile description is the durable peer-facing role brief (same cap
        // as conversation roster briefs). Empty is allowed; teammates then see
        // "(no brief…)" until a description or per-conversation brief is set.
        let description = crate::store::normalize_roster_brief(Some(req.description.as_str()));
        let row = self
            .store
            .create_agent_profile(
                &id,
                name,
                &description,
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
        let description = crate::store::normalize_roster_brief(Some(req.description.as_str()));
        let row = self
            .store
            .update_agent_profile(
                &req.id,
                name,
                &description,
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

/// Remote names are passed as bare `git` args — reject option injection.
fn validate_git_remote_name(remote: &str) -> Result<(), String> {
    let remote = remote.trim();
    if remote.is_empty() {
        return Err("remote name must not be empty".into());
    }
    if remote.starts_with('-') {
        return Err(format!("invalid remote name (leading dash): {remote}"));
    }
    if remote.len() > 128 {
        return Err("remote name too long".into());
    }
    let ok = remote
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
    if !ok {
        return Err(format!("invalid remote name: {remote}"));
    }
    Ok(())
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

impl AgentGlue {
    async fn conversation_summary_loaded(
        &self,
        row: ConversationRow,
    ) -> Result<minos_protocol::LocalConversationSummary, MinosError> {
        let cid = row.conversation_id.clone();
        let members = self
            .store
            .list_conversation_roster(&cid)
            .await
            .map_err(|e| map_store_error("conversation_summary.roster", e))?;
        let members = self
            .store
            .enrich_roster_with_profile_briefs(members)
            .await
            .map_err(|e| map_store_error("conversation_summary.roster_enrich", e))?;
        conversation_summary_from_row(row, roster_members_from_rows(&members)?)
    }

    async fn post_roster_system_message(
        &self,
        conversation_id: &str,
        body: &str,
        now_ms: i64,
    ) -> Result<(), MinosError> {
        let message_id = format!("sys:roster:{}:{}", conversation_id, uuid::Uuid::new_v4());
        let message_seq = self
            .store
            .upsert_conversation_message(
                conversation_id,
                &message_id,
                None,
                "system",
                None,
                body,
                now_ms,
                None,
                None,
                "[]",
            )
            .await
            .map_err(|e| map_store_error("post_roster_system_message", e))?;
        self.publish_conversation_message_appended(conversation_id, message_seq);
        Ok(())
    }

    async fn publish_roster_changed(&self, conversation_id: &str) {
        let members = match self.store.list_conversation_roster(conversation_id).await {
            Ok(rows) => match roster_members_from_rows(&rows) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %e,
                        conversation_id = %conversation_id,
                        "roster_changed event skipped: bad agent label",
                    );
                    return;
                }
            },
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    conversation_id = %conversation_id,
                    "roster_changed event skipped: store error",
                );
                return;
            }
        };
        let _ = self
            .local_conversation_event_tx
            .send(LocalConversationEvent::RosterChanged {
                conversation_id: conversation_id.to_owned(),
                members,
            });
    }

    /// Push a host coordination notice into **idle** live sessions only.
    ///
    /// - Does **not** write a conversation user message (timeline already has
    ///   `sender_role=system` via [`Self::post_roster_system_message`]).
    /// - Skips running/suspended/closed/subagent sessions to avoid mid-turn noise.
    /// - Uses provider user-input channel with `[minos:host]` envelope.
    async fn inject_roster_update_to_idle_sessions(
        &self,
        conversation_id: &str,
        change_summary: &str,
        exclude_agents: &[&str],
    ) {
        let members = match self.store.list_conversation_roster(conversation_id).await {
            Ok(m) => match self.store.enrich_roster_with_profile_briefs(m).await {
                Ok(enriched) => enriched,
                Err(e) => {
                    tracing::warn!(
                        target: "minos_daemon::agent",
                        error = %e,
                        conversation_id = %conversation_id,
                        "idle roster inject: profile brief enrich failed; using raw roster",
                    );
                    self.store
                        .list_conversation_roster(conversation_id)
                        .await
                        .unwrap_or_default()
                }
            },
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    conversation_id = %conversation_id,
                    "idle roster inject skipped: list roster failed",
                );
                return;
            }
        };
        let sessions = match self
            .store
            .list_sessions_by_conversation(conversation_id)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    conversation_id = %conversation_id,
                    "idle roster inject skipped: list sessions failed",
                );
                return;
            }
        };

        for row in sessions {
            if row.status == "closed" || row.parent_session_id.is_some() {
                continue;
            }
            if exclude_agents
                .iter()
                .any(|a| a.eq_ignore_ascii_case(row.agent.as_str()))
            {
                continue;
            }
            // Production path: use watch receiver (session_state is test-only).
            let Some(rx) = self.manager.session_state_stream(&row.session_id).await else {
                continue;
            };
            let state = rx.borrow().clone();
            if !matches!(state, SessionState::Idle) {
                tracing::debug!(
                    target: "minos_daemon::agent",
                    session_id = %row.session_id,
                    agent = %row.agent,
                    ?state,
                    "skip roster host inject: session not idle",
                );
                continue;
            }
            let self_label = row
                .bot_id
                .as_deref()
                .and_then(|bid| {
                    members
                        .iter()
                        .find(|m| m.bot_id == bid)
                        .map(crate::roster::member_label)
                })
                .unwrap_or(row.agent.as_str());
            let body = crate::roster::format_roster_host_session_inject(
                self_label,
                &members,
                change_summary,
            );
            if let Err(e) = self.manager.send_user_message(&row.session_id, body).await {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    session_id = %row.session_id,
                    agent = %row.agent,
                    conversation_id = %conversation_id,
                    "roster host inject to idle session failed",
                );
            } else {
                tracing::info!(
                    target: "minos_daemon::agent",
                    session_id = %row.session_id,
                    agent = %row.agent,
                    conversation_id = %conversation_id,
                    "injected roster host notice into idle session",
                );
            }
        }
    }
}

fn conversation_summary_from_row(
    row: ConversationRow,
    roster: Vec<minos_protocol::ConversationRosterMember>,
) -> Result<minos_protocol::LocalConversationSummary, MinosError> {
    let participating_agents = roster.iter().map(|m| m.agent).collect();
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
        roster,
        priority: row.priority.filter(|p| !p.is_empty()),
        progress: if row.progress.is_empty() {
            "todo".into()
        } else {
            row.progress
        },
        branch: row.branch.filter(|b| !b.is_empty()),
        worktree_path: row.worktree_path.filter(|w| !w.is_empty()),
        git_mode: row.git_mode.filter(|m| !m.is_empty()),
        git_dirty: row.git_dirty.map(|v| v != 0),
        git_head: row.git_head.filter(|h| !h.is_empty()),
        running_count: u32::try_from(row.running_count.max(0)).unwrap_or(u32::MAX),
        needs_attention_count: u32::try_from(row.needs_attention_count.max(0)).unwrap_or(u32::MAX),
    })
}

fn roster_members_from_rows(
    rows: &[crate::store::ConversationAgentMemberRow],
) -> Result<Vec<minos_protocol::ConversationRosterMember>, MinosError> {
    rows.iter()
        .map(|r| {
            let runtime = r
                .runtime_agent
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    // local-rt-{runtime} seed convention
                    r.bot_id
                        .strip_prefix("local-rt-")
                        .unwrap_or(r.bot_id.as_str())
                });
            Ok(minos_protocol::ConversationRosterMember {
                bot_id: r.bot_id.clone(),
                agent: parse_agent_label(runtime)?,
                display_name: r.display_name.clone(),
                brief: r.brief.clone(),
                joined_at_ms: r.joined_at_ms,
            })
        })
        .collect()
}

fn local_conversation_message_from_row(
    row: ChatMessageRow,
    reactions: Vec<minos_protocol::LocalReactionGroup>,
) -> Result<minos_protocol::LocalConversationMessage, MinosError> {
    let git_activity = crate::git::parse_activity_body(&row.body);
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
    // Derive runtime badge from bot_id seed convention when possible.
    let agent = row.bot_id.as_deref().and_then(|bid| {
        let runtime = bid.strip_prefix("local-rt-").unwrap_or(bid);
        parse_agent_label(runtime).ok()
    });
    Ok(minos_protocol::LocalConversationMessage {
        message_seq: row.message_seq,
        message_id: row.message_id,
        conversation_id: row.conversation_id,
        session_id: row.session_id,
        created_at_ms: row.created_at_ms,
        sender_role: row.sender_role,
        bot_id: row.bot_id,
        agent,
        body: row.body,
        reply_to_message_id: row.reply_to_message_id,
        delegation_id: row.delegation_id,
        mentions,
        reactions,
        git_activity,
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

/// After broadcast lag, rebuild lifecycle effects from manager snapshot + SQLite.
///
/// Dropped Idle/Closed/Crash must not leave completion watches running or
/// sessions stuck as `running` in SQLite. Live manager state is authoritative
/// for in-memory handles; SQLite rows still `starting|running|resuming` but
/// absent from the manager are treated as instance-reaped.
async fn reconcile_manager_lifecycle_after_lag(
    manager: &AgentManager,
    store: &LocalStore,
    completion: &crate::conversation_completion::ConversationCompletion,
    local_tx: &broadcast::Sender<LocalManagerEvent>,
    state_tx: &watch::Sender<SessionState>,
    skipped: u64,
) {
    tracing::warn!(
        target: "minos_daemon::agent",
        skipped,
        "manager event bridge lagged; reconciling lifecycle from manager snapshot"
    );
    let at_ms = current_unix_ms();
    let live = manager.list_sessions().await;
    let live_ids: HashSet<String> = live.iter().map(|s| s.session_id.clone()).collect();

    for snap in &live {
        let state = &snap.state;
        // Same DaemonRestart race guard as the live SessionStateChanged path.
        let skip_persist = matches!(
            state,
            SessionState::Suspended {
                reason: minos_agent_runtime::PauseReason::DaemonRestart
            }
        );
        if !skip_persist {
            persist_runtime_state_inner(store, &snap.session_id, state, at_ms).await;
        }
        // Re-drive completion for states that flush bubbles / source delivery.
        if matches!(
            state,
            SessionState::Idle | SessionState::Closed { .. } | SessionState::Suspended { .. }
        ) {
            completion.on_session_state(&snap.session_id, state).await;
        }
        let _ = local_tx.send(LocalManagerEvent::SessionStateChanged {
            session_id: snap.session_id.clone(),
            old: state_to_proto(state),
            new: state_to_proto(state),
            at_ms,
        });
        let _ = state_tx.send(state.clone());
    }

    let rows = match store.list_sessions(None, Some(1000), None).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(
                target: "minos_daemon::agent",
                error = %error,
                "lifecycle reconcile failed to list SQLite sessions"
            );
            return;
        }
    };

    for row in rows {
        if live_ids.contains(&row.session_id) {
            continue;
        }
        // Idle/closed/suspended in DB without a live handle is normal after
        // process restarts; only mid-flight rows imply a missed Crash/Close.
        if !matches!(row.status.as_str(), "starting" | "running" | "resuming") {
            continue;
        }
        let state = SessionState::Suspended {
            reason: minos_agent_runtime::PauseReason::InstanceReaped,
        };
        tracing::warn!(
            target: "minos_daemon::agent",
            session_id = %row.session_id,
            prior_status = %row.status,
            "lifecycle reconcile: active SQLite session missing from manager; suspending"
        );
        persist_runtime_state_inner(store, &row.session_id, &state, at_ms).await;
        completion.on_session_state(&row.session_id, &state).await;
        let old = row_state_to_proto(&row).unwrap_or(ProtoSessionState::Idle);
        let _ = local_tx.send(LocalManagerEvent::SessionStateChanged {
            session_id: row.session_id.clone(),
            old,
            new: state_to_proto(&state),
            at_ms,
        });
        let _ = state_tx.send(state);
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
            // Pin: PostGitUpdate / delegate paths make this future large.
            Box::pin(handle_daemon_mcp_request(
                manager,
                store,
                db_path,
                default_workspace,
                local_conversation_event_tx,
                request,
            ))
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
        SocketRequest::ListConversationRoster { conversation_id } => {
            let rows = store.list_conversation_roster(&conversation_id).await?;
            let members = rows
                .into_iter()
                .map(|r| {
                    let runtime = r
                        .runtime_agent
                        .clone()
                        .or_else(|| r.bot_id.strip_prefix("local-rt-").map(str::to_owned))
                        .unwrap_or_else(|| r.bot_id.clone());
                    Ok(serde_json::json!({
                        "bot_id": r.bot_id,
                        "agent": runtime,
                        "display_name": r.display_name,
                        "brief": r.brief,
                        "joined_at_ms": r.joined_at_ms,
                    }))
                })
                .collect::<Result<Vec<_>, anyhow::Error>>()?;
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({
                    "conversation_id": conversation_id,
                    "members": members,
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
            let target_bot_id = if let Some(pid) = resolved_profile_id.as_deref() {
                pid.to_owned()
            } else {
                store
                    .ensure_local_runtime_bot(target_label, current_unix_ms())
                    .await?
            };
            anyhow::ensure!(
                store
                    .is_conversation_agent_member(&conversation_id, &target_bot_id)
                    .await?
                    || store
                        .is_member_by_runtime(&conversation_id, target_label)
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
                Some(target_bot_id.as_str()),
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
            let now_ms = current_unix_ms();
            let bot_id = if sender_role == "agent" {
                resolve_message_bot_id(
                    &store,
                    source_session_id.as_deref(),
                    source_agent.map(agent_label),
                    now_ms,
                )
                .await?
            } else {
                None
            };
            let message_seq = store
                .upsert_conversation_message(
                    &conversation_id,
                    &visible_message_id,
                    source_session_id.as_deref(),
                    sender_role,
                    bot_id.as_deref(),
                    &visible_prompt,
                    now_ms,
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
            let now_ms = current_unix_ms();
            let bot_id = if sender_role == "agent" {
                resolve_message_bot_id(
                    &store,
                    source_session_id.as_deref(),
                    source_agent.map(agent_label),
                    now_ms,
                )
                .await?
            } else {
                None
            };
            let message_seq = store
                .upsert_conversation_message(
                    &conversation_id,
                    &message_id,
                    source_session_id.as_deref(),
                    sender_role,
                    bot_id.as_deref(),
                    &text,
                    now_ms,
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
        SocketRequest::ReactToMessage {
            conversation_id,
            source_agent,
            source_session_id,
            message_id,
            emoji,
        } => {
            let source_agent = source_agent
                .as_deref()
                .map(parse_socket_agent)
                .transpose()?
                .ok_or_else(|| anyhow::anyhow!("react_to_message requires source_agent"))?;
            validate_mcp_source_session(
                &store,
                &conversation_id,
                Some(source_agent),
                source_session_id.as_deref(),
            )
            .await?;
            let emoji = emoji.trim();
            anyhow::ensure!(!emoji.is_empty(), "emoji must not be empty");
            anyhow::ensure!(
                emoji.chars().count() <= 32,
                "emoji must be at most 32 characters"
            );
            let Some((msg_conversation_id, body, mentions_json)) =
                store.get_message_body_for_reaction(&message_id).await?
            else {
                anyhow::bail!("message not found: {message_id}");
            };
            anyhow::ensure!(
                msg_conversation_id == conversation_id,
                "message {message_id} is not in this conversation"
            );
            // Hard gate: only react to messages that @mentioned this agent.
            anyhow::ensure!(
                message_mentions_agent(&body, &mentions_json, source_agent),
                "react_to_message is only allowed on messages that @mention this agent ({})",
                source_agent.bin_name()
            );
            let reaction_id = format!("rx-agent-{}", uuid::Uuid::new_v4());
            let now_ms = current_unix_ms();
            let actor_id = source_agent.bin_name().to_owned();
            let display_name = source_agent.bin_name().to_owned();
            let (cid, added) = store
                .toggle_message_reaction(
                    &message_id,
                    emoji,
                    &reaction_id,
                    &actor_id,
                    "agent",
                    &display_name,
                    now_ms,
                )
                .await?;
            let reaction_rows = store
                .list_reactions_for_messages(&[message_id.clone()])
                .await?;
            let reactions = aggregate_reactions_by_message(reaction_rows)
                .remove(&message_id)
                .unwrap_or_default();
            tracing::info!(
                target: "minos_daemon::agent",
                conversation_id = %cid,
                message_id = %message_id,
                agent = %actor_id,
                emoji = %emoji,
                added,
                "agent reacted to conversation message"
            );
            let _ = local_conversation_event_tx.send(
                LocalConversationEvent::ConversationReactionToggled {
                    conversation_id: cid.clone(),
                    message_id: message_id.clone(),
                    reactions: reactions.clone(),
                },
            );
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({
                    "accepted": true,
                    "added": added,
                    "message_id": message_id,
                    "emoji": emoji,
                    "reactions": reactions,
                })),
            })
        }
        SocketRequest::PostGitUpdate {
            conversation_id,
            source_agent,
            source_session_id,
            activity,
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
            let activity: minos_protocol::GitActivity = serde_json::from_value(activity)
                .map_err(|e| anyhow::anyhow!("invalid git activity payload: {e}"))?;
            // Use AgentGlue helpers via a short-lived path: open store rows already loaded.
            // Re-implement post path inline to avoid requiring full AgentGlue in this free function.
            let body =
                crate::git::format_activity_body(&activity).map_err(|e| anyhow::anyhow!(e))?;
            let message_id = format!("git:{}:{}", conversation_id, uuid::Uuid::new_v4());
            let now_ms = current_unix_ms();
            let (sender_role, bot_id_opt) = if let Some(a) = source_agent {
                let bot_id = store
                    .ensure_local_runtime_bot(agent_label(a), now_ms)
                    .await
                    .ok();
                ("agent", bot_id)
            } else {
                ("user", None)
            };
            let (sender_role, session_id, bot_id_opt) = if sender_role == "agent"
                && (source_session_id.is_none() || bot_id_opt.is_none())
            {
                ("user", None, None)
            } else {
                (sender_role, source_session_id.as_deref(), bot_id_opt)
            };
            let message_seq = store
                .upsert_conversation_message(
                    &conversation_id,
                    &message_id,
                    session_id,
                    sender_role,
                    bot_id_opt.as_deref(),
                    &body,
                    now_ms,
                    None,
                    None,
                    "[]",
                )
                .await?;
            // Best-effort live git cache refresh for commit/review milestones.
            if matches!(
                activity,
                minos_protocol::GitActivity::CommitsMade { .. }
                    | minos_protocol::GitActivity::ReadyForReview { .. }
            ) {
                if let Ok(conv) = store.get_conversation(&conversation_id).await {
                    if let Some(conv) = conv {
                        if let Ok(Some(project)) = store.get_project(&conv.project_id).await {
                            if let Some(path) = crate::git::resolve_work_path(
                                conv.worktree_path.as_deref(),
                                project.workspace_path.as_deref(),
                            ) {
                                if let Ok(live) = crate::git::detect_live_status(&path) {
                                    let _ = store
                                        .update_conversation_git_fields(
                                            &conversation_id,
                                            live.branch.as_deref().or(conv.branch.as_deref()),
                                            conv.worktree_path.as_deref(),
                                            None,
                                            Some(live.dirty),
                                            live.short_head
                                                .as_deref()
                                                .or(live.head.as_deref())
                                                .or(conv.git_head.as_deref()),
                                            current_unix_ms(),
                                        )
                                        .await;
                                }
                            }
                        }
                    }
                }
            }
            publish_conversation_message_appended(
                &local_conversation_event_tx,
                &conversation_id,
                message_seq,
            );
            Ok(SocketResponse::Ok {
                data: Some(serde_json::json!({
                    "accepted": true,
                    "message_id": message_id,
                    "message_seq": message_seq,
                })),
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
    // Roster is membership SSOT: a removed bot must not keep using MCP.
    let member_label = agent_label(session_agent);
    let is_member = if let Some(bot_id) = row.bot_id.as_deref().filter(|s| !s.is_empty()) {
        store
            .is_conversation_agent_member(conversation_id, bot_id)
            .await?
    } else {
        false
    };
    let is_member = if is_member {
        true
    } else {
        // Fallback: session may predate bot_id column population — match by runtime.
        store
            .is_member_by_runtime(conversation_id, member_label)
            .await?
    };
    anyhow::ensure!(
        is_member,
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
    let target_bot_id = store
        .ensure_local_runtime_bot(target_label, current_unix_ms())
        .await?;
    anyhow::ensure!(
        store
            .is_conversation_agent_member(conversation_id, &target_bot_id)
            .await?
            || store
                .is_member_by_runtime(conversation_id, target_label)
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
        Some(target_bot_id.as_str()),
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

/// True when the message body or structured mentions target this agent runtime.
/// Used to hard-gate `react_to_message` to only @-mentioned agents.
fn message_mentions_agent(body: &str, mentions_json: &str, agent: AgentName) -> bool {
    let bin = agent.bin_name();
    // Structured mentions_json (ConversationMention[]).
    if let Ok(mentions) =
        serde_json::from_str::<Vec<minos_protocol::ConversationMention>>(mentions_json)
    {
        if mentions.iter().any(|m| m.agent == agent) {
            return true;
        }
    }
    // Body @tokens: @codex / @codex#short / case-insensitive runtime name.
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
        let name_part = token.split_once('#').map(|(n, _)| n).unwrap_or(token);
        if name_part.eq_ignore_ascii_case(bin) {
            return true;
        }
        if parse_socket_agent(name_part).ok() == Some(agent) {
            return true;
        }
    }
    false
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

fn short_prefix(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn short_mcp_session_id(session_id: &str) -> String {
    short_prefix(session_id, 8)
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

async fn drain_prepared_queue(
    writer: &EventWriter,
    coalescer: &IngestCoalescer,
    ingest_sync: &StdMutex<Option<IngestSyncHandle>>,
    persisted_ingest_tx: &broadcast::Sender<LocalIngestFrame>,
    completion: &crate::conversation_completion::ConversationCompletion,
    manager: &AgentManager,
) -> bool {
    let prepared_list = match coalescer.drain_ready().await {
        Ok(list) => list,
        Err(error) => {
            tracing::error!(
                target: "minos_daemon::agent",
                error = %error,
                "failed to drain prepared ingest queue",
            );
            return false;
        }
    };
    let mut pending = std::collections::VecDeque::from(prepared_list);
    while let Some(prepared) = pending.pop_front() {
        if !commit_prepared_ingest(
            writer,
            ingest_sync,
            persisted_ingest_tx,
            completion,
            prepared.clone(),
        )
        .await
        {
            let mut restore = Vec::with_capacity(pending.len() + 1);
            restore.push(prepared);
            restore.extend(pending);
            if let Err(full) = coalescer.restore_write_queue_front(restore).await {
                fail_session_ingest_queue_full(manager, &full).await;
            }
            return false;
        }
    }
    true
}

/// Explicit session failure when ingest queues cannot accept more work.
/// Prefer user-visible terminal close over silent event loss.
async fn fail_session_ingest_queue_full(
    manager: &AgentManager,
    full: &crate::ingest_coalescer::IngestQueueFull,
) {
    tracing::error!(
        target: "minos_daemon::agent",
        session_id = %full.session_id,
        queue = full.queue,
        "ingest queue full; terminating session (no silent drop)"
    );
    if let Err(error) = manager.close_session(&full.session_id).await {
        tracing::warn!(
            target: "minos_daemon::agent",
            session_id = %full.session_id,
            error = %error,
            "close_session after queue full failed (session may already be gone)"
        );
    }
}

/// Commit locally first (seq allocated in DB), then broadcast + live upload.
/// On write failure, re-queue without burning a seq.
async fn commit_prepared_ingest(
    writer: &EventWriter,
    ingest_sync: &StdMutex<Option<IngestSyncHandle>>,
    persisted_ingest_tx: &broadcast::Sender<LocalIngestFrame>,
    completion: &crate::conversation_completion::ConversationCompletion,
    prepared: PreparedIngest,
) -> bool {
    let session_id = prepared.ingest.session_id.clone();
    let agent = prepared.ingest.agent;
    let ts_ms = prepared.ingest.ts_ms;
    let payload_bytes = prepared.ingest.body_len();
    let conversation_id = prepared.conversation_id.clone();

    const MAX_ATTEMPTS: u32 = 4;
    let mut last_err = None;
    for attempt in 0..MAX_ATTEMPTS {
        match writer.write_prepared(prepared.clone()).await {
            Ok(committed) => {
                let seq = committed.seq;
                let ui_events = committed.projection.clone();
                completion
                    .on_ingest_frame(&session_id, agent, &ui_events)
                    .await;
                let _ = persisted_ingest_tx.send(LocalIngestFrame {
                    session_id: session_id.clone(),
                    seq,
                    agent,
                    ui_events,
                    ts_ms,
                });
                // Live upload only after local commit; seq is the committed one.
                let chunk =
                    IngestChunk::new(prepared.ingest, seq, committed.projection, conversation_id);
                let sync = ingest_sync.lock().ok().and_then(|guard| guard.clone());
                if let Some(sync) = sync {
                    sync.submit_live(chunk).await;
                }
                tracing::info!(
                    target: "minos_daemon::agent",
                    session_id = %session_id,
                    seq,
                    bytes = payload_bytes,
                    attempt,
                    "ingest event committed",
                );
                return true;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < MAX_ATTEMPTS {
                    let backoff_ms = 20u64 * (1u64 << attempt.min(3));
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }

    tracing::error!(
        target: "minos_daemon::agent",
        error = %last_err
            .as_ref()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown".into()),
        session_id = %session_id,
        "EventWriter.write_prepared failed after retries; re-queued (no seq allocated)",
    );
    false
}

/// Resolve bot_id for an agent-authored conversation message write.
async fn resolve_message_bot_id(
    store: &LocalStore,
    session_id: Option<&str>,
    runtime_label: Option<&str>,
    now_ms: i64,
) -> anyhow::Result<Option<String>> {
    if let Some(sid) = session_id.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(session) = store.get_session(sid).await? {
            if let Some(bid) = session.bot_id.filter(|s| !s.is_empty()) {
                return Ok(Some(bid));
            }
            return Ok(Some(
                store
                    .ensure_local_runtime_bot(&session.agent, now_ms)
                    .await?,
            ));
        }
    }
    if let Some(runtime) = runtime_label.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(Some(store.ensure_local_runtime_bot(runtime, now_ms).await?));
    }
    Ok(None)
}

async fn persist_thread_parent_rows_inner(
    store: &LocalStore,
    session_id: &str,
    cwd: &str,
    agent: minos_domain::AgentName,
    bot_id: Option<&str>,
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
    // Prefer explicit bot_id; otherwise seed a stable local runtime bot.
    let owned_bot_id;
    let bot_id = match bot_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => match store
            .ensure_local_runtime_bot(agent_label(agent), now_ms)
            .await
        {
            Ok(id) => {
                owned_bot_id = id;
                owned_bot_id.as_str()
            }
            Err(e) => {
                tracing::warn!(
                    target: "minos_daemon::agent",
                    error = %e,
                    session_id = %session_id,
                    agent = %agent_label(agent),
                    "ensure_local_runtime_bot failed; session will lack bot_id",
                );
                ""
            }
        },
    };
    let bot_id_opt = if bot_id.is_empty() {
        None
    } else {
        Some(bot_id)
    };
    if let Err(e) = store
        .insert_session_in_conversation(
            session_id,
            conversation_id,
            cwd,
            agent_label(agent),
            bot_id_opt,
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
    let bot_id = match store
        .ensure_local_runtime_bot(agent_label(agent), now_ms)
        .await
    {
        Ok(id) => Some(id),
        Err(error) => {
            tracing::warn!(
                target: "minos_daemon::agent",
                error = %error,
                session_id,
                "ensure_local_runtime_bot failed for subagent",
            );
            None
        }
    };
    if let Err(error) = store
        .insert_session_in_conversation(
            session_id,
            &parent.conversation_id,
            workspace_root,
            agent_label(agent),
            bot_id.as_deref(),
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

/// Bind a **Hub** conversation id onto a local project without inventing
/// `conversation-{slug}` / "Direct agent sessions".
///
/// Project resolution order:
/// 1. existing conversation row (keep its project)
/// 2. explicit `project_id` (ensure row)
/// 3. project matching `workspace_path`
/// 4. create project from workspace folder name (stable id from slug)
async fn ensure_hub_collaboration_conversation(
    store: &LocalStore,
    conversation_id: &str,
    project_id: Option<&str>,
    title: Option<&str>,
    workspace_path: Option<&str>,
) -> anyhow::Result<()> {
    let now_ms = current_unix_ms();
    let title = title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Conversation");

    if store.get_conversation(conversation_id).await?.is_some() {
        // Already bound to a local project; only refresh placeholder titles.
        let _ = store
            .ensure_conversation(
                conversation_id,
                // project_id ignored when row exists
                "_",
                title,
                now_ms,
            )
            .await;
        return Ok(());
    }

    let cwd = workspace_path.map(str::trim).filter(|s| !s.is_empty());
    let resolved_project_id = if let Some(pid) = project_id.map(str::trim).filter(|s| !s.is_empty())
    {
        let name = cwd
            .map(project_name_from_path)
            .unwrap_or_else(|| "Project".into());
        let slug = cwd.map(workspace_slug).unwrap_or_else(|| "project".into());
        store.ensure_project(pid, &name, &slug, cwd, now_ms).await?;
        pid.to_string()
    } else if let Some(path) = cwd {
        if let Some(existing) = store.find_project_by_workspace_path(path).await? {
            existing.project_id
        } else {
            let slug = workspace_slug(path);
            let project_id = format!("project-{slug}");
            let name = project_name_from_path(path);
            store
                .ensure_project(&project_id, &name, &slug, Some(path), now_ms)
                .await?;
            project_id
        }
    } else {
        let project_id = format!(
            "project-hub-{}",
            short_prefix(conversation_id, 8)
        );
        store
            .ensure_project(&project_id, "Hub", "hub", None, now_ms)
            .await?;
        project_id
    };

    store
        .ensure_conversation(conversation_id, &resolved_project_id, title, now_ms)
        .await?;
    Ok(())
}

fn project_name_from_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Project")
        .to_string()
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
        let mut members = Vec::new();
        for a in agents {
            let bot_id = glue.store.ensure_local_runtime_bot(a, 1).await.unwrap();
            members.push(crate::store::ConversationAgentMemberInput {
                bot_id,
                brief: None,
            });
        }
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
                Some("local-rt-codex"),
                Some("thread-codex-1234"),
                None,
                "idle",
                1,
                true,
            )
            .await
            .unwrap();
        let (event_tx, mut event_rx) = broadcast::channel(4);

        let response = Box::pin(handle_daemon_mcp_request(
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
        ))
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
        assert_eq!(rows[0].bot_id.as_deref(), Some("local-rt-codex"));
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
                Some("local-rt-opencode"),
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

        let error = Box::pin(handle_daemon_mcp_request(
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
        ))
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
                Some("local-rt-claude"),
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
                Some("local-rt-codex"),
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

        let response = Box::pin(test.glue.remove_conversation_agent(
            minos_protocol::RemoveConversationAgentParams {
                conversation_id: "conversation-roster".into(),
                agent: "claude".into(),
            },
        ))
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
        let error = Box::pin(handle_daemon_mcp_request(
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
        ))
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

        // Provider path fails (no live Codex instance). End-state runtime rolls
        // the Idle→Running claim back to Idle so the session is not permanently
        // stuck; manager events still persist that Idle status into the store.
        let result = test
            .glue
            .manager
            .send_user_message("thr-live", "ping".into())
            .await;
        assert!(result.is_err());

        // Bridge is async under suite load — poll until store + live mirror settle.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let row = test
                .glue
                .store
                .get_session("thr-live")
                .await
                .unwrap()
                .unwrap();
            let live = test.glue.current_state();
            if row.status == "idle" && matches!(live, SessionState::Idle) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "expected idle after failed send; store={} live={live:?}",
                row.status
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
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

    /// End-to-end work-unit path: create conversation with worktree → resolve
    /// agent cwd to that worktree → refresh dirty/branch via git_get_status.
    #[tokio::test]
    async fn create_conversation_worktree_first_and_agent_cwd_uses_worktree() {
        use std::process::Command;

        let test = test_glue().await;
        let repo = test._tmp.path().join("project-repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git_ok = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .expect("git");
            assert!(st.success(), "git {args:?} failed");
        };
        git_ok(&["init", "-b", "main"]);
        git_ok(&["config", "user.email", "test@example.com"]);
        git_ok(&["config", "user.name", "test"]);
        std::fs::write(repo.join("README"), "hello").unwrap();
        git_ok(&["add", "README"]);
        git_ok(&["commit", "-m", "init"]);

        let project = test
            .glue
            .create_project(minos_protocol::CreateProjectRequest {
                name: "Repo Project".into(),
                workspace_slug: "repo-project".into(),
                workspace_path: Some(repo.to_string_lossy().into_owned()),
            })
            .await
            .expect("create project");

        let created = test
            .glue
            .create_conversation(minos_protocol::CreateConversationParams {
                project_id: project.project.project_id.clone(),
                title: "Auth Fix".into(),
                priority: None,
                agents: vec![minos_protocol::ConversationAgentSpec {
                    agent: "codex".into(),
                    brief: Some("implements features".into()),
                }],
                git_mode: Some("worktree".into()),
            })
            .await
            .expect("create conversation");

        let conv = created.conversation;
        assert_eq!(conv.git_mode.as_deref(), Some("worktree"));
        let worktree = conv
            .worktree_path
            .as_deref()
            .expect("worktree_path must be set");
        let worktree_path = PathBuf::from(worktree);
        assert!(
            worktree_path.is_dir(),
            "worktree dir missing: {}",
            worktree_path.display()
        );
        assert!(
            worktree_path.join(".git").is_file(),
            "expected linked worktree (.git file)"
        );
        let branch = conv.branch.as_deref().expect("branch");
        assert!(
            branch.starts_with("minos/"),
            "unexpected branch name: {branch}"
        );

        // Same resolution order as start_agent_in_conversation when req.workspace is empty.
        let agent_cwd = if let Some(wt) = conv
            .worktree_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
        {
            wt
        } else {
            repo.clone()
        };
        assert_eq!(
            agent_cwd.canonicalize().unwrap(),
            worktree_path.canonicalize().unwrap(),
            "agent cwd must be the conversation worktree"
        );

        // Dirty a file in the worktree and refresh via GitService.
        std::fs::write(worktree_path.join("README"), "dirty").unwrap();
        let status = test
            .glue
            .git_get_status(minos_protocol::GitStatusParams {
                conversation_id: Some(conv.conversation_id.clone()),
                project_id: None,
                path: None,
                refresh_conversation: true,
            })
            .await
            .expect("git status");
        assert!(status.dirty, "expected dirty after edit");
        assert_eq!(status.branch.as_deref(), Some(branch));
        assert!(status.is_linked_worktree);

        let refreshed = status
            .conversation
            .expect("refresh_conversation should return summary");
        assert_eq!(refreshed.git_dirty, Some(true));
        assert_eq!(refreshed.branch.as_deref(), Some(branch));
        assert_eq!(
            refreshed.worktree_path.as_deref(),
            Some(worktree_path.to_string_lossy().as_ref())
        );

        // Timeline should include worktree_created activity from create.
        let messages = test
            .glue
            .list_conversation_messages(minos_protocol::ListConversationMessagesParams {
                conversation_id: conv.conversation_id.clone(),
                before_seq: None,
                limit: Some(20),
            })
            .await
            .expect("list messages");
        let has_worktree_activity = messages.messages.iter().any(|m| {
            matches!(
                m.git_activity,
                Some(minos_protocol::GitActivity::WorktreeCreated { .. })
            )
        });
        assert!(
            has_worktree_activity,
            "expected worktree_created activity in timeline"
        );

        // inherit mode must not create a new worktree.
        let inherited = test
            .glue
            .create_conversation(minos_protocol::CreateConversationParams {
                project_id: project.project.project_id,
                title: "Shared tree".into(),
                priority: None,
                agents: vec![minos_protocol::ConversationAgentSpec {
                    agent: "codex".into(),
                    brief: Some("implements features".into()),
                }],
                git_mode: Some("inherit".into()),
            })
            .await
            .expect("create inherit conversation")
            .conversation;
        assert_eq!(inherited.git_mode.as_deref(), Some("inherit"));
        assert!(
            inherited.worktree_path.is_none(),
            "inherit mode should not bind a linked worktree"
        );
    }

    #[tokio::test]
    async fn dispatch_message_binds_hub_conversation_without_inventing_direct() {
        use minos_agent_runtime::test_support::FakeCodexBackend;

        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("hub-dispatch-ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let store = Arc::new(
            crate::store::LocalStore::open(&tmp.path().join("daemon.sqlite"))
                .await
                .unwrap(),
        );
        let writer = Arc::new(EventWriter::spawn(store.clone()));
        let glue = AgentGlue::wire_with(manager, writer, store.clone(), workspace.clone());

        let cloud_conversation_id = "hub-conv-dispatch-1";
        let origin = "origin-msg-dispatch-1";
        let response = glue
            .dispatch_message(AgentDispatchRequest {
                agent: AgentName::Codex,
                session_id: None,
                text: "hello hub dispatch".into(),
                workspace: workspace.display().to_string(),
                approval_policy: None,
                sandbox_policy: None,
                conversation_id: Some(cloud_conversation_id.into()),
                origin_message_id: Some(origin.into()),
                model: None,
                reasoning_effort: None,
                attachments: Vec::new(),
            })
            .await
            .expect("dispatch with hub conversation_id");

        let row = store
            .get_session(&response.session_id)
            .await
            .unwrap()
            .expect("session row");
        assert_eq!(row.conversation_id, cloud_conversation_id);
        let conv = store
            .get_conversation(cloud_conversation_id)
            .await
            .unwrap()
            .expect("hub conversation must be upserted with same id");
        assert_eq!(conv.conversation_id, cloud_conversation_id);

        // Must not invent Direct agent sessions for Hub-dispatched messages.
        // ensure_workspace_conversation uses workspace slug; assert no extra
        // conversation rows beyond the hub one for this session's binding.
        assert_eq!(row.conversation_id, cloud_conversation_id);

        fake.stop().await;
    }

    #[tokio::test]
    async fn dispatch_message_keeps_existing_session_conversation_binding() {
        use minos_agent_runtime::test_support::FakeCodexBackend;

        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("bound-dispatch-ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_root = workspace.display().to_string();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let store = Arc::new(
            crate::store::LocalStore::open(&tmp.path().join("daemon.sqlite"))
                .await
                .unwrap(),
        );
        let writer = Arc::new(EventWriter::spawn(store.clone()));
        let glue = AgentGlue::wire_with(manager, writer, store.clone(), workspace.clone());

        let bound_conversation_id = "hub-already-bound";
        let first = glue
            .dispatch_message(AgentDispatchRequest {
                agent: AgentName::Codex,
                session_id: None,
                text: "create bound session".into(),
                workspace: workspace_root.clone(),
                approval_policy: None,
                sandbox_policy: None,
                conversation_id: Some(bound_conversation_id.into()),
                origin_message_id: Some("origin-create".into()),
                model: None,
                reasoning_effort: None,
                attachments: Vec::new(),
            })
            .await
            .expect("initial hub dispatch");
        // Let the first turn settle to Idle so follow-up dispatch can send.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let response = glue
            .dispatch_message(AgentDispatchRequest {
                agent: AgentName::Codex,
                session_id: Some(first.session_id.clone()),
                text: "follow-up without conversation_id".into(),
                workspace: workspace_root,
                approval_policy: None,
                sandbox_policy: None,
                conversation_id: None,
                origin_message_id: Some("origin-followup".into()),
                model: None,
                reasoning_effort: None,
                attachments: Vec::new(),
            })
            .await
            .expect("dispatch existing bound session");

        assert_eq!(response.session_id, first.session_id);
        let row = store
            .get_session(&first.session_id)
            .await
            .unwrap()
            .expect("session row");
        assert_eq!(
            row.conversation_id, bound_conversation_id,
            "must keep prior hub binding and not invent Direct"
        );

        fake.stop().await;
    }
}
