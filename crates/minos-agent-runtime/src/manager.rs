// Module-local allow for the two `kill(2)` group-signalling calls in
// `shutdown_instances`. The crate-level `deny(unsafe_code)` keeps everything
// else honest.
#![allow(unsafe_code)]

use crate::approvals::NonApprovalContext;
use crate::codex_client::{CodexClient, Inbound};
use crate::config::McpConfig;
use crate::instance::AppServerInstance;
use crate::manager_event::ManagerEvent;
use crate::process::CodexProcess;
use crate::session_handle::SessionHandle;
use crate::state_machine::{PauseReason, SessionState};
use crate::{AgentKind, AgentRuntimeConfig, RawIngest};
use dashmap::DashMap;
use minos_codex_protocol::{
    ClientInfo, InitializeCapabilities, InitializeParams, InitializeResponse,
    InitializedNotification, ServerRequest, SkillsConfigWriteResponse, SkillsListResponse,
    ThreadStartParams, ThreadStartResponse,
};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tracing::{info, warn};
use url::Url;

pub const MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS: &str = "\
You are running inside Minos teamwork mode, where CLI coding agents work in a shared conversation with the user and other agents. \
Treat the Minos conversation as coordination context, not as a generic terminal session. \
When conversation history, teammate output, mentions, current chat state, or cross-agent coordination matters, use the `minos_teamwork` MCP server to inspect the bound conversation before answering. \
Use `list_conversation_messages` for recent conversation history, `list_conversation_roster` for the live agent roster and role briefs (do not assume startup snapshot is complete after roster changes), `delegate_to_agent` with `wait_delegation` when blocked on the result (or `get_delegation_status`/`cancel_delegation` for tracking), and `post_conversation_update` only for concise user-visible updates. \
When a user message @mentioned you and a short acknowledgement is enough (received, agreed, watching), prefer `react_to_message` with a single emoji (👍 ✅ 👀) instead of a full reply — the tool only allows reactions on messages that @mention you. \
When shipping code changes, work in the conversation worktree when present (do not edit the default branch directly) and post git milestones with `post_git_update` (commits_made, pr_opened, ready_for_review, checks_failed, merged).";

/// Host-owned one-shot prompt injected when a turn was interrupted by process
/// death. Synthesized as a normal user message for history consistency.
pub const CONTINUE_PROMPT: &str = "\
Continue from where you left off. The previous host process exited while this turn was in progress; \
resume any incomplete work without restarting from scratch.";

#[derive(Clone, Debug)]
pub struct InstanceCaps {
    pub max_instances: usize,
    pub idle_timeout: std::time::Duration,
}

/// Where an interactive approval reply must be delivered.
#[derive(Clone, Debug)]
pub(crate) enum PendingApprovalTarget {
    Codex {
        request_id: Value,
        request: Box<ServerRequest>,
        client: Arc<CodexClient>,
    },
    /// ACP `session/request_permission` (Gemini / Grok).
    Acp {
        request_id: Value,
        client: Arc<crate::acp_client::AcpClient>,
        allow_option_id: Option<String>,
        reject_option_id: Option<String>,
    },
    /// Grok ACP `ext_method` reverse-request (e.g. `x.ai/exit_plan_mode`).
    /// Reply body is `{ outcome, feedback? }`, not permission option ids.
    GrokExtMethod {
        request_id: Value,
        client: Arc<crate::acp_client::AcpClient>,
        nested_method: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct PendingApproval {
    pub session_id: String,
    pub target: PendingApprovalTarget,
}

pub(crate) type PendingApprovals = Arc<DashMap<String, PendingApproval>>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct InstanceKey {
    workspace: PathBuf,
    conversation_id: Option<String>,
    source_session_id: Option<String>,
}

impl InstanceKey {
    fn new(
        workspace: &Path,
        conversation_id: Option<&str>,
        source_session_id: Option<&str>,
    ) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            conversation_id: conversation_id.map(str::to_owned),
            source_session_id: source_session_id.map(str::to_owned),
        }
    }

    fn for_handle(handle: &SessionHandle) -> Self {
        let source_session_id = handle
            .mcp_conversation_id
            .as_ref()
            .map(|_| handle.session_id.as_str());
        Self::new(
            &handle.workspace,
            handle.mcp_conversation_id.as_deref(),
            source_session_id,
        )
    }
}

const DURABLE_INGEST_QUEUE_CAPACITY: usize = 1024;

#[derive(Clone)]
pub struct IngestSink {
    broadcast_tx: broadcast::Sender<RawIngest>,
    durable_tx: Arc<StdMutex<Option<mpsc::Sender<RawIngest>>>>,
}

impl IngestSink {
    #[must_use]
    pub fn new(broadcast_capacity: usize) -> Self {
        let (broadcast_tx, _) = broadcast::channel(broadcast_capacity);
        Self {
            broadcast_tx,
            durable_tx: Arc::new(StdMutex::new(None)),
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<RawIngest> {
        self.broadcast_tx.subscribe()
    }

    #[must_use]
    pub fn install_durable_stream(&self) -> mpsc::Receiver<RawIngest> {
        let (tx, rx) = mpsc::channel(DURABLE_INGEST_QUEUE_CAPACITY);
        *self
            .durable_tx
            .lock()
            .expect("durable ingest sink lock poisoned") = Some(tx);
        rx
    }

    pub async fn emit(&self, ingest: RawIngest) -> Result<(), IngestClosed> {
        let durable_tx = self
            .durable_tx
            .lock()
            .expect("durable ingest sink lock poisoned")
            .clone();
        if let Some(tx) = durable_tx {
            match tx.try_send(ingest.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(ingest)) => {
                    tx.send(ingest).await.map_err(|_| IngestClosed)?;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(IngestClosed);
                }
            }
        }
        if let Err(error) = self.broadcast_tx.send(ingest) {
            tracing::debug!(
                target: "minos_agent_runtime::manager",
                error = %error,
                "events broadcast send failed (no subscribers)",
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("durable ingest sink is closed")]
pub struct IngestClosed;

impl Default for InstanceCaps {
    fn default() -> Self {
        Self {
            max_instances: 8,
            idle_timeout: std::time::Duration::from_mins(30),
        }
    }
}

pub struct AgentManager {
    pub config: Arc<AgentRuntimeConfig>,
    pub caps: InstanceCaps,
    pub(crate) instances: Arc<Mutex<HashMap<InstanceKey, Arc<AppServerInstance>>>>,
    pub(crate) sessions: Arc<Mutex<HashMap<String, SessionHandle>>>,
    pub(crate) pending_approvals: PendingApprovals,
    pub(crate) events_tx: IngestSink,
    pub(crate) manager_tx: broadcast::Sender<ManagerEvent>,
    pub(crate) claude_sessions:
        Arc<Mutex<HashMap<String, crate::claude_driver::ClaudeNdjsonSession>>>,
    pub(crate) opencode_instances: Arc<
        Mutex<HashMap<InstanceKey, Arc<Mutex<crate::opencode_driver::OpencodeServerInstance>>>>,
    >,
    pub(crate) opencode_session_map: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) gemini_instances:
        Arc<Mutex<HashMap<String, Arc<crate::gemini_driver::GeminiAcpInstance>>>>,
    pub(crate) grok_instances:
        Arc<Mutex<HashMap<String, Arc<crate::grok_driver::GrokAcpInstance>>>>,
}

impl AgentManager {
    pub fn new(config: AgentRuntimeConfig, caps: InstanceCaps) -> Self {
        let events_tx = IngestSink::new(1024);
        let (manager_tx, _) = broadcast::channel(64);
        let mgr = Self {
            config: Arc::new(config),
            caps,
            instances: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            pending_approvals: Arc::new(DashMap::new()),
            events_tx,
            manager_tx,
            claude_sessions: Arc::new(Mutex::new(HashMap::new())),
            opencode_instances: Arc::new(Mutex::new(HashMap::new())),
            opencode_session_map: Arc::new(Mutex::new(HashMap::new())),
            gemini_instances: Arc::new(Mutex::new(HashMap::new())),
            grok_instances: Arc::new(Mutex::new(HashMap::new())),
        };
        mgr.spawn_reaper();
        mgr
    }

    fn spawn_reaper(&self) {
        let caps = self.caps.clone();
        let instances = self.instances.clone();
        let sessions = self.sessions.clone();
        let manager_tx = self.manager_tx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_mins(1));
            loop {
                tick.tick().await;
                let mut to_reap: Vec<InstanceKey> = Vec::new();
                {
                    let ig = instances.lock().await;
                    for (key, inst) in ig.iter() {
                        let last = *inst.last_activity_at.lock().await;
                        let idle = last.elapsed() >= caps.idle_timeout;
                        let tids = inst.session_ids().await;
                        let tg = sessions.lock().await;
                        let any_running = tids.iter().any(|t| {
                            tg.get(t).is_some_and(|h| {
                                matches!(h.current_state(), SessionState::Running { .. })
                            })
                        });
                        drop(tg);
                        if idle && !any_running {
                            to_reap.push(key.clone());
                        }
                    }
                }
                for key in to_reap {
                    Self::reap_static(&instances, &sessions, &manager_tx, &key).await;
                }
            }
        });
    }

    async fn reap_static(
        instances: &Arc<Mutex<HashMap<InstanceKey, Arc<AppServerInstance>>>>,
        sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
        manager_tx: &broadcast::Sender<ManagerEvent>,
        key: &InstanceKey,
    ) {
        let Some(inst) = instances.lock().await.remove(key) else {
            return;
        };
        let tids = inst.session_ids().await;
        let workspace = inst.workspace.clone();
        let tg = sessions.lock().await;
        for tid in &tids {
            if let Some(h) = tg.get(tid) {
                let _ = h.transition(SessionState::Suspended {
                    reason: PauseReason::InstanceReaped,
                });
            }
        }
        drop(tg);
        let _ = manager_tx.send(ManagerEvent::InstanceCrashed {
            workspace,
            affected_threads: tids,
            reason: PauseReason::InstanceReaped,
        });
        let child_opt = inst.child.lock().await.take();
        drop(inst);
        if let Some(mut child) = child_opt {
            let _ = child.kill().await;
        }
    }

    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest> {
        self.events_tx.subscribe()
    }

    pub fn install_durable_ingest_stream(&self) -> mpsc::Receiver<RawIngest> {
        self.events_tx.install_durable_stream()
    }

    pub fn manager_event_stream(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager_tx.subscribe()
    }

    pub async fn session_state_stream(
        &self,
        session_id: &str,
    ) -> Option<watch::Receiver<SessionState>> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .map(|h| h.state_rx.clone())
    }

    pub async fn has_thread(&self, session_id: &str) -> bool {
        self.sessions.lock().await.contains_key(session_id)
    }

    pub async fn session_provider_session_id(&self, session_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .and_then(|handle| handle.codex_session_id.clone())
    }

    pub async fn register_persisted_thread(
        &self,
        session_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        codex_session_id: Option<String>,
        parent_session_id: Option<String>,
        mcp_conversation_id: Option<String>,
        initial_state: SessionState,
        last_seq: u64,
    ) -> anyhow::Result<()> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or(workspace);
        let mut sessions = self.sessions.lock().await;
        if sessions.contains_key(&session_id) {
            return Ok(());
        }
        let mut handle = SessionHandle::new(
            session_id.clone(),
            canon.clone(),
            agent,
            initial_state.clone(),
            last_seq,
        );
        handle.codex_session_id = codex_session_id;
        handle.parent_session_id = parent_session_id.clone();
        handle.mcp_conversation_id = mcp_conversation_id;
        sessions.insert(session_id.clone(), handle);
        drop(sessions);
        let _ = self.manager_tx.send(ManagerEvent::SessionAdded {
            session_id,
            workspace: canon,
            agent,
            parent_session_id,
        });
        Ok(())
    }

    pub async fn dispatch_message(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        session_id: Option<String>,
        text: String,
        policies: Option<SessionPolicies>,
    ) -> anyhow::Result<DispatchOutcome> {
        self.dispatch_message_with_options(agent, workspace, session_id, text, policies, None)
            .await
    }

    pub async fn dispatch_message_with_options(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        session_id: Option<String>,
        text: String,
        policies: Option<SessionPolicies>,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<DispatchOutcome> {
        match session_id {
            None => {
                let outcome = self
                    .start_agent_with_policies(agent, workspace, policies, launch)
                    .await?;
                self.send_user_message(&outcome.session_id, text).await?;
                Ok(DispatchOutcome {
                    session_id: outcome.session_id,
                    cwd: outcome.cwd,
                    provider_session_id: outcome.provider_session_id,
                })
            }
            Some(session_id) => {
                let handle = self
                    .sessions
                    .lock()
                    .await
                    .get(&session_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?;
                match handle.current_state() {
                    SessionState::Idle => self.send_user_message(&session_id, text).await?,
                    SessionState::Running { .. } => {
                        self.send_user_message(&session_id, text).await?
                    }
                    SessionState::Suspended { .. } => {
                        self.send_user_message(&session_id, text).await?;
                    }
                    other => anyhow::bail!("dispatch_message rejected: state={other:?}"),
                }
                Ok(DispatchOutcome {
                    session_id,
                    cwd: handle.workspace.clone(),
                    provider_session_id: handle.codex_session_id.clone(),
                })
            }
        }
    }

    pub async fn start_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
    ) -> anyhow::Result<StartAgentOutcome> {
        self.start_agent_with_policies(agent, workspace, None, None)
            .await
    }

    pub async fn start_agent_in_conversation(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        conversation_id: String,
    ) -> anyhow::Result<StartAgentOutcome> {
        self.start_agent_with_policies_and_conversation(
            agent,
            workspace,
            None,
            None,
            Some(conversation_id),
        )
        .await
    }

    pub async fn start_agent_in_conversation_with_options(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        conversation_id: String,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<StartAgentOutcome> {
        self.start_agent_with_policies_and_conversation(
            agent,
            workspace,
            None,
            launch,
            Some(conversation_id),
        )
        .await
    }

    pub async fn start_agent_with_policies(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        policies: Option<SessionPolicies>,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<StartAgentOutcome> {
        self.start_agent_with_policies_and_conversation(agent, workspace, policies, launch, None)
            .await
    }

    async fn start_agent_with_policies_and_conversation(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        policies: Option<SessionPolicies>,
        launch: Option<AgentLaunchOptions>,
        conversation_id: Option<String>,
    ) -> anyhow::Result<StartAgentOutcome> {
        match agent {
            AgentName::Codex => {
                self.start_codex_agent(
                    agent,
                    workspace,
                    policies,
                    launch,
                    conversation_id.as_deref(),
                )
                .await
            }
            AgentName::Claude => {
                self.start_claude_agent(agent, workspace, None, conversation_id, launch)
                    .await
            }
            AgentName::Opencode => {
                self.start_opencode_agent(
                    agent,
                    workspace,
                    None,
                    conversation_id.as_deref(),
                    launch,
                )
                .await
            }
            AgentName::Gemini => {
                self.start_gemini_agent(agent, workspace, None, conversation_id.as_deref(), launch)
                    .await
            }
            AgentName::Grok => {
                self.start_grok_agent(agent, workspace, None, conversation_id.as_deref(), launch)
                    .await
            }
        }
    }

    pub async fn start_agent_with_session_id(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        session_id: String,
        policies: Option<SessionPolicies>,
    ) -> anyhow::Result<StartAgentOutcome> {
        self.start_agent_with_session_id_and_options(agent, workspace, session_id, policies, None)
            .await
    }

    pub async fn start_agent_with_session_id_and_options(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        session_id: String,
        policies: Option<SessionPolicies>,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<StartAgentOutcome> {
        if let Some(handle) = self.sessions.lock().await.get(&session_id).cloned() {
            return Ok(StartAgentOutcome {
                session_id,
                cwd: handle.workspace,
                provider_session_id: handle.codex_session_id,
            });
        }

        match agent {
            AgentName::Codex => {
                self.start_codex_agent_with_session_id(
                    agent,
                    workspace,
                    policies,
                    launch,
                    Some(session_id),
                    None,
                )
                .await
            }
            AgentName::Claude => {
                self.start_claude_agent(agent, workspace, Some(session_id), None, launch)
                    .await
            }
            AgentName::Opencode => {
                self.start_opencode_agent(agent, workspace, Some(session_id), None, launch)
                    .await
            }
            AgentName::Gemini => {
                self.start_gemini_agent(agent, workspace, Some(session_id), None, launch)
                    .await
            }
            AgentName::Grok => {
                self.start_grok_agent(agent, workspace, Some(session_id), None, launch)
                    .await
            }
        }
    }

    async fn start_codex_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        policies: Option<SessionPolicies>,
        launch: Option<AgentLaunchOptions>,
        conversation_id: Option<&str>,
    ) -> anyhow::Result<StartAgentOutcome> {
        self.start_codex_agent_with_session_id(
            agent,
            workspace,
            policies,
            launch,
            None,
            conversation_id,
        )
        .await
    }

    async fn start_codex_agent_with_session_id(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        policies: Option<SessionPolicies>,
        launch: Option<AgentLaunchOptions>,
        logical_session_id: Option<String>,
        conversation_id: Option<&str>,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let preallocated_session_id = logical_session_id
            .or_else(|| conversation_id.map(|_| uuid::Uuid::new_v4().to_string()));
        let source_session_id = conversation_id.and(preallocated_session_id.as_deref());
        let instance = self
            .ensure_instance(
                &canon,
                policies.as_ref(),
                conversation_id,
                source_session_id,
            )
            .await?;

        // Allocate a fresh thread on the codex app-server. The
        // `thread/started` notification arrives later via the event pump and
        // populates `codex_session_id` + flips state Starting -> Idle.
        let model = launch.as_ref().and_then(|l| l.model.clone());
        let effort = launch.as_ref().and_then(|l| l.reasoning_effort.clone());
        let instructions = launch.as_ref().and_then(|l| l.instructions.clone());
        let developer = match instructions.as_deref() {
            Some(extra) if !extra.trim().is_empty() => {
                format!(
                    "{}\n\n{}",
                    MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS,
                    extra.trim()
                )
            }
            _ => MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS.to_string(),
        };
        let resp = instance
            .start_thread_with_options(
                &canon,
                model.as_deref(),
                effort.as_deref(),
                Some(developer.as_str()),
            )
            .await?;
        let session_id = preallocated_session_id.unwrap_or_else(|| resp.codex_session_id.clone());
        instance.add_thread(session_id.clone()).await;
        instance.touch().await;

        let mut handle = SessionHandle::new(
            session_id.clone(),
            canon.clone(),
            agent,
            SessionState::Starting,
            0,
        )
        .with_full_launch_options(model, effort, instructions);
        handle.mcp_conversation_id = conversation_id.map(str::to_owned);
        handle.codex_session_id = Some(resp.codex_session_id.clone());
        let _ = handle.transition(SessionState::Idle);
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::SessionAdded {
            session_id: session_id.clone(),
            workspace: canon.clone(),
            agent,
            parent_session_id: None,
        });

        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.clone(),
            old: SessionState::Starting,
            new: SessionState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });

        Ok(StartAgentOutcome {
            session_id,
            cwd: canon,
            provider_session_id: Some(resp.codex_session_id),
        })
    }

    async fn start_claude_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        logical_session_id: Option<String>,
        conversation_id: Option<String>,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let session_id = logical_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let provider_session_id = uuid::Uuid::new_v4().to_string();
        let mut handle = SessionHandle::new(
            session_id.clone(),
            canon.clone(),
            agent,
            SessionState::Starting,
            0,
        )
        .with_full_launch_options(
            launch.as_ref().and_then(|l| l.model.clone()),
            launch.as_ref().and_then(|l| l.reasoning_effort.clone()),
            launch.as_ref().and_then(|l| l.instructions.clone()),
        );
        handle.codex_session_id = Some(provider_session_id.clone());
        handle.mcp_conversation_id = conversation_id;
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::SessionAdded {
            session_id: session_id.clone(),
            workspace: canon.clone(),
            agent,
            parent_session_id: None,
        });
        if let Some(h) = self.sessions.lock().await.get(&session_id) {
            let _ = h.transition(SessionState::Idle);
        }
        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.clone(),
            old: SessionState::Starting,
            new: SessionState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        Ok(StartAgentOutcome {
            session_id,
            cwd: canon,
            provider_session_id: Some(provider_session_id),
        })
    }

    async fn start_opencode_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        logical_session_id: Option<String>,
        conversation_id: Option<&str>,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let session_id = logical_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let source_session_id = conversation_id.map(|_| session_id.as_str());
        let instance = self
            .ensure_opencode_instance(&canon, conversation_id, source_session_id)
            .await?;
        let model = launch.as_ref().and_then(|l| l.model.clone());
        let oc_session_id = instance
            .lock()
            .await
            .create_session_with_model(model.as_deref())
            .await?;
        self.opencode_session_map
            .lock()
            .await
            .insert(session_id.clone(), oc_session_id.clone());
        let mut handle = SessionHandle::new(
            session_id.clone(),
            canon.clone(),
            agent,
            SessionState::Idle,
            0,
        )
        .with_full_launch_options(
            model,
            launch.as_ref().and_then(|l| l.reasoning_effort.clone()),
            launch.as_ref().and_then(|l| l.instructions.clone()),
        );
        handle.codex_session_id = Some(oc_session_id.clone());
        handle.mcp_conversation_id = conversation_id.map(str::to_owned);
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::SessionAdded {
            session_id: session_id.clone(),
            workspace: canon.clone(),
            agent,
            parent_session_id: None,
        });
        Ok(StartAgentOutcome {
            session_id,
            cwd: canon,
            provider_session_id: Some(oc_session_id),
        })
    }

    async fn start_gemini_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        logical_session_id: Option<String>,
        conversation_id: Option<&str>,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let session_id = logical_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let model = launch.as_ref().and_then(|l| l.model.clone());
        let provider_session_id = self
            .ensure_gemini_instance_for_thread(
                &session_id,
                &canon,
                None,
                conversation_id,
                model.as_deref(),
            )
            .await?;
        let mut handle = SessionHandle::new(
            session_id.clone(),
            canon.clone(),
            agent,
            SessionState::Idle,
            0,
        )
        .with_full_launch_options(
            model,
            launch.as_ref().and_then(|l| l.reasoning_effort.clone()),
            launch.as_ref().and_then(|l| l.instructions.clone()),
        );
        handle.codex_session_id = Some(provider_session_id.clone());
        handle.mcp_conversation_id = conversation_id.map(str::to_owned);
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::SessionAdded {
            session_id: session_id.clone(),
            workspace: canon.clone(),
            agent,
            parent_session_id: None,
        });
        Ok(StartAgentOutcome {
            session_id,
            cwd: canon,
            provider_session_id: Some(provider_session_id),
        })
    }

    async fn ensure_gemini_instance_for_thread(
        &self,
        session_id: &str,
        workspace: &Path,
        resume_session_id: Option<&str>,
        conversation_id: Option<&str>,
        model: Option<&str>,
    ) -> anyhow::Result<String> {
        if let Some(existing) = self.gemini_instances.lock().await.get(session_id).cloned() {
            return existing.get_session_id().await.ok_or_else(|| {
                anyhow::anyhow!("gemini ACP instance has no active session: {session_id}")
            });
        }

        let bin_path = self
            .config
            .gemini_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(AgentName::Gemini.bin_name()));
        let (crash_tx, _crash_rx) = tokio::sync::mpsc::channel::<()>(1);
        let instance = crate::gemini_driver::GeminiAcpInstance::spawn_with_model(
            &bin_path,
            workspace,
            &self.config.subprocess_env,
            crash_tx,
            model,
        )
        .await
        .map_err(|error| anyhow::anyhow!("gemini ACP spawn failed: {error}"))?;
        let instance = Arc::new(instance);
        let initialize = instance
            .initialize()
            .await
            .map_err(|error| anyhow::anyhow!("gemini ACP initialize failed: {error}"))?;
        if initialize.protocol_version != 1 {
            warn!(
                target: "minos_agent_runtime::manager",
                protocol_version = initialize.protocol_version,
                "gemini ACP returned unexpected protocol version",
            );
        }
        let mut resumed = false;
        let mut provider_session_id = None;
        let mcp_server = resolve_mcp_server(
            self.config.mcp.as_ref(),
            workspace,
            AgentName::Gemini,
            conversation_id,
            Some(session_id),
        );
        if let Some(session_id) = resume_session_id {
            match instance
                .resume_session(
                    session_id,
                    workspace,
                    Some(mcp_server.iter().map(gemini_mcp_server).collect()),
                )
                .await
            {
                Ok(_) => {
                    resumed = true;
                    provider_session_id = Some(session_id.to_string());
                }
                Err(error) => {
                    warn!(
                        target: "minos_agent_runtime::manager",
                        session_id,
                        session_id,
                        error = %error,
                        "gemini ACP session/resume failed; starting a fresh session",
                    );
                }
            }
        }
        if !resumed {
            let response = instance
                .new_session(
                    workspace,
                    mcp_server.iter().map(gemini_mcp_server).collect(),
                )
                .await
                .map_err(|error| anyhow::anyhow!("gemini ACP session/new failed: {error}"))?;
            provider_session_id = Some(response.session_id);
        }
        let provider_session_id = provider_session_id
            .ok_or_else(|| anyhow::anyhow!("gemini ACP session setup did not return session id"))?;
        crate::gemini_driver::spawn_acp_pump(
            instance.client.clone(),
            session_id.to_string(),
            self.events_tx.clone(),
            self.pending_approvals.clone(),
        );
        self.gemini_instances
            .lock()
            .await
            .insert(session_id.to_string(), instance);
        Ok(provider_session_id)
    }

    async fn start_grok_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        logical_session_id: Option<String>,
        conversation_id: Option<&str>,
        launch: Option<AgentLaunchOptions>,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let session_id = logical_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let model = launch.as_ref().and_then(|l| l.model.clone());
        let effort = launch.as_ref().and_then(|l| l.reasoning_effort.clone());
        let instructions = launch.as_ref().and_then(|l| l.instructions.clone());
        let provider_session_id = self
            .ensure_grok_instance_for_thread(
                &session_id,
                &canon,
                None,
                conversation_id,
                model.as_deref(),
                effort.as_deref(),
                instructions.as_deref(),
            )
            .await?;
        let mut handle = SessionHandle::new(
            session_id.clone(),
            canon.clone(),
            agent,
            SessionState::Idle,
            0,
        )
        .with_full_launch_options(model, effort, instructions);
        handle.codex_session_id = Some(provider_session_id.clone());
        handle.mcp_conversation_id = conversation_id.map(str::to_owned);
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::SessionAdded {
            session_id: session_id.clone(),
            workspace: canon.clone(),
            agent,
            parent_session_id: None,
        });
        Ok(StartAgentOutcome {
            session_id,
            cwd: canon,
            provider_session_id: Some(provider_session_id),
        })
    }

    async fn ensure_grok_instance_for_thread(
        &self,
        session_id: &str,
        workspace: &Path,
        resume_session_id: Option<&str>,
        conversation_id: Option<&str>,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
        instructions: Option<&str>,
    ) -> anyhow::Result<String> {
        if let Some(existing) = self.grok_instances.lock().await.get(session_id).cloned() {
            return existing.get_session_id().await.ok_or_else(|| {
                anyhow::anyhow!("grok ACP instance has no active session: {session_id}")
            });
        }

        let bin_path = self
            .config
            .grok_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(AgentName::Grok.bin_name()));
        let (crash_tx, _crash_rx) = tokio::sync::mpsc::channel::<()>(1);
        // Conversation-bound sessions get teamwork guidance via top-level
        // `grok --rules ...` (appended to the system prompt), matching Claude's
        // `--append-system-prompt` / Codex developerInstructions.
        // Profile instructions are always layered on when present.
        let rules_owned = {
            let mut parts: Vec<&str> = Vec::new();
            if conversation_id.is_some() {
                parts.push(MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS);
            }
            if let Some(extra) = instructions.map(str::trim).filter(|s| !s.is_empty()) {
                parts.push(extra);
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n\n"))
            }
        };
        let instance = crate::grok_driver::GrokAcpInstance::spawn_with_model(
            &bin_path,
            workspace,
            &self.config.subprocess_env,
            crash_tx,
            rules_owned.as_deref(),
            model,
            reasoning_effort,
        )
        .await
        .map_err(|error| anyhow::anyhow!("grok ACP spawn failed: {error}"))?;
        let instance = Arc::new(instance);
        let initialize = instance
            .initialize()
            .await
            .map_err(|error| anyhow::anyhow!("grok ACP initialize failed: {error}"))?;
        if initialize.protocol_version != 1 {
            warn!(
                target: "minos_agent_runtime::manager",
                protocol_version = initialize.protocol_version,
                "grok ACP returned unexpected protocol version",
            );
        }
        let mut resumed = false;
        let mut provider_session_id = None;
        let mcp_server = resolve_mcp_server(
            self.config.mcp.as_ref(),
            workspace,
            AgentName::Grok,
            conversation_id,
            Some(session_id),
        );
        if let Some(session_id) = resume_session_id {
            match instance
                .resume_session(
                    session_id,
                    workspace,
                    Some(mcp_server.iter().map(gemini_mcp_server).collect()),
                )
                .await
            {
                Ok(_) => {
                    resumed = true;
                    provider_session_id = Some(session_id.to_string());
                }
                Err(error) => {
                    warn!(
                        target: "minos_agent_runtime::manager",
                        session_id,
                        session_id,
                        error = %error,
                        "grok ACP session/resume failed; starting a fresh session",
                    );
                }
            }
        }
        if !resumed {
            let response = instance
                .new_session(
                    workspace,
                    mcp_server.iter().map(gemini_mcp_server).collect(),
                )
                .await
                .map_err(|error| anyhow::anyhow!("grok ACP session/new failed: {error}"))?;
            provider_session_id = Some(response.session_id);
        }
        let provider_session_id = provider_session_id
            .ok_or_else(|| anyhow::anyhow!("grok ACP session setup did not return session id"))?;
        crate::grok_driver::spawn_acp_pump(
            instance.client.clone(),
            session_id.to_string(),
            self.events_tx.clone(),
            self.pending_approvals.clone(),
            self.sessions.clone(),
            self.manager_tx.clone(),
            workspace.to_path_buf(),
        );
        self.grok_instances
            .lock()
            .await
            .insert(session_id.to_string(), instance);
        Ok(provider_session_id)
    }

    async fn ensure_opencode_instance(
        &self,
        workspace: &Path,
        conversation_id: Option<&str>,
        source_session_id: Option<&str>,
    ) -> anyhow::Result<Arc<Mutex<crate::opencode_driver::OpencodeServerInstance>>> {
        let key = InstanceKey::new(workspace, conversation_id, source_session_id);
        let mut map = self.opencode_instances.lock().await;
        if let Some(existing) = map.get(&key) {
            return Ok(existing.clone());
        }
        let bin = self
            .config
            .opencode_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(AgentName::Opencode.bin_name()));
        let port = pick_free_port(self.config.opencode_port_range.clone())?;
        let password = uuid::Uuid::new_v4().to_string();
        let mcp_server = resolve_mcp_server(
            self.config.mcp.as_ref(),
            workspace,
            AgentName::Opencode,
            conversation_id,
            source_session_id,
        );
        let config = crate::opencode_driver::OpencodeServerConfig {
            opencode_bin: bin,
            port,
            password,
            subprocess_env: self.config.subprocess_env.clone(),
            opencode_config_content: mcp_server.as_ref().map(opencode_config_content),
        };
        let instance =
            crate::opencode_driver::OpencodeServerInstance::spawn(workspace, config).await?;
        let instance = Arc::new(Mutex::new(instance));
        let inst_guard = instance.lock().await;
        let sse_url = inst_guard.subscribe_sse_url();
        let auth = inst_guard.auth_header().to_string();
        drop(inst_guard);
        crate::opencode_driver::spawn_sse_pump(
            sse_url,
            auth,
            self.opencode_session_map.clone(),
            self.sessions.clone(),
            self.manager_tx.clone(),
            self.events_tx.clone(),
        );
        map.insert(key, instance.clone());
        Ok(instance)
    }

    async fn ensure_instance(
        &self,
        workspace: &Path,
        policies: Option<&SessionPolicies>,
        conversation_id: Option<&str>,
        source_session_id: Option<&str>,
    ) -> anyhow::Result<Arc<AppServerInstance>> {
        let key = InstanceKey::new(workspace, conversation_id, source_session_id);
        let mut guard = self.instances.lock().await;
        if let Some(existing) = guard.get(&key) {
            return Ok(existing.clone());
        }
        if guard.len() >= self.caps.max_instances {
            self.lru_evict(&mut guard).await?;
        }
        let inst = self
            .spawn_instance(workspace, policies, conversation_id, source_session_id)
            .await?;
        guard.insert(key, inst.clone());
        Ok(inst)
    }

    #[allow(clippy::too_many_lines)]
    async fn spawn_instance(
        &self,
        workspace: &Path,
        policies: Option<&SessionPolicies>,
        conversation_id: Option<&str>,
        source_session_id: Option<&str>,
    ) -> anyhow::Result<Arc<AppServerInstance>> {
        let workspace_buf = workspace.to_path_buf();
        let workspace_display = workspace_buf.display().to_string();

        // Test seam: when `cfg.test_ws_url` is set, skip the real codex spawn
        // and connect directly to the fake URL. Production builds never enable
        // this path because `test_ws_url` is `#[cfg(feature = "test-support")]`.
        #[cfg(feature = "test-support")]
        if let Some(url) = self.config.test_ws_url.clone() {
            let client = CodexClient::connect(&url)
                .await
                .map_err(|e| anyhow::anyhow!("fake codex WS connect failed: {e}"))?;
            let client = Arc::new(client);
            // Test path: skip the JSON-RPC handshake. The FakeCodexBackend
            // (see crate::test_support) replies to the typed calls fired by
            // start_thread / send_user_message / interrupt_turn with canned
            // responses, so the handshake adds no test value.
            let (crash_tx, mut crash_rx) = tokio::sync::mpsc::channel::<()>(1);
            let inst = build_fake_instance(
                workspace_buf.clone(),
                client,
                self.config.thread_start_timeout,
                crash_tx,
            );
            let pump_client = inst.client.clone();
            let pump_events = self.events_tx.clone();
            let pump_threads = self.sessions.clone();
            let pump_workspace = workspace_buf.clone();
            let pump_crash = inst.crash_signal.clone();
            tokio::spawn(event_pump_loop(
                pump_client,
                pump_events,
                pump_threads,
                self.pending_approvals.clone(),
                self.manager_tx.clone(),
                pump_workspace,
                pump_crash,
            ));

            let watcher_inst = inst.clone();
            let watcher_threads = self.sessions.clone();
            let watcher_mgr_tx = self.manager_tx.clone();
            tokio::spawn(async move {
                let _ = crash_rx.recv().await;
                let affected = watcher_inst.session_ids().await;
                let tg = watcher_threads.lock().await;
                for tid in &affected {
                    if let Some(h) = tg.get(tid) {
                        let _ = h.transition(SessionState::Suspended {
                            reason: PauseReason::CodexCrashed,
                        });
                    }
                }
                drop(tg);
                let _ = watcher_mgr_tx.send(ManagerEvent::InstanceCrashed {
                    workspace: watcher_inst.workspace.clone(),
                    affected_threads: affected,
                    reason: PauseReason::CodexCrashed,
                });
            });
            return Ok(inst);
        }

        // Pick a free port + spawn `codex app-server --listen ws://...`.
        let port = pick_free_port(self.config.ws_port_range.clone())?;
        let url =
            Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL is well-formed");

        let bin = self
            .config
            .codex_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(AgentName::Codex.bin_name()));

        let listen_arg = format!("ws://127.0.0.1:{port}");
        let spawn_policies = resolve_session_policies(policies, &self.config.subprocess_env);
        let mcp_server = resolve_mcp_server(
            self.config.mcp.as_ref(),
            workspace,
            AgentName::Codex,
            conversation_id,
            source_session_id,
        );
        let args = build_codex_spawn_args(
            &listen_arg,
            &workspace_display,
            &spawn_policies,
            mcp_server.as_ref(),
        );
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let env = self.config.subprocess_env.clone();
        let mut process = CodexProcess::spawn(&bin, &arg_refs, &env)
            .map_err(|e| anyhow::anyhow!("codex spawn failed: {e}"))?;
        process.stderr_drain();
        info!(
            target: "minos_agent_runtime::manager",
            bin = %bin.display(),
            port,
            workspace = %workspace_display,
            "spawned codex app-server",
        );

        // Connect WS + handshake.
        let client = CodexClient::connect(&url)
            .await
            .map_err(|e| anyhow::anyhow!("codex WS connect failed: {e}"))?;
        let client = Arc::new(client);

        let init_params = InitializeParams {
            client_info: ClientInfo {
                name: env!("CARGO_PKG_NAME").into(),
                title: Some("Minos".into()),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            capabilities: Some(InitializeCapabilities {
                experimental_api: true,
                opt_out_notification_methods: None,
            }),
        };
        let _initialize_response: InitializeResponse = tokio::time::timeout(
            self.config.handshake_call_timeout,
            client.call_typed(init_params),
        )
        .await
        .map_err(|_| anyhow::anyhow!("initialize timeout"))?
        .map_err(|e| anyhow::anyhow!("initialize failed: {e}"))?;
        tokio::time::timeout(
            self.config.handshake_call_timeout,
            client.notify_typed(InitializedNotification),
        )
        .await
        .map_err(|_| anyhow::anyhow!("initialized timeout"))?
        .map_err(|e| anyhow::anyhow!("initialized failed: {e}"))?;

        // Take the child out of CodexProcess so it can be supervised in the
        // crash-watcher task below.
        let child = process
            .take_child()
            .ok_or_else(|| anyhow::anyhow!("codex process had no child"))?;
        let (crash_tx, mut crash_rx) = tokio::sync::mpsc::channel::<()>(1);
        let inst = Arc::new(AppServerInstance::new(
            workspace_buf.clone(),
            child,
            client.clone(),
            self.config.thread_start_timeout,
            crash_tx.clone(),
        ));

        // Spawn the event pump. It owns the client handle for inbound reads
        // and forwards every notification verbatim into the manager's
        // `events_tx` broadcast.
        let pump_client = client.clone();
        let pump_events = self.events_tx.clone();
        let pump_threads = self.sessions.clone();
        let pump_workspace = workspace_buf.clone();
        let pump_crash = crash_tx.clone();
        tokio::spawn(event_pump_loop(
            pump_client,
            pump_events,
            pump_threads,
            self.pending_approvals.clone(),
            self.manager_tx.clone(),
            pump_workspace,
            pump_crash,
        ));

        // Spawn the crash watcher. When the codex child exits or the WS pump
        // signals end-of-stream, we mark all sessions on this instance as
        // Suspended { CodexCrashed } and broadcast InstanceCrashed.
        let watcher_inst = inst.clone();
        let watcher_threads = self.sessions.clone();
        let watcher_mgr_tx = self.manager_tx.clone();
        tokio::spawn(async move {
            let _ = crash_rx.recv().await;
            let affected = watcher_inst.session_ids().await;
            let tg = watcher_threads.lock().await;
            for tid in &affected {
                if let Some(h) = tg.get(tid) {
                    let _ = h.transition(SessionState::Suspended {
                        reason: PauseReason::CodexCrashed,
                    });
                }
            }
            drop(tg);
            let _ = watcher_mgr_tx.send(ManagerEvent::InstanceCrashed {
                workspace: watcher_inst.workspace.clone(),
                affected_threads: affected,
                reason: PauseReason::CodexCrashed,
            });
        });

        Ok(inst)
    }

    async fn lru_evict(
        &self,
        map: &mut HashMap<InstanceKey, Arc<AppServerInstance>>,
    ) -> anyhow::Result<()> {
        let mut candidates: Vec<(InstanceKey, std::time::Instant)> = Vec::new();
        let tg = self.sessions.lock().await;
        for (key, inst) in map.iter() {
            let tids = inst.session_ids().await;
            let any_running = tids.iter().any(|t| {
                tg.get(t)
                    .is_some_and(|h| matches!(h.current_state(), SessionState::Running { .. }))
            });
            if !any_running {
                candidates.push((key.clone(), *inst.last_activity_at.lock().await));
            }
        }
        drop(tg);
        candidates.sort_by_key(|(_, t)| *t);
        let victim = candidates.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("TooManyInstances: every instance has a Running thread")
        })?;
        let inst = map.remove(&victim.0).expect("victim was in map");
        let tids = inst.session_ids().await;
        let workspace = inst.workspace.clone();
        let tg = self.sessions.lock().await;
        for tid in &tids {
            if let Some(h) = tg.get(tid) {
                let _ = h.transition(SessionState::Suspended {
                    reason: PauseReason::InstanceReaped,
                });
            }
        }
        drop(tg);
        let _ = self.manager_tx.send(ManagerEvent::InstanceCrashed {
            workspace,
            affected_threads: tids,
            reason: PauseReason::InstanceReaped,
        });
        let child_opt = inst.child.lock().await.take();
        drop(inst);
        if let Some(mut child) = child_opt {
            let _ = child.kill().await;
        }
        Ok(())
    }

    /// Test-only snapshot of which workspaces have an open instance.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn open_workspaces(&self) -> Vec<PathBuf> {
        self.instances
            .lock()
            .await
            .keys()
            .map(|key| key.workspace.clone())
            .collect()
    }

    /// Test-only count of currently tracked sessions.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn thread_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Workspace path for a live session (for materializing Hub attachments).
    pub async fn session_workspace(&self, session_id: &str) -> Option<std::path::PathBuf> {
        let sessions = self.sessions.lock().await;
        sessions.get(session_id).map(|h| h.workspace.clone())
    }

    pub async fn session_state(&self, session_id: &str) -> Option<SessionState> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .map(SessionHandle::current_state)
    }

    /// Test-only helper: run one pass of the reaper synchronously. Production
    /// code spawns the periodic loop in [`AgentManager::spawn_reaper`].
    #[doc(hidden)]
    pub async fn tick_reaper_once(&self) {
        let mut to_reap: Vec<InstanceKey> = Vec::new();
        {
            let ig = self.instances.lock().await;
            for (key, inst) in ig.iter() {
                let last = *inst.last_activity_at.lock().await;
                let idle = last.elapsed() >= self.caps.idle_timeout;
                let tids = inst.session_ids().await;
                let tg = self.sessions.lock().await;
                let any_running = tids.iter().any(|t| {
                    tg.get(t)
                        .is_some_and(|h| matches!(h.current_state(), SessionState::Running { .. }))
                });
                drop(tg);
                if idle && !any_running {
                    to_reap.push(key.clone());
                }
            }
        }
        for key in to_reap {
            self.reap_instance(&key).await;
        }
    }

    async fn reap_instance(&self, key: &InstanceKey) {
        Self::reap_static(&self.instances, &self.sessions, &self.manager_tx, key).await;
    }

    pub async fn send_user_message(&self, session_id: &str, text: String) -> anyhow::Result<()> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?;
        match handle.current_state() {
            SessionState::Idle => {
                self.send_user_message_while_idle(session_id, text, &handle)
                    .await
            }
            SessionState::Running { .. } => match handle.agent {
                AgentName::Codex => self.steer_turn(session_id, text).await,
                AgentName::Opencode => self.send_opencode_prompt(session_id, &text, &handle).await,
                AgentName::Claude => self.send_claude_prompt(session_id, &text, &handle).await,
                AgentName::Gemini => anyhow::bail!("gemini turn is already running"),
                AgentName::Grok => anyhow::bail!("grok turn is already running"),
            },
            SessionState::Suspended { .. } => {
                // Reattach to Idle, then send as a normal idle turn (user text wins;
                // CONTINUE is never injected on this path).
                self.reattach_suspended_thread(session_id).await?;
                let handle = self
                    .sessions
                    .lock()
                    .await
                    .get(session_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?;
                self.send_user_message_while_idle(session_id, text, &handle)
                    .await
            }
            other => anyhow::bail!("send_user_message rejected: state={other:?}"),
        }
    }

    async fn send_user_message_while_idle(
        &self,
        session_id: &str,
        text: String,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        if !matches!(handle.current_state(), SessionState::Idle) {
            anyhow::bail!(
                "send_user_message_while_idle rejected: state={:?}",
                handle.current_state()
            );
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let new_state = SessionState::Running {
            turn_started_at_ms: now_ms,
        };
        handle.transition(new_state.clone())?;
        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old: SessionState::Idle,
            new: new_state,
            at_ms: now_ms,
        });
        self.synth_user_message_ingest(session_id, &text, handle.agent)
            .await?;
        match handle.agent {
            AgentName::Codex => {
                let key = InstanceKey::for_handle(handle);
                let inst = self
                    .instances
                    .lock()
                    .await
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("instance for conversation gone"))?;
                inst.touch().await;
                let provider_session_id = codex_provider_session_id(session_id, handle);
                let turn_id = inst.send_user_message(&provider_session_id, &text).await?;
                handle.set_active_turn_id_if_absent(turn_id);
            }
            AgentName::Claude => {
                self.start_claude_turn(session_id, &text, handle).await?;
            }
            AgentName::Opencode => {
                let oc_session_id = self
                    .ensure_opencode_session_for_session(session_id, handle)
                    .await?;
                let key = InstanceKey::for_handle(handle);
                let instance = self
                    .opencode_instances
                    .lock()
                    .await
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("opencode instance not found"))?;
                instance
                    .lock()
                    .await
                    .send_prompt(&oc_session_id, &text)
                    .await?;
            }
            AgentName::Gemini => {
                self.spawn_gemini_prompt_task(session_id.to_string(), text, handle.clone())
                    .await?;
            }
            AgentName::Grok => {
                self.spawn_grok_prompt_task(session_id.to_string(), text, handle.clone())
                    .await?;
            }
        }
        Ok(())
    }

    /// Provider reattach for a suspended thread; ends in Idle. No user text, no CONTINUE.
    pub async fn reattach_suspended_thread(&self, session_id: &str) -> anyhow::Result<()> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?;
        match handle.current_state() {
            SessionState::Idle | SessionState::Running { .. } => Ok(()),
            SessionState::Closed { .. } => {
                anyhow::bail!("reattach rejected: thread is closed")
            }
            SessionState::Starting | SessionState::Resuming => {
                anyhow::bail!("reattach rejected: thread is {:?}", handle.current_state())
            }
            SessionState::Suspended { .. } => {
                let result = match handle.agent {
                    AgentName::Codex => self.reattach_codex_suspended(session_id, &handle).await,
                    AgentName::Claude => self.resume_claude_thread(session_id, "", &handle).await,
                    AgentName::Opencode => self.resume_opencode_thread(session_id, &handle).await,
                    AgentName::Gemini => self.resume_gemini_thread(session_id, &handle).await,
                    AgentName::Grok => self.resume_grok_thread(session_id, &handle).await,
                };
                if result.is_err() {
                    // Roll back mid-flight Resuming (or other non-terminal) so a later
                    // send can re-try from Suspended rather than stuck Resuming.
                    let cur = handle.current_state();
                    if matches!(cur, SessionState::Resuming | SessionState::Starting) {
                        let suspended = SessionState::Suspended {
                            reason: PauseReason::DaemonRestart,
                        };
                        if handle.transition(suspended.clone()).is_ok() {
                            let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
                                session_id: session_id.to_string(),
                                old: cur,
                                new: suspended,
                                at_ms: chrono::Utc::now().timestamp_millis(),
                            });
                        }
                    }
                }
                result
            }
        }
    }

    /// Inject [`CONTINUE_PROMPT`] after reattach. Caller must have already
    /// claimed `needs_continue` in the store (one-shot). Ends with a Running turn.
    pub async fn inject_continue_prompt(&self, session_id: &str) -> anyhow::Result<()> {
        self.reattach_suspended_thread(session_id).await?;
        self.send_user_message(session_id, CONTINUE_PROMPT.to_string())
            .await
    }

    /// Best-effort cancel + transition to `Suspended { DaemonRestart }`.
    /// Returns whether auto-continue should be offered on next resume.
    /// Does **not** close provider sessions (unlike [`Self::close_session`]).
    pub async fn suspend_for_daemon_stop(&self, session_id: &str) -> anyhow::Result<bool> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?;
        let from = handle.current_state();
        if matches!(from, SessionState::Closed { .. }) {
            return Ok(false);
        }
        let needs_continue = matches!(
            from,
            SessionState::Running { .. } | SessionState::Starting | SessionState::Resuming
        );
        if matches!(from, SessionState::Running { .. }) {
            self.best_effort_cancel_turn(session_id, &handle).await;
        }
        handle.set_active_turn_id(None);
        let new = SessionState::Suspended {
            reason: PauseReason::DaemonRestart,
        };
        if from != new {
            handle.transition(new.clone())?;
            let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
                session_id: session_id.to_string(),
                old: from,
                new,
                at_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        Ok(needs_continue)
    }

    async fn best_effort_cancel_turn(&self, session_id: &str, handle: &SessionHandle) {
        match handle.agent {
            AgentName::Codex => {
                let key = InstanceKey::for_handle(handle);
                if let Some(inst) = self.instances.lock().await.get(&key).cloned() {
                    let provider_session_id = codex_provider_session_id(session_id, handle);
                    let _ = inst.interrupt_turn(&provider_session_id).await;
                }
            }
            AgentName::Claude => {
                if let Some(session) = self.claude_sessions.lock().await.get_mut(session_id) {
                    if let Some(child) = session.current_turn_child.as_mut() {
                        let _ = child.start_kill();
                    }
                }
            }
            AgentName::Opencode => {
                let oc_session_id = self
                    .opencode_session_map
                    .lock()
                    .await
                    .get(session_id)
                    .cloned();
                if let Some(oc_sid) = oc_session_id {
                    let key = InstanceKey::for_handle(handle);
                    let instance = self.opencode_instances.lock().await.get(&key).cloned();
                    if let Some(inst) = instance {
                        let _ = inst.lock().await.abort_session(&oc_sid).await;
                    }
                }
            }
            AgentName::Gemini => {
                if let Some(instance) = self.gemini_instances.lock().await.get(session_id).cloned()
                {
                    let _ = instance.cancel().await;
                }
            }
            AgentName::Grok => {
                if let Some(instance) = self.grok_instances.lock().await.get(session_id).cloned() {
                    let _ = instance.cancel().await;
                }
            }
        }
    }

    async fn reattach_codex_suspended(
        &self,
        session_id: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        let from_state = handle.current_state();
        handle.transition(SessionState::Resuming)?;
        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old: from_state,
            new: SessionState::Resuming,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        let workspace = handle.workspace.clone();
        let codex_session_id = handle.codex_session_id.clone();
        let source_session_id = handle.mcp_conversation_id.as_ref().map(|_| session_id);

        let inst = self
            .ensure_instance(
                &workspace,
                None,
                handle.mcp_conversation_id.as_deref(),
                source_session_id,
            )
            .await?;
        if let Some(sid) = codex_session_id {
            let provider_session_id = sid.clone();
            inst.add_thread(session_id.to_string()).await;
            inst.start_thread_resume(&provider_session_id, &sid).await?;
        } else {
            let _ = handle.transition(SessionState::Closed {
                reason: crate::state_machine::CloseReason::TerminalError,
            });
            anyhow::bail!("resume failed: no codex_session_id");
        }
        handle.transition(SessionState::Idle)?;
        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old: SessionState::Resuming,
            new: SessionState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        inst.touch().await;
        Ok(())
    }

    async fn start_claude_turn(
        &self,
        session_id: &str,
        text: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        let cli_path = PathBuf::from(AgentName::Claude.bin_name());
        let provider_session_id = match provider_resume_session_id(session_id, handle) {
            Some(id) => id.to_string(),
            None => {
                let new_provider_id = uuid::Uuid::new_v4().to_string();
                self.set_session_provider_session_id(session_id, new_provider_id.clone())
                    .await;
                new_provider_id
            }
        };
        let has_runtime_session = self.claude_sessions.lock().await.contains_key(session_id);
        let has_persisted_history = handle.last_seq.load(std::sync::atomic::Ordering::SeqCst) > 0;
        let resume_sid =
            (has_runtime_session || has_persisted_history).then_some(provider_session_id.as_str());
        // Claude CLI --session-id only when not resuming an existing provider session.
        let claude_session_id = resume_sid.is_none().then_some(provider_session_id.as_str());
        let mcp_server = resolve_mcp_server(
            self.config.mcp.as_ref(),
            &handle.workspace,
            AgentName::Claude,
            handle.mcp_conversation_id.as_deref(),
            Some(session_id),
        );
        let claude_mcp_config = mcp_server.as_ref().map(claude_mcp_config_json);
        let session = crate::claude_driver::ClaudeNdjsonSession::start_turn(
            &cli_path,
            &handle.workspace,
            session_id.to_string(),
            text,
            claude_session_id,
            resume_sid,
            self.sessions.clone(),
            self.manager_tx.clone(),
            self.events_tx.clone(),
            &self.config.subprocess_env,
            claude_mcp_config.as_deref(),
            handle.model.as_deref(),
            handle.instructions.as_deref(),
        )
        .await?;
        self.claude_sessions
            .lock()
            .await
            .insert(session_id.to_string(), session);
        Ok(())
    }

    async fn send_claude_prompt(
        &self,
        session_id: &str,
        text: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        self.synth_user_message_ingest(session_id, text, handle.agent)
            .await?;
        self.start_claude_turn(session_id, text, handle).await
    }

    async fn ensure_opencode_session_for_session(
        &self,
        session_id: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<String> {
        if let Some(existing) = self
            .opencode_session_map
            .lock()
            .await
            .get(session_id)
            .cloned()
        {
            return Ok(existing);
        }

        let workspace = handle.workspace.clone();
        let source_session_id = handle.mcp_conversation_id.as_ref().map(|_| session_id);
        let instance = self
            .ensure_opencode_instance(
                &workspace,
                handle.mcp_conversation_id.as_deref(),
                source_session_id,
            )
            .await?;
        let provider_session_id = match provider_resume_session_id(session_id, handle) {
            Some(id) => id.to_string(),
            None => instance.lock().await.create_session().await?,
        };
        self.opencode_session_map
            .lock()
            .await
            .insert(session_id.to_string(), provider_session_id.clone());
        self.set_session_provider_session_id(session_id, provider_session_id.clone())
            .await;
        Ok(provider_session_id)
    }

    async fn send_opencode_prompt(
        &self,
        session_id: &str,
        text: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        let oc_session_id = self
            .ensure_opencode_session_for_session(session_id, handle)
            .await?;
        let key = InstanceKey::for_handle(handle);
        let instance = self
            .opencode_instances
            .lock()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("opencode instance not found"))?;
        self.synth_user_message_ingest(session_id, text, handle.agent)
            .await?;
        let result = instance
            .lock()
            .await
            .send_prompt(&oc_session_id, text)
            .await;
        result
    }

    async fn set_session_provider_session_id(&self, session_id: &str, provider_session_id: String) {
        if let Some(handle) = self.sessions.lock().await.get_mut(session_id) {
            handle.codex_session_id = Some(provider_session_id);
        }
    }

    fn transition_resumed_thread_to_idle(
        &self,
        session_id: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        let old = handle.current_state();
        handle.transition(SessionState::Resuming)?;
        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old,
            new: SessionState::Resuming,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        handle.transition(SessionState::Idle)?;
        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old: SessionState::Resuming,
            new: SessionState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        Ok(())
    }

    async fn resume_claude_thread(
        &self,
        session_id: &str,
        _text: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        self.transition_resumed_thread_to_idle(session_id, handle)
    }

    async fn resume_opencode_thread(
        &self,
        session_id: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        self.ensure_opencode_session_for_session(session_id, handle)
            .await?;
        self.transition_resumed_thread_to_idle(session_id, handle)
    }

    async fn resume_gemini_thread(
        &self,
        session_id: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        let provider_session_id = self
            .ensure_gemini_instance_for_thread(
                session_id,
                &handle.workspace,
                provider_resume_session_id(session_id, handle),
                handle.mcp_conversation_id.as_deref(),
                handle.model.as_deref(),
            )
            .await?;
        self.set_session_provider_session_id(session_id, provider_session_id)
            .await;
        self.transition_resumed_thread_to_idle(session_id, handle)
    }

    async fn spawn_gemini_prompt_task(
        &self,
        session_id: String,
        text: String,
        handle: SessionHandle,
    ) -> anyhow::Result<()> {
        let instance = self
            .gemini_instances
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("gemini ACP instance not found: {session_id}"))?;
        let events_tx = self.events_tx.clone();
        let manager_tx = self.manager_tx.clone();
        tokio::spawn(async move {
            instance.touch().await;
            let result = instance.prompt(&text).await;
            let payload = match result {
                Ok(response) => serde_json::json!({
                    "kind": "acp_prompt_response",
                    "stopReason": response.stop_reason,
                }),
                Err(error) => serde_json::json!({
                    "kind": "acp_error",
                    "code": "session/prompt",
                    "message": error.to_string(),
                }),
            };
            if let Err(error) = events_tx
                .emit(RawIngest::from_json(
                    AgentName::Gemini,
                    session_id.clone(),
                    payload,
                    current_unix_ms(),
                ))
                .await
            {
                warn!(
                    target: "minos_agent_runtime::manager",
                    error = %error,
                    session_id,
                    "failed to emit gemini prompt result ingest",
                );
            }
            mark_session_idle_with_tx(&session_id, &handle, &manager_tx);
        });
        Ok(())
    }

    async fn resume_grok_thread(
        &self,
        session_id: &str,
        handle: &SessionHandle,
    ) -> anyhow::Result<()> {
        let provider_session_id = self
            .ensure_grok_instance_for_thread(
                session_id,
                &handle.workspace,
                provider_resume_session_id(session_id, handle),
                handle.mcp_conversation_id.as_deref(),
                handle.model.as_deref(),
                handle.reasoning_effort.as_deref(),
                handle.instructions.as_deref(),
            )
            .await?;
        self.set_session_provider_session_id(session_id, provider_session_id)
            .await;
        self.transition_resumed_thread_to_idle(session_id, handle)
    }

    async fn spawn_grok_prompt_task(
        &self,
        session_id: String,
        text: String,
        handle: SessionHandle,
    ) -> anyhow::Result<()> {
        let instance = self
            .grok_instances
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("grok ACP instance not found: {session_id}"))?;
        let events_tx = self.events_tx.clone();
        let manager_tx = self.manager_tx.clone();
        tokio::spawn(async move {
            instance.touch().await;
            let result = instance.prompt(&text).await;
            let payload = match result {
                Ok(response) => serde_json::json!({
                    "kind": "acp_prompt_response",
                    "stopReason": response.stop_reason,
                }),
                Err(error) => serde_json::json!({
                    "kind": "acp_error",
                    "code": "session/prompt",
                    "message": error.to_string(),
                }),
            };
            if let Err(error) = events_tx
                .emit(RawIngest::from_json(
                    AgentName::Grok,
                    session_id.clone(),
                    payload,
                    current_unix_ms(),
                ))
                .await
            {
                warn!(
                    target: "minos_agent_runtime::manager",
                    error = %error,
                    session_id,
                    "failed to emit grok prompt result ingest",
                );
            }
            mark_session_idle_with_tx(&session_id, &handle, &manager_tx);
        });
        Ok(())
    }

    pub async fn steer_turn(&self, session_id: &str, text: String) -> anyhow::Result<()> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?;
        if !matches!(handle.current_state(), SessionState::Running { .. }) {
            let state = handle.current_state();
            anyhow::bail!("steer_turn rejected: state={state:?}");
        }
        let expected_turn_id = handle
            .active_turn_id()
            .ok_or_else(|| anyhow::anyhow!("steer_turn rejected: missing active turn id"))?;
        let key = InstanceKey::for_handle(&handle);
        let inst = self
            .instances
            .lock()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("instance for conversation gone"))?;
        inst.touch().await;
        self.synth_user_message_ingest(session_id, &text, handle.agent)
            .await?;
        let provider_session_id = codex_provider_session_id(session_id, &handle);
        let turn_id = inst
            .steer_turn(&provider_session_id, &expected_turn_id, &text)
            .await?;
        handle.set_active_turn_id(Some(turn_id));
        Ok(())
    }

    /// Build and broadcast a synthetic user-message ingest event.
    ///
    /// Codex-style agents use a synthetic codex `item/started{userMessage}`
    /// notification matching the real codex 2026-04 wire shape (see
    /// `minos-codex-protocol::ItemStartedNotification` + `ThreadItem`).
    /// Gemini uses a Minos-owned `kind:user_message` event so its translator
    /// stays scoped to ACP instead of accepting codex JSON-RPC shapes.
    ///
    /// The `EventWriter` bridge persists it to the local SQLite store and
    /// `RelayClient` forwards it to the backend, which translates it into a
    /// `MessageStarted{role:User} + TextDelta` pair and fans out to
    /// paired mobile peers.
    ///
    /// Codex 2026-04 does NOT echo user inputs as separate notifications
    /// (the user content lives inside the synchronous `turn/start`
    /// request body). Without this synthesis the user message would never
    /// reach either persistence layer, so killing the app would lose it.
    async fn synth_user_message_ingest(
        &self,
        session_id: &str,
        text: &str,
        agent: AgentName,
    ) -> anyhow::Result<()> {
        let item_id = uuid::Uuid::new_v4().to_string();
        let payload = match agent {
            AgentName::Gemini | AgentName::Grok => serde_json::json!({
                "kind": "user_message",
                "messageId": item_id,
                "text": text,
                "sessionId": session_id,
            }),
            _ => serde_json::json!({
                "method": "item/started",
                "params": {
                    "item": {
                        "type": "userMessage",
                        "id": item_id,
                        "content": [{"type": "text", "text": text}],
                    },
                    "threadId": session_id,
                    "turnId": "",
                }
            }),
        };
        let ingest =
            RawIngest::from_json(agent, session_id.to_string(), payload, current_unix_ms());
        self.events_tx.emit(ingest).await?;
        Ok(())
    }

    pub async fn interrupt_session(&self, session_id: &str) -> anyhow::Result<()> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        if !matches!(
            handle.current_state(),
            SessionState::Running { .. } | SessionState::Idle
        ) {
            let s = handle.current_state();
            anyhow::bail!("interrupt rejected: state={s:?}");
        }
        match handle.agent {
            AgentName::Codex => {
                let key = InstanceKey::for_handle(&handle);
                if let Some(inst) = self.instances.lock().await.get(&key).cloned() {
                    let provider_session_id = codex_provider_session_id(session_id, &handle);
                    let _ = inst.interrupt_turn(&provider_session_id).await;
                }
            }
            AgentName::Claude => {
                if let Some(session) = self.claude_sessions.lock().await.get_mut(session_id) {
                    if let Some(child) = session.current_turn_child.as_mut() {
                        let _ = child.start_kill();
                    }
                }
            }
            AgentName::Opencode => {
                let oc_session_id = self
                    .opencode_session_map
                    .lock()
                    .await
                    .get(session_id)
                    .cloned();
                if let Some(oc_sid) = oc_session_id {
                    let key = InstanceKey::for_handle(&handle);
                    let instance = self.opencode_instances.lock().await.get(&key).cloned();
                    if let Some(inst) = instance {
                        let _ = inst.lock().await.abort_session(&oc_sid).await;
                    }
                }
            }
            AgentName::Gemini => {
                if let Some(instance) = self.gemini_instances.lock().await.get(session_id).cloned()
                {
                    let _ = instance.cancel().await;
                }
            }
            AgentName::Grok => {
                if let Some(instance) = self.grok_instances.lock().await.get(session_id).cloned() {
                    let _ = instance.cancel().await;
                }
            }
        }
        let from_state = handle.current_state();
        handle.set_active_turn_id(None);
        handle.transition(SessionState::Suspended {
            reason: PauseReason::UserInterrupt,
        })?;
        let _ = self.manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old: from_state,
            new: SessionState::Suspended {
                reason: PauseReason::UserInterrupt,
            },
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        Ok(())
    }

    pub async fn list_host_skills(
        &self,
        workspace: PathBuf,
        force_reload: bool,
    ) -> anyhow::Result<SkillsListResponse> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or(workspace);
        let inst = self.ensure_instance(&canon, None, None, None).await?;
        inst.touch().await;
        inst.list_host_skills(&canon, force_reload).await
    }

    pub async fn write_host_skill_config(
        &self,
        workspace: PathBuf,
        path: PathBuf,
        enabled: bool,
    ) -> anyhow::Result<SkillsConfigWriteResponse> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or(workspace);
        let inst = self.ensure_instance(&canon, None, None, None).await?;
        inst.touch().await;
        inst.write_host_skill_config(&path, enabled).await
    }

    pub async fn close_session(&self, session_id: &str) -> anyhow::Result<()> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        if matches!(handle.current_state(), SessionState::Closed { .. }) {
            return Ok(());
        }
        handle.transition(SessionState::Closed {
            reason: crate::state_machine::CloseReason::UserClose,
        })?;
        match handle.agent {
            AgentName::Codex => {
                let key = InstanceKey::for_handle(&handle);
                if let Some(inst) = self.instances.lock().await.get(&key).cloned() {
                    inst.remove_thread(session_id).await;
                }
            }
            AgentName::Claude => {
                if let Some(session) = self.claude_sessions.lock().await.remove(session_id) {
                    session.close(&self.events_tx).await;
                }
            }
            AgentName::Opencode => {
                self.opencode_session_map.lock().await.remove(session_id);
            }
            AgentName::Gemini => {
                if let Some(instance) = self.gemini_instances.lock().await.remove(session_id) {
                    let _ = instance.close_session().await;
                }
            }
            AgentName::Grok => {
                if let Some(instance) = self.grok_instances.lock().await.remove(session_id) {
                    let _ = instance.close_session().await;
                }
            }
        }
        let _ = self.manager_tx.send(ManagerEvent::SessionClosed {
            session_id: session_id.to_string(),
            reason: crate::state_machine::CloseReason::UserClose,
        });
        Ok(())
    }

    pub async fn resolve_approval(
        &self,
        request_id: &str,
        session_id: &str,
        decision: Value,
    ) -> anyhow::Result<()> {
        let Some(pending) = self
            .pending_approvals
            .get(request_id)
            .map(|entry| entry.value().clone())
        else {
            return Ok(());
        };

        if pending.session_id != session_id {
            anyhow::bail!(
                "approval request thread mismatch: expected {}, got {session_id}",
                pending.session_id,
            );
        }

        let reply = match &pending.target {
            PendingApprovalTarget::Codex { request, .. } => {
                crate::approvals::validate_decision(request.as_ref(), &decision)?
            }
            PendingApprovalTarget::Acp {
                allow_option_id,
                reject_option_id,
                ..
            } => crate::approvals::validate_acp_permission_decision(
                &decision,
                allow_option_id.as_deref(),
                reject_option_id.as_deref(),
            )?,
            PendingApprovalTarget::GrokExtMethod { nested_method, .. } => {
                crate::approvals::validate_grok_ext_method_decision(nested_method, &decision)?
            }
        };
        let Some((_, pending)) = self.pending_approvals.remove(request_id) else {
            return Ok(());
        };
        let agent = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .map(|h| h.agent)
            .unwrap_or(AgentName::Codex);
        let reply_result = match pending.target {
            PendingApprovalTarget::Codex {
                request_id, client, ..
            } => client
                .reply(request_id, reply)
                .await
                .map_err(|error| anyhow::anyhow!("approval reply failed: {error}")),
            PendingApprovalTarget::Acp {
                request_id, client, ..
            }
            | PendingApprovalTarget::GrokExtMethod {
                request_id, client, ..
            } => client
                .reply(request_id, reply)
                .await
                .map_err(|error| anyhow::anyhow!("ACP approval reply failed: {error}")),
        };
        // Durable resolution marker so history replay / assemblers can demote the
        // interactive approval card (request alone is not enough after approve).
        if reply_result.is_ok() {
            let decision_label = decision
                .get("decision")
                .or_else(|| decision.get("outcome"))
                .and_then(Value::as_str)
                .unwrap_or("resolved");
            let ingest = RawIngest::from_json(
                agent,
                session_id.to_owned(),
                serde_json::json!({
                    "method": "approval/resolved",
                    "params": {
                        "request_id": request_id,
                        "session_id": session_id,
                        "decision": decision_label,
                    }
                }),
                current_unix_ms(),
            );
            if let Err(error) = self.events_tx.emit(ingest).await {
                warn!(
                    target: "minos_agent_runtime::manager",
                    error = %error,
                    request_id,
                    session_id,
                    "failed to emit approval/resolved ingest"
                );
            }
        }
        reply_result
    }

    pub async fn respond_opencode_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> anyhow::Result<()> {
        let handle = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(session_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?
        };
        anyhow::ensure!(
            handle.agent == AgentName::Opencode,
            "thread {session_id} is not an opencode thread"
        );

        let session_id = self
            .opencode_session_map
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("opencode session not found for thread {session_id}"))?;
        let instance = self
            .opencode_instances
            .lock()
            .await
            .get(&InstanceKey::for_handle(&handle))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("opencode instance not found"))?;

        let result = instance
            .lock()
            .await
            .respond_permission(&session_id, permission_id, response)
            .await;
        result
    }

    pub async fn respond_opencode_question(
        &self,
        session_id: &str,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> anyhow::Result<()> {
        let handle = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(session_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?
        };
        anyhow::ensure!(
            handle.agent == AgentName::Opencode,
            "thread {session_id} is not an opencode thread"
        );

        let instance = self
            .opencode_instances
            .lock()
            .await
            .get(&InstanceKey::for_handle(&handle))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("opencode instance not found"))?;

        let result = instance
            .lock()
            .await
            .respond_question(question_id, answers)
            .await;
        result
    }

    /// Shut every **provider child process** down (Codex app-server, OpenCode
    /// `serve`, Gemini/Grok ACP). Polite SIGTERM to each process group, wait
    /// `grace`, then SIGKILL. Drops every instance map. Used by
    /// [`DaemonHandle::stop`](minos_daemon path).
    ///
    /// Each provider is spawned with `setpgid(0, 0)` so group signals reach
    /// helpers the CLI forked. Without this, only the leader was reaped and
    /// children were reparented to launchd on macOS — classic OpenCode port
    /// leak (`4096..=4106 all occupied`) after Desktop restart.
    ///
    /// Historically this method only drained Codex `instances`. OpenCode /
    /// Gemini / Grok were left alive across daemon stop → zombie `opencode
    /// serve` processes with PPID=1.
    pub async fn shutdown_instances(&self, grace: std::time::Duration) {
        // Snapshot group-leader pids from every provider map first so we can
        // signal without holding locks across the grace sleep.
        let mut pgids: Vec<i32> = Vec::new();

        {
            let g = self.instances.lock().await;
            for inst in g.values() {
                if let Some(child) = inst.child.lock().await.as_ref() {
                    if let Some(pid) = child.id() {
                        if let Ok(pid_i32) = i32::try_from(pid) {
                            pgids.push(pid_i32);
                        }
                    }
                }
            }
        }
        {
            let g = self.opencode_instances.lock().await;
            for inst in g.values() {
                if let Some(child) = inst.lock().await.child.as_ref() {
                    if let Some(pid) = child.id() {
                        if let Ok(pid_i32) = i32::try_from(pid) {
                            pgids.push(pid_i32);
                        }
                    }
                }
            }
        }
        {
            let g = self.gemini_instances.lock().await;
            for inst in g.values() {
                if let Some(child) = inst.child.lock().await.as_ref() {
                    if let Some(pid) = child.id() {
                        if let Ok(pid_i32) = i32::try_from(pid) {
                            pgids.push(pid_i32);
                        }
                    }
                }
            }
        }
        {
            let g = self.grok_instances.lock().await;
            for inst in g.values() {
                if let Some(child) = inst.child.lock().await.as_ref() {
                    if let Some(pid) = child.id() {
                        if let Ok(pid_i32) = i32::try_from(pid) {
                            pgids.push(pid_i32);
                        }
                    }
                }
            }
        }

        // Phase 1: SIGTERM each process group (negative pid = group whose
        // leader is this pid; set in each driver's pre_exec setpgid).
        #[cfg(unix)]
        for &pgid in &pgids {
            // SAFETY: kill(2) with negative pid is the documented group form.
            let _ = unsafe { libc::kill(-pgid, libc::SIGTERM) };
        }

        tokio::time::sleep(grace).await;

        // Phase 2: SIGKILL stragglers.
        #[cfg(unix)]
        for &pgid in &pgids {
            let _ = unsafe { libc::kill(-pgid, libc::SIGKILL) };
        }

        // Drain maps and reap leaders (also covers non-Unix kill_on_drop paths).
        {
            let mut g = self.instances.lock().await;
            for (_, inst) in std::mem::take(&mut *g) {
                let child_opt = inst.child.lock().await.take();
                drop(inst);
                if let Some(mut child) = child_opt {
                    let _ = child.kill().await;
                }
            }
        }
        {
            let mut g = self.opencode_instances.lock().await;
            for (_, inst) in std::mem::take(&mut *g) {
                // Prefer the dedicated close path (SIGTERM → wait → SIGKILL).
                let taken = {
                    let mut guard = inst.lock().await;
                    // Swap out so Drop of Arc doesn't double-kill; close consumes child.
                    let child = guard.child.take();
                    (child, guard.workspace.clone())
                };
                if let Some(mut child) = taken.0 {
                    #[cfg(unix)]
                    if let Some(pid) = child.id() {
                        if let Ok(pid_i32) = i32::try_from(pid) {
                            let _ = unsafe { libc::kill(-pid_i32, libc::SIGKILL) };
                        }
                    }
                    let _ = child.kill().await;
                }
                tracing::info!(
                    target: "minos_agent_runtime::manager",
                    workspace = %taken.1.display(),
                    "opencode server instance shut down"
                );
            }
        }
        {
            let mut g = self.gemini_instances.lock().await;
            for (_, inst) in std::mem::take(&mut *g) {
                if let Some(mut child) = inst.child.lock().await.take() {
                    let _ = child.kill().await;
                }
            }
        }
        {
            let mut g = self.grok_instances.lock().await;
            for (_, inst) in std::mem::take(&mut *g) {
                if let Some(mut child) = inst.child.lock().await.take() {
                    let _ = child.kill().await;
                }
            }
        }
        // Clear session maps so a later resume cannot target dead provider ids.
        self.opencode_session_map.lock().await.clear();
    }

    pub async fn list_sessions(&self) -> Vec<crate::store_facing::SessionSnapshot> {
        let g = self.sessions.lock().await;
        g.values()
            .map(|h| crate::store_facing::SessionSnapshot {
                session_id: h.session_id.clone(),
                workspace: h.workspace.clone(),
                state: h.current_state(),
                parent_session_id: h.parent_session_id.clone(),
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct StartAgentOutcome {
    pub session_id: String,
    pub cwd: PathBuf,
    pub provider_session_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionPolicies {
    pub approval_policy: Option<String>,
    pub sandbox_policy: Option<String>,
}

/// Create-time model / effort / instructions binding (not mid-session switch).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentLaunchOptions {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    /// Extra system / developer instructions appended at session start.
    pub instructions: Option<String>,
}

impl AgentLaunchOptions {
    pub fn from_parts(model: Option<String>, reasoning_effort: Option<String>) -> Option<Self> {
        Self::from_parts_full(model, reasoning_effort, None)
    }

    pub fn from_parts_full(
        model: Option<String>,
        reasoning_effort: Option<String>,
        instructions: Option<String>,
    ) -> Option<Self> {
        let model = model.and_then(|m| {
            let t = m.trim().to_owned();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });
        let reasoning_effort = reasoning_effort.and_then(|m| {
            let t = m.trim().to_owned();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });
        let instructions = instructions.and_then(|m| {
            let t = m.trim().to_owned();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });
        if model.is_none() && reasoning_effort.is_none() && instructions.is_none() {
            None
        } else {
            Some(Self {
                model,
                reasoning_effort,
                instructions,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub session_id: String,
    pub cwd: PathBuf,
    pub provider_session_id: Option<String>,
}

#[cfg(feature = "test-support")]
fn build_fake_instance(
    workspace: PathBuf,
    client: Arc<CodexClient>,
    thread_start_timeout: Duration,
    crash_signal: tokio::sync::mpsc::Sender<()>,
) -> Arc<AppServerInstance> {
    use std::collections::HashSet;
    use std::time::Instant;
    use tokio::sync::Mutex;
    let now = Instant::now();
    Arc::new(AppServerInstance {
        workspace,
        child: Mutex::new(None),
        client,
        thread_start_timeout,
        sessions: Mutex::new(HashSet::new()),
        spawned_at: now,
        last_activity_at: Mutex::new(now),
        crash_signal,
    })
}

/// Pick the first free port in `range` by bind-probing.
fn pick_free_port(range: std::ops::RangeInclusive<u16>) -> anyhow::Result<u16> {
    let (first, last) = (*range.start(), *range.end());
    for port in range {
        let addr = format!("127.0.0.1:{port}");
        if std::net::TcpListener::bind(&addr).is_ok() {
            return Ok(port);
        }
    }
    Err(anyhow::anyhow!(
        "all ports in range {first}..={last} occupied"
    ))
}

pub(crate) fn current_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn provider_resume_session_id<'a>(session_id: &str, handle: &'a SessionHandle) -> Option<&'a str> {
    handle
        .codex_session_id
        .as_deref()
        .filter(|provider_id| *provider_id != session_id)
}

fn codex_provider_session_id(session_id: &str, handle: &SessionHandle) -> String {
    handle
        .codex_session_id
        .clone()
        .unwrap_or_else(|| session_id.to_string())
}

fn mark_session_idle_with_tx(
    session_id: &str,
    handle: &SessionHandle,
    manager_tx: &broadcast::Sender<ManagerEvent>,
) {
    let old = handle.current_state();
    if !matches!(old, SessionState::Running { .. }) {
        return;
    }
    if handle.transition(SessionState::Idle).is_ok() {
        let _ = manager_tx.send(ManagerEvent::SessionStateChanged {
            session_id: session_id.to_string(),
            old,
            new: SessionState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
    }
}

const VALID_APPROVAL_POLICIES: &[&str] = &["never", "unless-allow-listed", "on-failure", "always"];
const VALID_SANDBOX_POLICIES: &[&str] = &["none", "read-only", "full-access"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ResolvedSessionPolicies {
    approval_policy: Option<String>,
    sandbox_policy: Option<String>,
}

#[derive(serde::Deserialize)]
struct CodexConfigPolicies {
    #[serde(default)]
    approval_policy: Option<String>,
    #[serde(default)]
    sandbox_policy: Option<String>,
}

fn env_value(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .cloned()
        .or_else(|| std::env::var(key).ok())
        .filter(|value| !value.is_empty())
}

fn codex_config_path(env: &HashMap<String, String>) -> Option<PathBuf> {
    if let Some(codex_home) = env_value(env, "CODEX_HOME") {
        return Some(PathBuf::from(codex_home).join("config.toml"));
    }
    if let Some(home) = env_value(env, "HOME").or_else(|| env_value(env, "USERPROFILE")) {
        return Some(PathBuf::from(home).join(".codex").join("config.toml"));
    }
    let home_drive = env_value(env, "HOMEDRIVE");
    let home_path = env_value(env, "HOMEPATH");
    if let (Some(home_drive), Some(home_path)) = (home_drive, home_path) {
        let mut home = PathBuf::from(home_drive);
        home.push(home_path);
        return Some(home.join(".codex").join("config.toml"));
    }
    None
}

fn validate_policy(
    value: Option<&str>,
    allowed: &[&str],
    policy_name: &str,
    source: &str,
) -> Option<String> {
    let value = value?;
    if allowed.contains(&value) {
        Some(value.to_string())
    } else {
        warn!(
            target: "minos_agent_runtime::manager",
            policy_name,
            policy_value = value,
            source,
            "ignoring invalid policy value",
        );
        None
    }
}

fn load_codex_config_policies(env: &HashMap<String, String>) -> ResolvedSessionPolicies {
    let Some(path) = codex_config_path(env) else {
        return ResolvedSessionPolicies::default();
    };
    if !path.is_file() {
        return ResolvedSessionPolicies::default();
    }

    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) => {
            warn!(
                target: "minos_agent_runtime::manager",
                error = %error,
                path = %path.display(),
                "failed to read codex config.toml; ignoring policy defaults",
            );
            return ResolvedSessionPolicies::default();
        }
    };

    let parsed: CodexConfigPolicies = match toml::from_str(&contents) {
        Ok(parsed) => parsed,
        Err(error) => {
            warn!(
                target: "minos_agent_runtime::manager",
                error = %error,
                path = %path.display(),
                "failed to parse codex config.toml; ignoring policy defaults",
            );
            return ResolvedSessionPolicies::default();
        }
    };

    let source = format!("config.toml ({})", path.display());
    ResolvedSessionPolicies {
        approval_policy: validate_policy(
            parsed.approval_policy.as_deref(),
            VALID_APPROVAL_POLICIES,
            "approval_policy",
            &source,
        ),
        sandbox_policy: validate_policy(
            parsed.sandbox_policy.as_deref(),
            VALID_SANDBOX_POLICIES,
            "sandbox_policy",
            &source,
        ),
    }
}

fn resolve_session_policies(
    overrides: Option<&SessionPolicies>,
    env: &HashMap<String, String>,
) -> ResolvedSessionPolicies {
    let defaults = load_codex_config_policies(env);
    let Some(overrides) = overrides else {
        return defaults;
    };

    ResolvedSessionPolicies {
        approval_policy: validate_policy(
            overrides.approval_policy.as_deref(),
            VALID_APPROVAL_POLICIES,
            "approval_policy",
            "session override",
        )
        .or(defaults.approval_policy),
        sandbox_policy: validate_policy(
            overrides.sandbox_policy.as_deref(),
            VALID_SANDBOX_POLICIES,
            "sandbox_policy",
            "session override",
        )
        .or(defaults.sandbox_policy),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedMcpServer {
    name: String,
    command: String,
    args: Vec<String>,
}

fn resolve_mcp_server(
    config: Option<&McpConfig>,
    _workspace: &Path,
    source_agent: AgentName,
    conversation_id: Option<&str>,
    source_session_id: Option<&str>,
) -> Option<ResolvedMcpServer> {
    let config = config?;
    let conversation_id = conversation_id?;
    let mut args = config.server_args.clone();
    args.extend([
        "--conversation-id".into(),
        conversation_id.to_owned(),
        "--source-agent".into(),
        source_agent.bin_name().into(),
        "--socket-path".into(),
        config.socket_path.display().to_string(),
    ]);
    if let Some(source_session_id) = source_session_id {
        args.extend(["--source-thread-id".into(), source_session_id.to_owned()]);
    }
    args.extend(mcp_permission_args(config.permissions));
    Some(ResolvedMcpServer {
        name: "minos_teamwork".into(),
        command: config.server_bin.display().to_string(),
        args,
    })
}

fn mcp_permission_args(
    permissions: minos_chat_store::mcp_server::McpToolPermissions,
) -> Vec<String> {
    let mut args = Vec::new();
    if !permissions.list_conversation_messages {
        args.push("--disable-list-conversation-messages".into());
    }
    if !permissions.list_conversation_roster {
        args.push("--disable-list-conversation-roster".into());
    }
    if !permissions.delegate_to_agent {
        args.push("--disable-delegate-to-agent".into());
    }
    if !permissions.get_delegation_status {
        args.push("--disable-get-delegation-status".into());
    }
    if !permissions.wait_delegation {
        args.push("--disable-wait-delegation".into());
    }
    if !permissions.cancel_delegation {
        args.push("--disable-cancel-delegation".into());
    }
    if !permissions.post_conversation_update {
        args.push("--disable-post-conversation-update".into());
    }
    if !permissions.post_git_update {
        args.push("--disable-post-git-update".into());
    }
    args
}

fn codex_mcp_config_args(server: &ResolvedMcpServer) -> Vec<String> {
    let args_value = serde_json::Value::Array(
        server
            .args
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    );
    vec![
        "-c".into(),
        format!(
            "mcp_servers.{}.command={}",
            server.name,
            toml_string(&server.command)
        ),
        "-c".into(),
        format!("mcp_servers.{}.args={}", server.name, args_value),
        "-c".into(),
        format!("mcp_servers.{}.enabled=true", server.name),
    ]
}

fn claude_mcp_config_json(server: &ResolvedMcpServer) -> String {
    let mut servers = serde_json::Map::new();
    servers.insert(
        server.name.clone(),
        serde_json::json!({
            "command": server.command.clone(),
            "args": server.args.clone(),
        }),
    );
    serde_json::json!({ "mcpServers": servers }).to_string()
}

fn opencode_config_content(server: &ResolvedMcpServer) -> String {
    let mut command = Vec::with_capacity(server.args.len() + 1);
    command.push(server.command.clone());
    command.extend(server.args.clone());
    let mut mcp = serde_json::Map::new();
    mcp.insert(
        server.name.clone(),
        serde_json::json!({
            "type": "local",
            "command": command,
            "enabled": true,
        }),
    );
    serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": mcp,
    })
    .to_string()
}

fn gemini_mcp_server(server: &ResolvedMcpServer) -> minos_acp_protocol::McpServer {
    minos_acp_protocol::McpServer {
        name: server.name.clone(),
        transport: minos_acp_protocol::McpTransport::Stdio {
            command: server.command.clone(),
            args: server.args.clone(),
            env: Vec::new(),
        },
    }
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn build_codex_spawn_args(
    listen_arg: &str,
    workspace_display: &str,
    policies: &ResolvedSessionPolicies,
    mcp_server: Option<&ResolvedMcpServer>,
) -> Vec<String> {
    let mut args = vec![
        "app-server".to_string(),
        "--listen".to_string(),
        listen_arg.to_string(),
    ];

    if let Some(approval_policy) = &policies.approval_policy {
        args.push("-c".to_string());
        args.push(format!("approval_policy={approval_policy}"));
    }
    if let Some(sandbox_policy) = &policies.sandbox_policy {
        args.push("-c".to_string());
        args.push(format!("sandbox_policy={sandbox_policy}"));
    }

    let sandbox_arg = format!(
        "sandbox_permissions=['disk-full-read-access','disk-write-folder={workspace_display}']"
    );
    args.push("-c".to_string());
    args.push(sandbox_arg);
    args.push("-c".to_string());
    args.push("shell_environment_policy.inherit=all".to_string());
    if let Some(server) = mcp_server {
        args.extend(codex_mcp_config_args(server));
    }
    args
}

fn jsonrpc_id_key(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

async fn logical_session_id_for_provider(
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    provider_session_id: &str,
) -> String {
    // Provider server-requests can arrive immediately after `thread/start`,
    // before the manager has inserted the logical handle with its provider id.
    for _ in 0..20 {
        let (session_id, known) =
            logical_session_id_for_provider_known(sessions, provider_session_id).await;
        if known {
            return session_id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    provider_session_id.to_string()
}

async fn logical_session_id_for_provider_known(
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    provider_session_id: &str,
) -> (String, bool) {
    let guard = sessions.lock().await;
    if guard.contains_key(provider_session_id) {
        return (provider_session_id.to_string(), true);
    }
    guard
        .values()
        .find_map(|handle| {
            (handle.codex_session_id.as_deref() == Some(provider_session_id))
                .then(|| handle.session_id.clone())
        })
        .map_or_else(
            || (provider_session_id.to_string(), false),
            |session_id| (session_id, true),
        )
}

fn rewrite_payload_session_id(params: &mut Value, session_id: &str) {
    if let Some(object) = params.as_object_mut() {
        object.insert(
            "threadId".to_string(),
            Value::String(session_id.to_string()),
        );
        object.insert(
            "session_id".to_string(),
            Value::String(session_id.to_string()),
        );
        if let Some(item) = object.get_mut("item").and_then(Value::as_object_mut) {
            if item.contains_key("senderThreadId") {
                item.insert(
                    "senderThreadId".to_string(),
                    Value::String(session_id.to_string()),
                );
            }
        }
    }
}

fn request_session_id(params: &Value) -> Option<String> {
    params
        .get("threadId")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn request_turn_id(params: &Value) -> String {
    params
        .get("turnId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[derive(Debug)]
#[allow(clippy::struct_field_names)] // domain ids: parent/sub thread + tool call
struct CodexSubagentRegistration {
    parent_session_id: String,
    sub_session_id: String,
    tool_call_id: String,
}

fn codex_collab_subagent_registrations(
    method: &str,
    parent_session_id: &str,
    params: &Value,
) -> Vec<CodexSubagentRegistration> {
    if method != "item/started" && method != "item/completed" {
        return Vec::new();
    }
    let item = params.get("item").unwrap_or(&Value::Null);
    if item.get("type").and_then(Value::as_str) != Some("collabAgentToolCall") {
        return Vec::new();
    }
    let parent = item
        .get("senderThreadId")
        .and_then(Value::as_str)
        .unwrap_or(parent_session_id)
        .to_string();
    item.get("receiverThreadIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|sub_session_id| CodexSubagentRegistration {
            parent_session_id: parent.clone(),
            sub_session_id: sub_session_id.to_string(),
            tool_call_id: item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

async fn register_codex_subagent_thread(
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    manager_tx: &broadcast::Sender<ManagerEvent>,
    fallback_workspace: &Path,
    registration: &CodexSubagentRegistration,
) {
    let mut guard = sessions.lock().await;
    if let Some(handle) = guard.get_mut(&registration.sub_session_id) {
        handle.parent_session_id = Some(registration.parent_session_id.clone());
        handle.codex_session_id = Some(registration.sub_session_id.clone());
        return;
    }
    let workspace = guard
        .get(&registration.parent_session_id)
        .map(|handle| handle.workspace.clone())
        .unwrap_or_else(|| fallback_workspace.to_path_buf());
    guard.insert(
        registration.sub_session_id.clone(),
        SessionHandle::new_subagent(
            registration.sub_session_id.clone(),
            workspace.clone(),
            AgentName::Codex,
            registration.parent_session_id.clone(),
            Some(registration.sub_session_id.clone()),
            SessionState::Starting,
            0,
        ),
    );
    drop(guard);
    let _ = manager_tx.send(ManagerEvent::SessionAdded {
        session_id: registration.sub_session_id.clone(),
        workspace: workspace.clone(),
        agent: AgentName::Codex,
        parent_session_id: Some(registration.parent_session_id.clone()),
    });
    info!(
        target: "minos_agent_runtime::manager",
        parent_session_id = %registration.parent_session_id,
        sub_session_id = %registration.sub_session_id,
        tool_call_id = %registration.tool_call_id,
        workspace = %workspace.display(),
        "registered codex subagent session",
    );
}

async fn non_approval_context_for_request(
    sessions: &Arc<Mutex<HashMap<String, SessionHandle>>>,
    _instance_workspace: &Path,
    session_id: Option<&str>,
) -> NonApprovalContext {
    let conversation_id = if let Some(session_id) = session_id {
        sessions
            .lock()
            .await
            .get(session_id)
            .and_then(|handle| handle.mcp_conversation_id.clone())
    } else {
        None
    };
    NonApprovalContext { conversation_id }
}

async fn broadcast_ingest(events_tx: &IngestSink, ingest: RawIngest) -> Result<(), IngestClosed> {
    events_tx.emit(ingest).await
}

pub(crate) fn approval_request_ingest(
    agent: AgentName,
    session_id: String,
    request_id: String,
    turn_id: String,
    method: String,
    params: Value,
) -> RawIngest {
    let payload_session_id = session_id.clone();
    // Approvals wait until the user decides (or the agent process dies).
    RawIngest::from_json(
        agent,
        session_id,
        serde_json::json!({
            "method": "approval/request",
            "params": {
                "request_id": request_id,
                "session_id": payload_session_id,
                "turn_id": turn_id,
                "method": method,
                "params": params,
            }
        }),
        current_unix_ms(),
    )
}

/// Long-running event-pump task per instance: drains every inbound frame from
/// the codex WS and forwards `Notification` payloads as `RawIngest` records
/// keyed by the notification's `params.threadId`.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn event_pump_loop(
    client: Arc<CodexClient>,
    events_tx: IngestSink,
    sessions: Arc<Mutex<HashMap<String, SessionHandle>>>,
    pending_approvals: PendingApprovals,
    manager_tx: broadcast::Sender<ManagerEvent>,
    workspace: PathBuf,
    crash_tx: tokio::sync::mpsc::Sender<()>,
) {
    let mut orphan_notifications: HashMap<String, Vec<(Instant, String, Value)>> = HashMap::new();
    while let Some(inbound) = client.next_inbound().await {
        match inbound {
            Inbound::Notification { method, mut params } => {
                orphan_notifications.retain(|provider_session_id, notifications| {
                    notifications.retain(|(created_at, _, _)| {
                        created_at.elapsed() < Duration::from_secs(30)
                    });
                    if notifications.is_empty() {
                        tracing::debug!(
                            target: "minos_agent_runtime::manager",
                            provider_session_id,
                            "dropped expired orphan codex notifications",
                        );
                        false
                    } else {
                        true
                    }
                });
                let provider_session_id = params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let Some(provider_session_id) = provider_session_id else {
                    continue;
                };
                let (session_id, known_thread) =
                    logical_session_id_for_provider_known(&sessions, &provider_session_id).await;
                if !known_thread {
                    orphan_notifications
                        .entry(provider_session_id)
                        .or_default()
                        .push((Instant::now(), method, params));
                    continue;
                }
                rewrite_payload_session_id(&mut params, &session_id);
                let subagent_registrations =
                    codex_collab_subagent_registrations(&method, &session_id, &params);
                for registration in &subagent_registrations {
                    register_codex_subagent_thread(
                        &sessions,
                        &manager_tx,
                        &workspace,
                        registration,
                    )
                    .await;
                    if let Some(orphaned) =
                        orphan_notifications.remove(&registration.sub_session_id)
                    {
                        for (_, orphan_method, mut orphan_params) in orphaned {
                            rewrite_payload_session_id(
                                &mut orphan_params,
                                &registration.sub_session_id,
                            );
                            let payload = serde_json::json!({
                                "method": orphan_method,
                                "params": orphan_params,
                            });
                            if let Err(error) = broadcast_ingest(
                                &events_tx,
                                RawIngest::from_json(
                                    AgentName::Codex,
                                    registration.sub_session_id.clone(),
                                    payload,
                                    current_unix_ms(),
                                ),
                            )
                            .await
                            {
                                warn!(
                                    target: "minos_agent_runtime::manager",
                                    error = %error,
                                    "event pump durable ingest sink closed",
                                );
                                break;
                            }
                        }
                    }
                }
                if method == "turn/started" {
                    let turn_id = params
                        .get("turn")
                        .and_then(|turn| turn.get("id"))
                        .or_else(|| params.get("turnId"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(turn_id) = turn_id {
                        let tg = sessions.lock().await;
                        if let Some(handle) = tg.get(&session_id) {
                            handle.set_active_turn_id(Some(turn_id));
                        }
                    }
                }
                if method == "turn/completed" {
                    let maybe_transition = {
                        let tg = sessions.lock().await;
                        tg.get(&session_id).and_then(|handle| {
                            handle.set_active_turn_id(None);
                            let old = handle.current_state();
                            if matches!(old, SessionState::Running { .. } | SessionState::Resuming)
                            {
                                handle.transition(SessionState::Idle).ok()?;
                                Some((old, SessionState::Idle))
                            } else {
                                None
                            }
                        })
                    };
                    if let Some((old, new)) = maybe_transition {
                        let _ = manager_tx.send(ManagerEvent::SessionStateChanged {
                            session_id: session_id.clone(),
                            old,
                            new,
                            at_ms: current_unix_ms(),
                        });
                    }
                }
                // Look up agent kind for the thread; default to Codex if absent
                // (notifications can race the manager's bookkeeping).
                let agent = sessions
                    .lock()
                    .await
                    .get(&session_id)
                    .map_or(AgentName::Codex, |h| h.agent);
                let payload = serde_json::json!({ "method": method, "params": params });
                let ingest = RawIngest::from_json(agent, session_id, payload, current_unix_ms());
                if let Err(error) = broadcast_ingest(&events_tx, ingest).await {
                    warn!(
                        target: "minos_agent_runtime::manager",
                        error = %error,
                        "event pump durable ingest sink closed",
                    );
                    break;
                }
            }
            Inbound::ServerRequest {
                id,
                method,
                mut params,
            } => {
                if let Some(provider_session_id) = request_session_id(&params) {
                    let session_id =
                        logical_session_id_for_provider(&sessions, &provider_session_id).await;
                    rewrite_payload_session_id(&mut params, &session_id);
                }
                let envelope = serde_json::json!({ "method": method, "params": params.clone() });
                match serde_json::from_value::<minos_codex_protocol::ServerRequest>(envelope) {
                    Ok(req) if crate::approvals::is_approval_request(&req) => {
                        let Some(session_id) = request_session_id(&params) else {
                            warn!(
                                target: "minos_agent_runtime::manager",
                                method = %method,
                                "approval request missing threadId; falling back to immediate reject",
                            );
                            if let Some(reply) = crate::approvals::auto_reject(&req) {
                                if let Err(error) = client.reply(id.clone(), reply).await {
                                    warn!(
                                        target: "minos_agent_runtime::manager",
                                        error = %error,
                                        method = %method,
                                        "fallback approval reject reply failed",
                                    );
                                }
                            }
                            continue;
                        };

                        let agent = sessions
                            .lock()
                            .await
                            .get(&session_id)
                            .map_or(AgentName::Codex, |h| h.agent);
                        let request_id = jsonrpc_id_key(&id);
                        let turn_id = request_turn_id(&params);

                        pending_approvals.insert(
                            request_id.clone(),
                            PendingApproval {
                                session_id: session_id.clone(),
                                target: PendingApprovalTarget::Codex {
                                    request_id: id.clone(),
                                    request: Box::new(req),
                                    client: client.clone(),
                                },
                            },
                        );

                        if let Err(error) = broadcast_ingest(
                            &events_tx,
                            approval_request_ingest(
                                agent,
                                session_id.clone(),
                                request_id.clone(),
                                turn_id,
                                method.clone(),
                                params.clone(),
                            ),
                        )
                        .await
                        {
                            warn!(
                                target: "minos_agent_runtime::manager",
                                error = %error,
                                "event pump durable ingest sink closed",
                            );
                            break;
                        }
                    }
                    Ok(req) => {
                        let session_id = request_session_id(&params);
                        if let Some(session_id) = session_id.clone() {
                            let agent = sessions
                                .lock()
                                .await
                                .get(&session_id)
                                .map_or(AgentName::Codex, |h| h.agent);
                            let synthetic_method = format!("server_request/{method}");
                            let payload = serde_json::json!({
                                "method": synthetic_method,
                                "params": params.clone(),
                            });
                            if let Err(error) = broadcast_ingest(
                                &events_tx,
                                RawIngest::from_json(agent, session_id, payload, current_unix_ms()),
                            )
                            .await
                            {
                                warn!(
                                    target: "minos_agent_runtime::manager",
                                    error = %error,
                                    "event pump durable ingest sink closed",
                                );
                                break;
                            }
                        }

                        let context = non_approval_context_for_request(
                            &sessions,
                            &workspace,
                            session_id.as_deref(),
                        )
                        .await;
                        if let Some(reply) =
                            crate::approvals::auto_resolve_non_approval(&req, context)
                        {
                            let server_name_for_log = params
                                .get("serverName")
                                .and_then(|value| value.as_str())
                                .unwrap_or("");
                            let mode_for_log = params
                                .get("mode")
                                .and_then(|value| value.as_str())
                                .unwrap_or("");
                            let session_id_for_log = session_id.as_deref().unwrap_or("");
                            let reply_action = reply
                                .get("action")
                                .and_then(Value::as_str)
                                .or_else(|| reply.get("answers").map(|_| "answers"))
                                .unwrap_or("reply");
                            info!(
                                target: "minos_agent_runtime::manager",
                                method = %method,
                                server_name = %server_name_for_log,
                                mode = %mode_for_log,
                                session_id = %session_id_for_log,
                                action = %reply_action,
                                "non-approval server request auto-resolved",
                            );
                            if let Err(error) = client.reply(id.clone(), reply).await {
                                warn!(
                                    target: "minos_agent_runtime::manager",
                                    error = %error,
                                    method = %method,
                                    "non-approval server request fallback reply failed",
                                );
                            }
                        } else {
                            warn!(
                                target: "minos_agent_runtime::manager",
                                method = %method,
                                "non-approval server request received; no fallback reply available",
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            target: "minos_agent_runtime::manager",
                            method = %method,
                            error = %e,
                            "unknown server request method; not replying",
                        );
                        if let Some(session_id) = request_session_id(&params) {
                            let agent = sessions
                                .lock()
                                .await
                                .get(&session_id)
                                .map_or(AgentName::Codex, |h| h.agent);
                            let synthetic_method = format!("server_request/{method}");
                            let payload = serde_json::json!({
                                "method": synthetic_method,
                                "params": params,
                            });
                            if let Err(error) = broadcast_ingest(
                                &events_tx,
                                RawIngest::from_json(agent, session_id, payload, current_unix_ms()),
                            )
                            .await
                            {
                                warn!(
                                    target: "minos_agent_runtime::manager",
                                    error = %error,
                                    "event pump durable ingest sink closed",
                                );
                                break;
                            }
                        }
                    }
                }
            }
            Inbound::Closed => break,
        }
    }
    info!(
        target: "minos_agent_runtime::manager",
        "event pump exiting (WS closed)",
    );
    let _ = crash_tx.send(()).await;
}

/// Internal helper for `AppServerInstance::start_thread`. Issues the
/// `thread/start` JSON-RPC and returns the session id (which doubles as the
/// codex session id for resume purposes per spec §6.1).
pub(crate) async fn rpc_start_thread(
    client: &CodexClient,
    cwd: &Path,
    timeout: Duration,
    developer_instructions: Option<&str>,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
) -> anyhow::Result<StartThreadResult> {
    let cwd_str = cwd.display().to_string();
    let mut config = serde_json::Map::new();
    if let Some(effort) = reasoning_effort.map(str::trim).filter(|s| !s.is_empty()) {
        config.insert(
            "model_reasoning_effort".into(),
            serde_json::Value::String(effort.to_owned()),
        );
    }
    let start_params = ThreadStartParams {
        cwd: Some(cwd_str),
        developer_instructions: developer_instructions.map(str::to_owned),
        model: model
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        config: if config.is_empty() {
            None
        } else {
            Some(config)
        },
        ..Default::default()
    };
    let resp: ThreadStartResponse = tokio::time::timeout(timeout, client.call_typed(start_params))
        .await
        .map_err(|_| anyhow::anyhow!("thread/start timeout"))?
        .map_err(|e| anyhow::anyhow!("thread/start failed: {e}"))?;
    let session_id = resp.thread.id;
    Ok(StartThreadResult {
        codex_session_id: session_id.clone(),
    })
}

#[derive(Debug, Clone)]
pub(crate) struct StartThreadResult {
    pub codex_session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentRuntimeConfig, McpConfig};
    use crate::state_machine::PauseReason;
    use crate::test_support::{FakeCodexBackend, FakeCodexServer, Step};
    use serde_json::json;
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn write_codex_config(dir: &std::path::Path, contents: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("config.toml"), contents).unwrap();
    }

    fn has_arg(args: &[String], expected: &str) -> bool {
        args.iter().any(|arg| arg == expected)
    }

    fn fake_thread_start_reply(session_id: &str) -> serde_json::Value {
        json!({
            "approvalPolicy": "never",
            "approvalsReviewer": "user",
            "cwd": "/tmp",
            "instructionSources": [],
            "model": "fake",
            "modelProvider": "fake",
            "sandbox": { "type": "dangerFullAccess" },
            "thread": {
                "id": session_id,
                "cliVersion": "0.0.0-fake",
                "createdAt": 0,
                "cwd": "/tmp",
                "ephemeral": true,
                "modelProvider": "fake",
                "preview": "",
                "source": "appServer",
                "status": { "type": "idle" },
                "turns": [],
                "updatedAt": 0
            }
        })
    }

    fn command_approval_params(session_id: &str, turn_id: &str) -> serde_json::Value {
        json!({
            "itemId": "item-1",
            "threadId": session_id,
            "turnId": turn_id,
        })
    }

    #[test]
    fn resolve_session_policies_uses_config_defaults_when_overrides_missing() {
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_config(
            codex_home.path(),
            "approval_policy = \"on-failure\"\nsandbox_policy = \"read-only\"\n",
        );
        let env = HashMap::from([(
            "CODEX_HOME".to_string(),
            codex_home.path().display().to_string(),
        )]);

        let resolved = resolve_session_policies(None, &env);

        assert_eq!(
            resolved,
            ResolvedSessionPolicies {
                approval_policy: Some("on-failure".into()),
                sandbox_policy: Some("read-only".into()),
            }
        );

        let args = build_codex_spawn_args("ws://127.0.0.1:9999", "/tmp/ws", &resolved, None);
        assert!(has_arg(&args, "approval_policy=on-failure"));
        assert!(has_arg(&args, "sandbox_policy=read-only"));
    }

    #[test]
    fn resolve_session_policies_prefers_valid_overrides_and_falls_back_for_invalid_values() {
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_config(
            codex_home.path(),
            "approval_policy = \"never\"\nsandbox_policy = \"full-access\"\n",
        );
        let env = HashMap::from([(
            "CODEX_HOME".to_string(),
            codex_home.path().display().to_string(),
        )]);
        let overrides = SessionPolicies {
            approval_policy: Some("unless-allow-listed".into()),
            sandbox_policy: Some("workspace_write".into()),
        };

        let resolved = resolve_session_policies(Some(&overrides), &env);

        assert_eq!(
            resolved,
            ResolvedSessionPolicies {
                approval_policy: Some("unless-allow-listed".into()),
                sandbox_policy: Some("full-access".into()),
            }
        );
    }

    #[test]
    fn resolve_session_policies_ignores_invalid_config_defaults() {
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_config(
            codex_home.path(),
            "approval_policy = \"on_request\"\nsandbox_policy = \"workspace_write\"\n",
        );
        let env = HashMap::from([(
            "CODEX_HOME".to_string(),
            codex_home.path().display().to_string(),
        )]);

        let resolved = resolve_session_policies(None, &env);
        let args = build_codex_spawn_args("ws://127.0.0.1:9999", "/tmp/ws", &resolved, None);

        assert_eq!(resolved, ResolvedSessionPolicies::default());
        assert!(!has_arg(&args, "approval_policy=on_request"));
        assert!(!has_arg(&args, "sandbox_policy=workspace_write"));
    }

    #[test]
    fn codex_spawn_args_include_mcp_config_when_enabled() {
        let resolved = ResolvedSessionPolicies::default();
        let server = ResolvedMcpServer {
            name: "minos_teamwork".into(),
            command: "/tmp/minos-teamwork-mcp".into(),
            args: vec![
                "--conversation-id".into(),
                "conversation-main".into(),
                "--source-agent".into(),
                "codex".into(),
                "--socket-path".into(),
                "/tmp/mcp-daemon.sock".into(),
            ],
        };

        let args =
            build_codex_spawn_args("ws://127.0.0.1:9999", "/tmp/ws", &resolved, Some(&server));

        assert!(has_arg(
            &args,
            "mcp_servers.minos_teamwork.command=\"/tmp/minos-teamwork-mcp\""
        ));
        assert!(has_arg(
            &args,
            "mcp_servers.minos_teamwork.args=[\"--conversation-id\",\"conversation-main\",\"--source-agent\",\"codex\",\"--socket-path\",\"/tmp/mcp-daemon.sock\"]"
        ));
        assert!(has_arg(&args, "mcp_servers.minos_teamwork.enabled=true"));
    }

    #[test]
    fn resolve_mcp_server_preserves_command_prefix_args() {
        let config = McpConfig {
            server_bin: "/tmp/minos-tui".into(),
            server_args: vec!["minos-teamwork-mcp".into()],
            socket_path: "/tmp/mcp-test.sock".into(),
            db_path: "/tmp/minos.sqlite".into(),
            permissions: minos_chat_store::mcp_server::McpToolPermissions::default(),
        };

        let server = resolve_mcp_server(
            Some(&config),
            std::path::Path::new("/tmp/minos"),
            AgentName::Gemini,
            Some("conversation-main"),
            Some("thread-source-1234"),
        )
        .expect("chat MCP should resolve");

        assert_eq!(server.command, "/tmp/minos-tui");
        assert_eq!(
            server.args,
            vec![
                "minos-teamwork-mcp",
                "--conversation-id",
                "conversation-main",
                "--source-agent",
                "gemini",
                "--socket-path",
                "/tmp/mcp-test.sock",
                "--source-thread-id",
                "thread-source-1234"
            ]
        );
    }

    #[test]
    fn claude_mcp_config_json_includes_bound_conversation_and_source_agent() {
        let server = ResolvedMcpServer {
            name: "minos_teamwork".into(),
            command: "/tmp/minos-tui".into(),
            args: vec![
                "minos-teamwork-mcp".into(),
                "--conversation-id".into(),
                "conversation-main".into(),
                "--source-agent".into(),
                "claude".into(),
                "--socket-path".into(),
                "/tmp/mcp-test.sock".into(),
            ],
        };

        let config: Value =
            serde_json::from_str(&claude_mcp_config_json(&server)).expect("valid JSON");

        assert_eq!(
            config["mcpServers"]["minos_teamwork"]["command"],
            "/tmp/minos-tui"
        );
        assert_eq!(
            config["mcpServers"]["minos_teamwork"]["args"][0],
            "minos-teamwork-mcp"
        );
        assert!(config["mcpServers"]["minos_teamwork"]["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--conversation-id" && pair[1] == "conversation-main"));
        assert!(config["mcpServers"]["minos_teamwork"]["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--source-agent" && pair[1] == "claude"));
        assert!(config["mcpServers"]["minos_teamwork"]["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--socket-path" && pair[1] == "/tmp/mcp-test.sock"));
    }

    #[test]
    fn opencode_config_content_includes_local_minos_teamwork_server() {
        let server = ResolvedMcpServer {
            name: "minos_teamwork".into(),
            command: "/tmp/minos-tui".into(),
            args: vec![
                "minos-teamwork-mcp".into(),
                "--conversation-id".into(),
                "conversation-main".into(),
                "--source-agent".into(),
                "opencode".into(),
                "--socket-path".into(),
                "/tmp/mcp-test.sock".into(),
            ],
        };

        let config: Value =
            serde_json::from_str(&opencode_config_content(&server)).expect("valid JSON");

        assert_eq!(config["mcp"]["minos_teamwork"]["type"], "local");
        assert_eq!(config["mcp"]["minos_teamwork"]["enabled"], true);
        assert_eq!(
            config["mcp"]["minos_teamwork"]["command"][0],
            "/tmp/minos-tui"
        );
        assert_eq!(
            config["mcp"]["minos_teamwork"]["command"][1],
            "minos-teamwork-mcp"
        );
        assert!(config["mcp"]["minos_teamwork"]["command"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--source-agent" && pair[1] == "opencode"));
    }

    #[test]
    fn gemini_mcp_server_includes_bound_conversation_and_source_agent() {
        let server = ResolvedMcpServer {
            name: "minos_teamwork".into(),
            command: "/tmp/minos-tui".into(),
            args: vec![
                "minos-teamwork-mcp".into(),
                "--conversation-id".into(),
                "conversation-main".into(),
                "--source-agent".into(),
                "gemini".into(),
                "--socket-path".into(),
                "/tmp/mcp-test.sock".into(),
            ],
        };

        let config = serde_json::to_value(gemini_mcp_server(&server)).expect("valid JSON");

        assert_eq!(config["name"], "minos_teamwork");
        assert_eq!(config["command"], "/tmp/minos-tui");
        assert_eq!(config["args"][0], "minos-teamwork-mcp");
        assert!(config["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--conversation-id" && pair[1] == "conversation-main"));
        assert!(config["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--source-agent" && pair[1] == "gemini"));
    }

    #[tokio::test]
    async fn start_agent_creates_instance_and_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let ws = std::path::PathBuf::from("/w-test");
        let resp = mgr.start_agent(AgentKind::Codex, ws.clone()).await.unwrap();
        assert_eq!(resp.cwd, ws);
        let snap = mgr.list_sessions().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].workspace, ws);
        assert!(matches!(
            mgr.session_state(&resp.session_id).await,
            Some(SessionState::Idle)
        ));
        assert_eq!(
            mgr.open_workspaces().await,
            vec![std::path::PathBuf::from("/w-test")]
        );
        fake.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn codex_thread_start_includes_minos_teamwork_developer_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-minos-instructions";
        let script = vec![Step::ExpectRequestMatching {
            method: "thread/start".into(),
            params_subset: json!({
                "developerInstructions": MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS,
            }),
            reply: fake_thread_start_reply(session_id),
        }];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let started = mgr
            .start_agent(AgentName::Codex, tmp.path().to_path_buf())
            .await
            .unwrap();

        assert_eq!(started.session_id, session_id);
        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn codex_thread_start_honors_configured_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let script = vec![
            Step::Sleep { ms: 25 },
            Step::ExpectRequestMatching {
                method: "thread/start".into(),
                params_subset: json!({
                    "developerInstructions": MINOS_TEAMWORK_DEVELOPER_INSTRUCTIONS,
                }),
                reply: fake_thread_start_reply("thr-too-late"),
            },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.thread_start_timeout = Duration::from_millis(5);
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let error = mgr
            .start_agent(AgentName::Codex, tmp.path().to_path_buf())
            .await
            .expect_err("thread/start should honor configured timeout");

        assert!(error.to_string().contains("thread/start timeout"));
        server.stop().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gemini_send_user_message_runs_acp_prompt_and_returns_idle() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("fake-gemini.sh");
        std::fs::write(
            &script_path,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,"authMethods":[],"agentCapabilities":{}}}\n' "$id"
      ;;
    session/new)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"session-1"}}\n' "$id"
      ;;
    session/prompt)
      printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"gemini says hi"}}}}\n'
      printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"end_turn"}}\n' "$id"
      ;;
    session/close)
      printf '{"jsonrpc":"2.0","id":"%s","result":{}}\n' "$id"
      exit 0
      ;;
  esac
done
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.gemini_bin = Some(script_path);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let started = mgr
            .start_agent(AgentName::Gemini, tmp.path().to_path_buf())
            .await
            .unwrap();
        let session_id = started.session_id.clone();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(&session_id, "ping".into())
            .await
            .unwrap();

        let user = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("synthetic Gemini user message should arrive")
            .expect("ingest stream should stay open");
        assert_eq!(user.session_id, session_id);
        assert_eq!(
            user.json_value()
                .expect("raw ingest should contain JSON payload")
                .get("kind")
                .and_then(Value::as_str),
            Some("user_message")
        );
        assert_eq!(
            user.json_value()
                .expect("raw ingest should contain JSON payload")
                .get("text")
                .and_then(Value::as_str),
            Some("ping")
        );
        assert!(user
            .json_value()
            .expect("raw ingest should contain JSON payload")
            .get("messageId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty()));

        let chunk = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.session_id == session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("kind")
                        .and_then(Value::as_str)
                        == Some("acp_notification")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("fake Gemini ACP notification should arrive");

        assert_eq!(
            chunk
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["update"]["content"]
                ["text"],
            "gemini says hi"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    mgr.session_state(&session_id).await,
                    Some(SessionState::Idle)
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Gemini prompt task should return thread to idle");
        assert!(matches!(
            mgr.session_state(&session_id).await,
            Some(SessionState::Idle)
        ));

        mgr.close_session(&session_id).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grok_send_user_message_runs_acp_prompt_and_returns_idle() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("fake-grok.sh");
        std::fs::write(
            &script_path,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,"authMethods":[],"agentCapabilities":{}}}\n' "$id"
      ;;
    session/new)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"session-1"}}\n' "$id"
      ;;
    session/prompt)
      printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"grok says hi"}}}}\n'
      printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"end_turn"}}\n' "$id"
      ;;
    session/close)
      printf '{"jsonrpc":"2.0","id":"%s","result":{}}\n' "$id"
      exit 0
      ;;
  esac
done
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.grok_bin = Some(script_path);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let started = mgr
            .start_agent(AgentName::Grok, tmp.path().to_path_buf())
            .await
            .unwrap();
        let session_id = started.session_id.clone();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(&session_id, "ping".into())
            .await
            .unwrap();

        let user = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("synthetic Grok user message should arrive")
            .expect("ingest stream should stay open");
        assert_eq!(user.session_id, session_id);
        assert_eq!(
            user.json_value()
                .expect("raw ingest should contain JSON payload")
                .get("kind")
                .and_then(Value::as_str),
            Some("user_message")
        );
        assert_eq!(
            user.json_value()
                .expect("raw ingest should contain JSON payload")
                .get("text")
                .and_then(Value::as_str),
            Some("ping")
        );
        assert!(user
            .json_value()
            .expect("raw ingest should contain JSON payload")
            .get("messageId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty()));

        let chunk = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.session_id == session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("kind")
                        .and_then(Value::as_str)
                        == Some("acp_notification")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("fake Grok ACP notification should arrive");

        assert_eq!(
            chunk
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["update"]["content"]
                ["text"],
            "grok says hi"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    mgr.session_state(&session_id).await,
                    Some(SessionState::Idle)
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Grok prompt task should return thread to idle");
        assert!(matches!(
            mgr.session_state(&session_id).await,
            Some(SessionState::Idle)
        ));

        mgr.close_session(&session_id).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gemini_session_new_uses_gemini_acp_mcp_server_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("fake-gemini-mcp-shape.sh");
        let request_path = tmp.path().join("session-new.json");
        std::fs::write(
            &script_path,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,"authMethods":[],"agentCapabilities":{}}}\n' "$id"
      ;;
    session/new)
      printf '%s\n' "$line" > "$FAKE_GEMINI_SESSION_NEW"
      printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"session-1"}}\n' "$id"
      ;;
    session/close)
      printf '{"jsonrpc":"2.0","id":"%s","result":{}}\n' "$id"
      exit 0
      ;;
  esac
done
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.gemini_bin = Some(script_path);
        cfg.mcp = Some(McpConfig {
            server_bin: "/tmp/minos-tui".into(),
            server_args: vec!["minos-teamwork-mcp".into()],
            socket_path: "/tmp/mcp-test.sock".into(),
            db_path: "/tmp/minos-chat.sqlite".into(),
            permissions: minos_chat_store::mcp_server::McpToolPermissions::default(),
        });
        cfg.subprocess_env = Arc::new(HashMap::from([(
            "FAKE_GEMINI_SESSION_NEW".to_string(),
            request_path.display().to_string(),
        )]));
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let started = mgr
            .start_agent_in_conversation(
                AgentName::Gemini,
                tmp.path().to_path_buf(),
                "conversation-main".into(),
            )
            .await
            .unwrap();

        let request: Value = serde_json::from_str(&std::fs::read_to_string(request_path).unwrap())
            .expect("session/new request should be JSON");
        let mcp_server = &request["params"]["mcpServers"][0];
        assert_eq!(mcp_server["name"], "minos_teamwork");
        assert_eq!(mcp_server["command"], "/tmp/minos-tui");
        assert_eq!(mcp_server["args"][0], "minos-teamwork-mcp");
        assert!(mcp_server["args"]
            .as_array()
            .unwrap()
            .iter()
            .all(|arg| arg != "--db-path"));
        assert!(mcp_server["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--conversation-id" && pair[1] == "conversation-main"));
        assert!(mcp_server["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--source-agent" && pair[1] == "gemini"));
        assert!(mcp_server["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--socket-path" && pair[1] == "/tmp/mcp-test.sock"));
        assert!(mcp_server["args"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--source-thread-id" && pair[1] == started.session_id.as_str()));
        assert!(mcp_server.get("transportType").is_none());
        assert!(mcp_server.get("type").is_none());
        assert_eq!(mcp_server["env"], json!([]));

        mgr.close_session(&started.session_id).await.unwrap();
    }

    #[tokio::test]
    async fn persisted_thread_restores_mcp_conversation_context() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = AgentManager::new(
            AgentRuntimeConfig::new(tmp.path().to_path_buf()),
            InstanceCaps::default(),
        );

        mgr.register_persisted_thread(
            "thread-codex-1234".into(),
            tmp.path().to_path_buf(),
            AgentName::Codex,
            Some("provider-codex-1234".into()),
            None,
            Some("conversation-main".into()),
            SessionState::Idle,
            0,
        )
        .await
        .unwrap();

        let sessions = mgr.sessions.lock().await;
        let handle = sessions
            .get("thread-codex-1234")
            .expect("thread should be registered");
        assert_eq!(
            handle.mcp_conversation_id.as_deref(),
            Some("conversation-main")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gemini_server_permission_request_waits_for_user_decision() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("fake-gemini-permission.sh");
        let reply_path = tmp.path().join("permission-reply.json");
        std::fs::write(
            &script_path,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,"authMethods":[],"agentCapabilities":{}}}\n' "$id"
      ;;
    session/new)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"session-1"}}\n' "$id"
      ;;
    session/prompt)
      printf '{"jsonrpc":"2.0","id":"perm-1","method":"session/request_permission","params":{"sessionId":"session-1","options":[{"optionId":"proceed_once","name":"Allow","kind":"allow_once"},{"optionId":"cancel","name":"Reject","kind":"reject_once"}],"toolCall":{"toolCallId":"tool-1","status":"pending","title":"fake tool","kind":"other"}}}\n'
      IFS= read -r reply || exit 1
      printf '%s\n' "$reply" > "$FAKE_GEMINI_PERMISSION_REPLY"
      case "$reply" in
        *'"optionId":"proceed_once"'*)
          printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"permission was allowed"}}}}\n'
          printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"end_turn"}}\n' "$id"
          ;;
        *)
          printf '{"jsonrpc":"2.0","id":"%s","error":{"code":-32000,"message":"bad permission reply"}}\n' "$id"
          ;;
      esac
      ;;
    session/close)
      printf '{"jsonrpc":"2.0","id":"%s","result":{}}\n' "$id"
      exit 0
      ;;
  esac
done
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.gemini_bin = Some(script_path);
        cfg.subprocess_env = Arc::new(HashMap::from([(
            "FAKE_GEMINI_PERMISSION_REPLY".to_string(),
            reply_path.display().to_string(),
        )]));
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let started = mgr
            .start_agent(AgentName::Gemini, tmp.path().to_path_buf())
            .await
            .unwrap();
        let session_id = started.session_id.clone();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(&session_id, "please use a tool".into())
            .await
            .unwrap();

        let approval = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                let payload = ingest
                    .json_value()
                    .expect("raw ingest should contain JSON payload");
                if ingest.session_id == session_id
                    && payload.get("method").and_then(Value::as_str) == Some("approval/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("Gemini ACP permission should surface as approval/request");

        let request_id = approval
            .json_value()
            .expect("payload")
            .get("params")
            .and_then(|p| p.get("request_id"))
            .and_then(Value::as_str)
            .expect("request_id")
            .to_string();
        assert_eq!(request_id, "perm-1");

        mgr.resolve_approval(
            &request_id,
            &session_id,
            serde_json::json!({ "approved": true }),
        )
        .await
        .expect("user approval should resolve");

        let chunk = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.session_id == session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("kind")
                        .and_then(Value::as_str)
                        == Some("acp_notification")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("fake Gemini should continue after permission approval");
        assert_eq!(
            chunk
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["update"]["content"]
                ["text"],
            "permission was allowed"
        );

        let reply = std::fs::read_to_string(&reply_path).unwrap();
        assert!(
            reply.contains(r#""optionId":"proceed_once""#)
                || reply.contains(r#""option_id":"proceed_once""#),
            "reply was: {reply}"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    mgr.session_state(&session_id).await,
                    Some(SessionState::Idle)
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Gemini prompt task should return thread to idle");

        mgr.close_session(&session_id).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gemini_suspended_thread_recreates_acp_instance_before_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("fake-gemini-resume.sh");
        std::fs::write(
            &script_path,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,"authMethods":[],"agentCapabilities":{"resume":{}}}}\n' "$id"
      ;;
    session/resume)
      printf '{"jsonrpc":"2.0","id":"%s","result":{}}\n' "$id"
      ;;
    session/new)
      printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"new-session"}}\n' "$id"
      ;;
    session/prompt)
      printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"resume-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"resumed gemini"}}}}\n'
      printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"end_turn"}}\n' "$id"
      ;;
    session/close)
      printf '{"jsonrpc":"2.0","id":"%s","result":{}}\n' "$id"
      exit 0
      ;;
  esac
done
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.gemini_bin = Some(script_path);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let session_id = "gemini-resume-thread";
        mgr.register_persisted_thread(
            session_id.into(),
            tmp.path().to_path_buf(),
            AgentName::Gemini,
            Some("resume-session".into()),
            None,
            None,
            SessionState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            4,
        )
        .await
        .unwrap();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(session_id, "continue".into())
            .await
            .unwrap();

        let chunk = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.session_id == session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("kind")
                        .and_then(Value::as_str)
                        == Some("acp_notification")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("resumed Gemini ACP notification should arrive");

        assert_eq!(
            chunk
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["update"]["content"]
                ["text"],
            "resumed gemini"
        );
        assert_eq!(
            mgr.session_provider_session_id(session_id).await.as_deref(),
            Some("resume-session")
        );
        mgr.close_session(session_id).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_first_turn_uses_generated_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let script_path = bin_dir.join("claude");
        let args_path = tmp.path().join("claude-first-args.txt");
        std::fs::write(
            &script_path,
            r#"#!/bin/sh
printf '%s\n' "$*" > "$FAKE_CLAUDE_ARGS"
printf '{"type":"result","is_error":false}\n'
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.subprocess_env = Arc::new(HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            (
                "FAKE_CLAUDE_ARGS".to_string(),
                args_path.display().to_string(),
            ),
        ]));
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let started = mgr
            .start_agent(AgentName::Claude, tmp.path().to_path_buf())
            .await
            .unwrap();
        let provider_session_id = started
            .provider_session_id
            .as_deref()
            .expect("claude start should allocate a provider session id");
        uuid::Uuid::parse_str(provider_session_id).expect("provider session id must be a UUID");

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(&started.session_id, "first claude".into())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.session_id == started.session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("type")
                        .and_then(Value::as_str)
                        == Some("result")
                {
                    break;
                }
            }
        })
        .await
        .expect("fake Claude result should arrive");

        let args = std::fs::read_to_string(args_path).unwrap();
        assert!(args.contains(&format!("--session-id {provider_session_id}")));
        assert!(!args.contains("--resume"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_suspended_thread_starts_claude_turn_with_provider_session() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let script_path = bin_dir.join("claude");
        let args_path = tmp.path().join("claude-args.txt");
        let provider_session_id = "cbdad4f3-ca95-4ac3-9bd0-2d6ec33fdc3d";
        std::fs::write(
            &script_path,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$*" > "$FAKE_CLAUDE_ARGS"
printf '{{"type":"result","session_id":"{provider_session_id}","is_error":false}}\n'
"#
            ),
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.subprocess_env = Arc::new(HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            (
                "FAKE_CLAUDE_ARGS".to_string(),
                args_path.display().to_string(),
            ),
        ]));
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let session_id = "claude-resume-thread";
        mgr.register_persisted_thread(
            session_id.into(),
            tmp.path().to_path_buf(),
            AgentName::Claude,
            Some(provider_session_id.into()),
            None,
            None,
            SessionState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            9,
        )
        .await
        .unwrap();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(session_id, "continue claude".into())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.session_id == session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("type")
                        .and_then(Value::as_str)
                        == Some("result")
                {
                    break;
                }
            }
        })
        .await
        .expect("fake Claude result should arrive");

        let args = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(args) = std::fs::read_to_string(&args_path) {
                    break args;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fake Claude args should be written");
        assert!(args.contains(&format!("--resume {provider_session_id}")));
        assert!(matches!(
            mgr.session_state(session_id).await,
            Some(SessionState::Idle)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn running_claude_message_resumes_bound_session_and_synthesizes_user_message() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let script_path = bin_dir.join("claude");
        let args_path = tmp.path().join("claude-running-args.txt");
        let provider_session_id = "d5f4d81e-c934-4551-a8d0-bf3ef6db96cc";
        std::fs::write(
            &script_path,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$*" > "$FAKE_CLAUDE_ARGS"
printf '{{"type":"result","session_id":"{provider_session_id}","is_error":false}}\n'
"#
            ),
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.subprocess_env = Arc::new(HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            (
                "FAKE_CLAUDE_ARGS".to_string(),
                args_path.display().to_string(),
            ),
        ]));
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let session_id = "claude-running-thread";
        mgr.register_persisted_thread(
            session_id.into(),
            tmp.path().to_path_buf(),
            AgentName::Claude,
            Some(provider_session_id.into()),
            None,
            None,
            SessionState::Running {
                turn_started_at_ms: 1,
            },
            1,
        )
        .await
        .unwrap();

        let mut rx = mgr.ingest_stream();
        let outcome = mgr
            .dispatch_message(
                AgentName::Claude,
                "/unused".into(),
                Some(session_id.into()),
                "answer while running".into(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(outcome.session_id, session_id);

        let args = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(args) = std::fs::read_to_string(&args_path) {
                    break args;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fake Claude args should be written");
        assert!(args.contains(&format!("--resume {provider_session_id}")));

        let user = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.session_id == session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("method")
                        .and_then(Value::as_str)
                        == Some("item/started")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("synthetic Claude user message should arrive");
        assert_eq!(
            user.json_value()
                .expect("raw ingest should contain JSON payload")["params"]["item"]["content"][0]
                ["text"],
            "answer while running"
        );
    }

    #[tokio::test]
    async fn running_opencode_message_uses_prompt_async_and_synthesizes_user_message() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tmp.path()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (request_tx, request_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let headers_end = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .unwrap()
                        + 4;
                    let headers = String::from_utf8_lossy(&request[..headers_end]);
                    let content_len = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= headers_end + content_len {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&request).to_string();
            let _ = request_tx.send(text);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let session_id = "opencode-running-thread";
        mgr.register_persisted_thread(
            session_id.into(),
            workspace.clone(),
            AgentName::Opencode,
            Some("sess_running".into()),
            None,
            None,
            SessionState::Running {
                turn_started_at_ms: 1,
            },
            0,
        )
        .await
        .unwrap();
        let instance = crate::opencode_driver::OpencodeServerInstance {
            workspace: workspace.clone(),
            config: crate::opencode_driver::OpencodeServerConfig {
                opencode_bin: "opencode".into(),
                port: addr.port(),
                password: "pw".into(),
                subprocess_env: Arc::new(HashMap::new()),
                opencode_config_content: None,
            },
            child: None,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            base_url: format!("http://{addr}"),
            auth_header: "Basic test".into(),
        };
        let handle = mgr.sessions.lock().await.get(session_id).cloned().unwrap();
        mgr.opencode_instances.lock().await.insert(
            InstanceKey::for_handle(&handle),
            Arc::new(Mutex::new(instance)),
        );

        let mut rx = mgr.ingest_stream();
        let outcome = mgr
            .dispatch_message(
                AgentName::Opencode,
                "/unused".into(),
                Some(session_id.into()),
                "running answer".into(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(outcome.session_id, session_id);

        let request = request_rx.await.unwrap();
        assert!(request.starts_with("POST /session/sess_running/prompt_async "));
        assert!(request.contains(r#""text":"running answer""#));

        let user = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("synthetic opencode user message should arrive")
            .expect("ingest stream should stay open");
        assert_eq!(user.session_id, session_id);
        assert_eq!(
            user.json_value()
                .expect("raw ingest should contain JSON payload")
                .get("method")
                .and_then(Value::as_str),
            Some("item/started")
        );
        assert_eq!(
            user.json_value()
                .expect("raw ingest should contain JSON payload")["params"]["item"]["content"][0]
                ["text"],
            "running answer"
        );
    }

    #[tokio::test]
    async fn reattach_and_send_from_suspended() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-resume".into())
            .await
            .unwrap();
        mgr.interrupt_session(&started.session_id).await.unwrap();
        assert!(matches!(
            mgr.session_state(&started.session_id).await,
            Some(SessionState::Suspended {
                reason: PauseReason::UserInterrupt
            })
        ));

        mgr.reattach_suspended_thread(&started.session_id)
            .await
            .unwrap();
        assert!(matches!(
            mgr.session_state(&started.session_id).await,
            Some(SessionState::Idle)
        ));

        mgr.send_user_message(&started.session_id, "resume".into())
            .await
            .unwrap();
        assert!(matches!(
            mgr.session_state(&started.session_id).await,
            Some(SessionState::Running { .. })
        ));
        fake.stop().await;
    }

    #[tokio::test]
    async fn suspend_for_daemon_stop_sets_continue_only_when_running() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

        let idle = mgr
            .start_agent(AgentKind::Codex, "/w-stop-idle".into())
            .await
            .unwrap();
        let needs = mgr.suspend_for_daemon_stop(&idle.session_id).await.unwrap();
        assert!(!needs);
        assert!(matches!(
            mgr.session_state(&idle.session_id).await,
            Some(SessionState::Suspended {
                reason: PauseReason::DaemonRestart
            })
        ));

        let running = mgr
            .start_agent(AgentKind::Codex, "/w-stop-run".into())
            .await
            .unwrap();
        mgr.send_user_message(&running.session_id, "go".into())
            .await
            .unwrap();
        let needs = mgr
            .suspend_for_daemon_stop(&running.session_id)
            .await
            .unwrap();
        assert!(needs);
        assert!(matches!(
            mgr.session_state(&running.session_id).await,
            Some(SessionState::Suspended {
                reason: PauseReason::DaemonRestart
            })
        ));
        fake.stop().await;
    }

    #[tokio::test]
    async fn send_user_message_steers_when_thread_is_running() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-steer".into())
            .await
            .unwrap();

        mgr.send_user_message(&started.session_id, "first".into())
            .await
            .unwrap();
        let first_turn_id = mgr
            .sessions
            .lock()
            .await
            .get(&started.session_id)
            .and_then(SessionHandle::active_turn_id)
            .expect("turn/start should record an active turn id");

        mgr.send_user_message(&started.session_id, "second".into())
            .await
            .unwrap();

        let second_turn_id = mgr
            .sessions
            .lock()
            .await
            .get(&started.session_id)
            .and_then(SessionHandle::active_turn_id)
            .expect("turn/steer should preserve an active turn id");
        assert_eq!(second_turn_id, first_turn_id);
        assert!(matches!(
            mgr.session_state(&started.session_id).await,
            Some(SessionState::Running { .. })
        ));

        fake.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::too_many_lines)]
    async fn turn_notifications_update_active_turn_id_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-turn-lifecycle";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: serde_json::json!({
                    "approvalPolicy": "never",
                    "approvalsReviewer": "user",
                    "cwd": "/tmp",
                    "instructionSources": [],
                    "model": "fake",
                    "modelProvider": "fake",
                    "sandbox": { "type": "dangerFullAccess" },
                    "thread": {
                        "id": session_id,
                        "cliVersion": "0.0.0-fake",
                        "createdAt": 0,
                        "cwd": "/tmp",
                        "ephemeral": true,
                        "modelProvider": "fake",
                        "preview": "",
                        "source": "appServer",
                        "status": { "type": "idle" },
                        "turns": [],
                        "updatedAt": 0
                    }
                }),
            },
            Step::ExpectRequest {
                method: "turn/start".into(),
                reply: json!({
                    "turn": {
                        "id": "turn-from-response",
                        "items": [],
                        "status": "inProgress"
                    }
                }),
            },
            Step::EmitNotification {
                method: "turn/started".into(),
                params: json!({
                    "threadId": session_id,
                    "turn": {
                        "id": "turn-from-notification",
                        "items": [],
                        "status": "inProgress"
                    }
                }),
            },
            Step::Sleep { ms: 750 },
            Step::EmitNotification {
                method: "turn/completed".into(),
                params: json!({
                    "threadId": session_id,
                    "finishedAtMs": 123
                }),
            },
            Step::Sleep { ms: 100 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-turn-lifecycle".into())
            .await
            .unwrap();
        assert_eq!(started.session_id, session_id);

        let mut ingest_rx = mgr.ingest_stream();

        mgr.send_user_message(session_id, "hello".into())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest broadcast should stay open");
                if ingest.session_id == session_id
                    && ingest
                        .json_value()
                        .expect("raw ingest should contain JSON payload")
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        == Some("turn/started")
                {
                    break;
                }
            }
        })
        .await
        .expect("turn/started ingest should arrive");

        let turn_id = mgr
            .sessions
            .lock()
            .await
            .get(session_id)
            .and_then(SessionHandle::active_turn_id);
        assert_eq!(turn_id.as_deref(), Some("turn-from-notification"));

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = mgr.session_state(session_id).await;
                let turn_id = mgr
                    .sessions
                    .lock()
                    .await
                    .get(session_id)
                    .and_then(SessionHandle::active_turn_id);
                if matches!(state, Some(SessionState::Idle)) && turn_id.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("turn/completed should clear the active turn id and return to idle");

        server.stop().await;
    }

    #[tokio::test]
    async fn dispatch_message_creates_session_when_missing_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let outcome = mgr
            .dispatch_message(
                AgentKind::Codex,
                "/w-dispatch-new".into(),
                None,
                "hello".into(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.cwd, std::path::PathBuf::from("/w-dispatch-new"));
        assert!(mgr.has_thread(&outcome.session_id).await);
        assert!(matches!(
            mgr.session_state(&outcome.session_id).await,
            Some(SessionState::Running { .. })
        ));

        fake.stop().await;
    }

    #[tokio::test]
    async fn dispatch_message_sends_on_idle_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-dispatch-idle".into())
            .await
            .unwrap();

        let outcome = mgr
            .dispatch_message(
                AgentKind::Codex,
                "/unused".into(),
                Some(started.session_id.clone()),
                "hello".into(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.session_id, started.session_id);
        assert!(matches!(
            mgr.session_state(&outcome.session_id).await,
            Some(SessionState::Running { .. })
        ));

        fake.stop().await;
    }

    #[tokio::test]
    async fn dispatch_message_steers_running_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-dispatch-running".into())
            .await
            .unwrap();
        mgr.send_user_message(&started.session_id, "first".into())
            .await
            .unwrap();
        let first_turn_id = mgr
            .sessions
            .lock()
            .await
            .get(&started.session_id)
            .and_then(SessionHandle::active_turn_id)
            .expect("turn/start should record turn id before steer");

        let outcome = mgr
            .dispatch_message(
                AgentKind::Codex,
                "/unused".into(),
                Some(started.session_id.clone()),
                "second".into(),
                None,
            )
            .await
            .unwrap();

        let second_turn_id = mgr
            .sessions
            .lock()
            .await
            .get(&started.session_id)
            .and_then(SessionHandle::active_turn_id)
            .expect("turn/steer should preserve turn id");
        assert_eq!(outcome.session_id, started.session_id);
        assert_eq!(second_turn_id, first_turn_id);

        fake.stop().await;
    }

    #[tokio::test]
    async fn dispatch_message_resumes_suspended_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-dispatch-suspended".into())
            .await
            .unwrap();
        mgr.interrupt_session(&started.session_id).await.unwrap();

        let outcome = mgr
            .dispatch_message(
                AgentKind::Codex,
                "/unused".into(),
                Some(started.session_id.clone()),
                "resume".into(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.session_id, started.session_id);
        assert!(matches!(
            mgr.session_state(&outcome.session_id).await,
            Some(SessionState::Running { .. })
        ));

        fake.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn approval_requests_are_forwarded_as_ingest_and_tracked() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-approval-forward";
        let turn_id = "turn-approval-forward";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(session_id),
            },
            Step::EmitServerRequest {
                method: "item/commandExecution/requestApproval".into(),
                params: command_approval_params(session_id, turn_id),
            },
            Step::Sleep { ms: 100 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-approval-forward".into())
            .await
            .unwrap();
        assert_eq!(started.session_id, session_id);

        let ingest = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest
                    .json_value()
                    .expect("raw ingest should contain JSON payload")
                    .get("method")
                    .and_then(Value::as_str)
                    == Some("approval/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("approval/request ingest should arrive");

        let request_id = server
            .server_request_ids()
            .await
            .into_iter()
            .next()
            .expect("server request id should be recorded");
        assert_eq!(ingest.session_id, session_id);
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["request_id"],
            json!(request_id)
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["session_id"],
            json!(session_id)
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["turn_id"],
            json!(turn_id)
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["method"],
            json!("item/commandExecution/requestApproval")
        );
        assert!(mgr.pending_approvals.contains_key(&request_id));

        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mcp_elicitation_requests_are_forwarded_and_auto_cancelled() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-mcp-elicit";
        let turn_id = "turn-mcp-elicit";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(session_id),
            },
            Step::EmitServerRequest {
                method: "mcpServer/elicitation/request".into(),
                params: json!({
                    "elicitationId": "elic-1",
                    "message": "Open this URL",
                    "mode": "url",
                    "serverName": "minos_teamwork",
                    "threadId": session_id,
                    "turnId": turn_id,
                    "url": "https://example.com"
                }),
            },
            Step::ExpectResponse {
                result: json!({ "action": "cancel" }),
            },
            Step::Sleep { ms: 20 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-mcp-elicit".into())
            .await
            .unwrap();
        assert_eq!(started.session_id, session_id);

        let ingest = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest
                    .json_value()
                    .expect("raw ingest should contain JSON payload")
                    .get("method")
                    .and_then(Value::as_str)
                    == Some("server_request/mcpServer/elicitation/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("mcp elicitation synthetic ingest should arrive");

        assert_eq!(ingest.session_id, session_id);
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["threadId"],
            json!(session_id)
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["turnId"],
            json!(turn_id)
        );

        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn minos_teamwork_form_elicitation_is_forwarded_and_auto_accepted_with_conversation() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-mcp-chat";
        let turn_id = "turn-mcp-chat";
        let workspace = PathBuf::from("/w-mcp-chat");
        let conversation_id = "conversation-main";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(session_id),
            },
            Step::EmitServerRequest {
                method: "mcpServer/elicitation/request".into(),
                params: json!({
                    "message": "Select the Minos conversation to read",
                    "mode": "form",
                    "requestedSchema": {
                        "type": "object",
                        "properties": {
                            "conversation_id": { "type": "string" }
                        },
                        "required": ["conversation_id"]
                    },
                    "serverName": "minos_teamwork",
                    "threadId": session_id,
                    "turnId": turn_id,
                }),
            },
            Step::ExpectResponse {
                result: json!({
                    "action": "accept",
                    "content": { "conversation_id": conversation_id }
                }),
            },
            Step::Sleep { ms: 20 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        let started = mgr
            .start_agent_in_conversation(AgentKind::Codex, workspace, conversation_id.to_owned())
            .await
            .unwrap();
        assert_ne!(started.session_id, session_id);
        assert_eq!(started.provider_session_id.as_deref(), Some(session_id));

        let ingest = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest
                    .json_value()
                    .expect("raw ingest should contain JSON payload")
                    .get("method")
                    .and_then(Value::as_str)
                    == Some("server_request/mcpServer/elicitation/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("mcp elicitation synthetic ingest should arrive");

        assert_eq!(ingest.session_id, started.session_id);
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["serverName"],
            json!("minos_teamwork")
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["threadId"],
            json!(started.session_id)
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["turnId"],
            json!(turn_id)
        );

        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tool_request_user_input_requests_are_forwarded_and_can_be_answered() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-tool-input";
        let turn_id = "turn-tool-input";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(session_id),
            },
            Step::EmitServerRequest {
                method: "item/tool/requestUserInput".into(),
                params: json!({
                    "itemId": "item-1",
                    "questions": [{
                        "header": "Need input",
                        "id": "q1",
                        "question": "Pick one"
                    }],
                    "threadId": session_id,
                    "turnId": turn_id,
                }),
            },
            Step::ExpectResponse {
                result: json!({ "answers": { "q1": { "answers": ["blue"] } } }),
            },
            Step::Sleep { ms: 20 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        mgr.start_agent(AgentKind::Codex, "/w-tool-input".into())
            .await
            .unwrap();

        let ingest = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest
                    .json_value()
                    .expect("raw ingest should contain JSON payload")
                    .get("method")
                    .and_then(Value::as_str)
                    == Some("approval/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("tool request user input synthetic ingest should arrive");

        assert_eq!(ingest.session_id, session_id);
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["method"],
            json!("item/tool/requestUserInput")
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["session_id"],
            json!(session_id)
        );
        assert_eq!(
            ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")["params"]["turn_id"],
            json!(turn_id)
        );
        let request_id = ingest
            .json_value()
            .expect("raw ingest should contain JSON payload")["params"]["request_id"]
            .as_str()
            .expect("request id should be present")
            .to_string();

        mgr.resolve_approval(
            &request_id,
            session_id,
            json!({ "answers": { "q1": { "answers": ["blue"] } } }),
        )
        .await
        .unwrap();

        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn approval_requests_do_not_auto_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-approval-no-timeout";
        let turn_id = "turn-approval-no-timeout";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(session_id),
            },
            Step::EmitServerRequest {
                method: "item/commandExecution/requestApproval".into(),
                params: command_approval_params(session_id, turn_id),
            },
            // Stay open long enough for a short wait; no timeout reply expected.
            Step::Sleep { ms: 200 },
            Step::ExpectResponse {
                result: json!({ "decision": "decline" }),
            },
            Step::Sleep { ms: 20 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        // Host must not auto-cancel approvals.
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        mgr.start_agent(AgentKind::Codex, "/w-approval-no-timeout".into())
            .await
            .unwrap();

        let request = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest
                    .json_value()
                    .expect("raw ingest should contain JSON payload")
                    .get("method")
                    .and_then(Value::as_str)
                    == Some("approval/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("approval/request ingest should arrive");

        let request_id = request
            .json_value()
            .expect("raw ingest should contain JSON payload")["params"]["request_id"]
            .as_str()
            .expect("request should carry request_id")
            .to_string();
        assert!(mgr.pending_approvals.contains_key(&request_id));

        // Wait longer than the old default auto-timeout would have been.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            mgr.pending_approvals.contains_key(&request_id),
            "pending approval must stay open until the user decides"
        );

        // Explicit reject still works.
        mgr.resolve_approval(&request_id, session_id, json!({ "decision": "decline" }))
            .await
            .unwrap();
        assert!(!mgr.pending_approvals.contains_key(&request_id));

        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_approval_replies_to_codex_and_clears_pending_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = "thr-approval-decision";
        let turn_id = "turn-approval-decision";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(session_id),
            },
            Step::EmitServerRequest {
                method: "item/commandExecution/requestApproval".into(),
                params: command_approval_params(session_id, turn_id),
            },
            Step::ExpectResponse {
                result: json!({ "decision": "decline" }),
            },
            Step::Sleep { ms: 20 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        mgr.start_agent(AgentKind::Codex, "/w-approval-decision".into())
            .await
            .unwrap();

        let approval_request = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest
                    .json_value()
                    .expect("raw ingest should contain JSON payload")
                    .get("method")
                    .and_then(Value::as_str)
                    == Some("approval/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("approval/request ingest should arrive");
        let request_id = approval_request
            .json_value()
            .expect("raw ingest should contain JSON payload")["params"]["request_id"]
            .as_str()
            .expect("approval/request ingest should carry request_id")
            .to_string();

        mgr.resolve_approval(&request_id, session_id, json!({ "decision": "decline" }))
            .await
            .unwrap();
        assert!(!mgr.pending_approvals.contains_key(&request_id));

        server.stop().await;
    }
}
