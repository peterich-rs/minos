// Module-local allow for the two `kill(2)` group-signalling calls in
// `shutdown_instances`. The crate-level `deny(unsafe_code)` keeps everything
// else honest.
#![allow(unsafe_code)]

use crate::codex_client::{CodexClient, Inbound};
use crate::instance::AppServerInstance;
use crate::manager_event::ManagerEvent;
use crate::process::CodexProcess;
use crate::state_machine::{PauseReason, ThreadState};
use crate::thread_handle::ThreadHandle;
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
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch, Mutex};
use tracing::{info, warn};
use url::Url;

#[derive(Clone, Debug)]
pub struct InstanceCaps {
    pub max_instances: usize,
    pub idle_timeout: std::time::Duration,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingApproval {
    pub thread_id: String,
    pub codex_request_id: Value,
    pub request: ServerRequest,
    pub client: Arc<CodexClient>,
    pub created_at: Instant,
}

pub(crate) type PendingApprovals = Arc<DashMap<String, PendingApproval>>;

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
    pub(crate) instances: Arc<Mutex<HashMap<PathBuf, Arc<AppServerInstance>>>>,
    pub(crate) threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
    pub(crate) pending_approvals: PendingApprovals,
    pub(crate) events_tx: broadcast::Sender<RawIngest>,
    pub(crate) manager_tx: broadcast::Sender<ManagerEvent>,
    pub(crate) claude_sessions:
        Arc<Mutex<HashMap<String, crate::claude_driver::ClaudeNdjsonSession>>>,
    pub(crate) opencode_instances:
        Arc<Mutex<HashMap<PathBuf, Arc<Mutex<crate::opencode_driver::OpencodeServerInstance>>>>>,
    pub(crate) opencode_session_map: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) gemini_instances:
        Arc<Mutex<HashMap<String, Arc<crate::gemini_driver::GeminiAcpInstance>>>>,
}

impl AgentManager {
    pub fn new(config: AgentRuntimeConfig, caps: InstanceCaps) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        let (manager_tx, _) = broadcast::channel(64);
        let mgr = Self {
            config: Arc::new(config),
            caps,
            instances: Arc::new(Mutex::new(HashMap::new())),
            threads: Arc::new(Mutex::new(HashMap::new())),
            pending_approvals: Arc::new(DashMap::new()),
            events_tx,
            manager_tx,
            claude_sessions: Arc::new(Mutex::new(HashMap::new())),
            opencode_instances: Arc::new(Mutex::new(HashMap::new())),
            opencode_session_map: Arc::new(Mutex::new(HashMap::new())),
            gemini_instances: Arc::new(Mutex::new(HashMap::new())),
        };
        mgr.spawn_reaper();
        mgr
    }

    fn spawn_reaper(&self) {
        let caps = self.caps.clone();
        let instances = self.instances.clone();
        let threads = self.threads.clone();
        let manager_tx = self.manager_tx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_mins(1));
            loop {
                tick.tick().await;
                let mut to_reap: Vec<PathBuf> = Vec::new();
                {
                    let ig = instances.lock().await;
                    for (ws, inst) in ig.iter() {
                        let last = *inst.last_activity_at.lock().await;
                        let idle = last.elapsed() >= caps.idle_timeout;
                        let tids = inst.thread_ids().await;
                        let tg = threads.lock().await;
                        let any_running = tids.iter().any(|t| {
                            tg.get(t).is_some_and(|h| {
                                matches!(h.current_state(), ThreadState::Running { .. })
                            })
                        });
                        drop(tg);
                        if idle && !any_running {
                            to_reap.push(ws.clone());
                        }
                    }
                }
                for ws in to_reap {
                    Self::reap_static(&instances, &threads, &manager_tx, &ws).await;
                }
            }
        });
    }

    async fn reap_static(
        instances: &Arc<Mutex<HashMap<PathBuf, Arc<AppServerInstance>>>>,
        threads: &Arc<Mutex<HashMap<String, ThreadHandle>>>,
        manager_tx: &broadcast::Sender<ManagerEvent>,
        ws: &Path,
    ) {
        let Some(inst) = instances.lock().await.remove(ws) else {
            return;
        };
        let tids = inst.thread_ids().await;
        let workspace = inst.workspace.clone();
        let tg = threads.lock().await;
        for tid in &tids {
            if let Some(h) = tg.get(tid) {
                let _ = h.transition(ThreadState::Suspended {
                    reason: PauseReason::InstanceReaped,
                });
            }
        }
        drop(tg);
        let _ = manager_tx.send(ManagerEvent::InstanceCrashed {
            workspace,
            affected_threads: tids,
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

    pub fn manager_event_stream(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager_tx.subscribe()
    }

    pub async fn thread_state_stream(
        &self,
        thread_id: &str,
    ) -> Option<watch::Receiver<ThreadState>> {
        self.threads
            .lock()
            .await
            .get(thread_id)
            .map(|h| h.state_rx.clone())
    }

    pub async fn has_thread(&self, thread_id: &str) -> bool {
        self.threads.lock().await.contains_key(thread_id)
    }

    pub async fn thread_provider_session_id(&self, thread_id: &str) -> Option<String> {
        self.threads
            .lock()
            .await
            .get(thread_id)
            .and_then(|handle| handle.codex_session_id.clone())
    }

    pub async fn register_persisted_thread(
        &self,
        thread_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        codex_session_id: Option<String>,
        initial_state: ThreadState,
        last_seq: u64,
    ) -> anyhow::Result<()> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or(workspace);
        let mut threads = self.threads.lock().await;
        if threads.contains_key(&thread_id) {
            return Ok(());
        }
        let mut handle = ThreadHandle::new(
            thread_id.clone(),
            canon.clone(),
            agent,
            initial_state.clone(),
            last_seq,
        );
        handle.codex_session_id = codex_session_id;
        threads.insert(thread_id.clone(), handle);
        drop(threads);
        let _ = self.manager_tx.send(ManagerEvent::ThreadAdded {
            thread_id,
            workspace: canon,
            agent,
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
        match session_id {
            None => {
                let outcome = self
                    .start_agent_with_policies(agent, workspace, policies)
                    .await?;
                self.send_user_message(&outcome.thread_id, text).await?;
                Ok(DispatchOutcome {
                    session_id: outcome.thread_id,
                    cwd: outcome.cwd,
                    provider_session_id: outcome.provider_session_id,
                })
            }
            Some(session_id) => {
                let handle = self
                    .threads
                    .lock()
                    .await
                    .get(&session_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("thread not found: {session_id}"))?;
                match handle.current_state() {
                    ThreadState::Idle => self.send_user_message(&session_id, text).await?,
                    ThreadState::Running { .. } => {
                        self.send_user_message(&session_id, text).await?
                    }
                    ThreadState::Suspended { .. } => {
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
        self.start_agent_with_policies(agent, workspace, None).await
    }

    pub async fn start_agent_with_policies(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        policies: Option<SessionPolicies>,
    ) -> anyhow::Result<StartAgentOutcome> {
        match agent {
            AgentName::Codex => self.start_codex_agent(agent, workspace, policies).await,
            AgentName::Claude => self.start_claude_agent(agent, workspace).await,
            AgentName::Opencode => self.start_opencode_agent(agent, workspace).await,
            AgentName::Gemini => self.start_gemini_agent(agent, workspace).await,
        }
    }

    async fn start_codex_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
        policies: Option<SessionPolicies>,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let instance = self.ensure_instance(&canon, policies.as_ref()).await?;

        // Allocate a fresh thread on the codex app-server. The
        // `thread/started` notification arrives later via the event pump and
        // populates `codex_session_id` + flips state Starting -> Idle.
        let resp = instance.start_thread(&canon).await?;
        let thread_id = resp.thread_id.clone();
        instance.add_thread(thread_id.clone()).await;
        instance.touch().await;

        let handle = ThreadHandle::new(
            thread_id.clone(),
            canon.clone(),
            agent,
            ThreadState::Starting,
            0,
        );
        self.threads.lock().await.insert(thread_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::ThreadAdded {
            thread_id: thread_id.clone(),
            workspace: canon.clone(),
            agent,
        });

        // The event pump will surface the `thread/started` notification; in
        // the absence of an explicit notification we still flip to Idle so
        // callers can dispatch turns. Real codex emits the notification before
        // returning the response, so by the time we get here the pump has
        // already advanced the state if it was going to. To match the codex
        // app-server contract documented in spec §6.1, mark the thread Idle
        // synchronously once the response carries `thread.id`.
        if let Some(handle) = self.threads.lock().await.get_mut(&thread_id) {
            handle.codex_session_id = Some(resp.codex_session_id.clone());
            let _ = handle.transition(ThreadState::Idle);
        }
        let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.clone(),
            old: ThreadState::Starting,
            new: ThreadState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });

        Ok(StartAgentOutcome {
            thread_id,
            cwd: canon,
            provider_session_id: Some(resp.codex_session_id),
        })
    }

    async fn start_claude_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let thread_id = uuid::Uuid::new_v4().to_string();
        let provider_session_id = uuid::Uuid::new_v4().to_string();
        let mut handle = ThreadHandle::new(
            thread_id.clone(),
            canon.clone(),
            agent,
            ThreadState::Starting,
            0,
        );
        handle.codex_session_id = Some(provider_session_id.clone());
        self.threads.lock().await.insert(thread_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::ThreadAdded {
            thread_id: thread_id.clone(),
            workspace: canon.clone(),
            agent,
        });
        if let Some(h) = self.threads.lock().await.get(&thread_id) {
            let _ = h.transition(ThreadState::Idle);
        }
        let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.clone(),
            old: ThreadState::Starting,
            new: ThreadState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        Ok(StartAgentOutcome {
            thread_id,
            cwd: canon,
            provider_session_id: Some(provider_session_id),
        })
    }

    async fn start_opencode_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let thread_id = uuid::Uuid::new_v4().to_string();
        let instance = self.ensure_opencode_instance(&canon).await?;
        let oc_session_id = instance.lock().await.create_session().await?;
        self.opencode_session_map
            .lock()
            .await
            .insert(thread_id.clone(), oc_session_id.clone());
        let mut handle = ThreadHandle::new(
            thread_id.clone(),
            canon.clone(),
            agent,
            ThreadState::Idle,
            0,
        );
        handle.codex_session_id = Some(oc_session_id.clone());
        self.threads.lock().await.insert(thread_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::ThreadAdded {
            thread_id: thread_id.clone(),
            workspace: canon.clone(),
            agent,
        });
        Ok(StartAgentOutcome {
            thread_id,
            cwd: canon,
            provider_session_id: Some(oc_session_id),
        })
    }

    async fn start_gemini_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let thread_id = uuid::Uuid::new_v4().to_string();
        let provider_session_id = self
            .ensure_gemini_instance_for_thread(&thread_id, &canon, None)
            .await?;
        let mut handle = ThreadHandle::new(
            thread_id.clone(),
            canon.clone(),
            agent,
            ThreadState::Idle,
            0,
        );
        handle.codex_session_id = Some(provider_session_id.clone());
        self.threads.lock().await.insert(thread_id.clone(), handle);
        let _ = self.manager_tx.send(ManagerEvent::ThreadAdded {
            thread_id: thread_id.clone(),
            workspace: canon.clone(),
            agent,
        });
        Ok(StartAgentOutcome {
            thread_id,
            cwd: canon,
            provider_session_id: Some(provider_session_id),
        })
    }

    async fn ensure_gemini_instance_for_thread(
        &self,
        thread_id: &str,
        workspace: &Path,
        resume_session_id: Option<&str>,
    ) -> anyhow::Result<String> {
        if let Some(existing) = self.gemini_instances.lock().await.get(thread_id).cloned() {
            return existing.get_session_id().await.ok_or_else(|| {
                anyhow::anyhow!("gemini ACP instance has no active session: {thread_id}")
            });
        }

        let bin_path = self
            .config
            .gemini_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(AgentName::Gemini.bin_name()));
        let (crash_tx, _crash_rx) = tokio::sync::mpsc::channel::<()>(1);
        let instance = crate::gemini_driver::GeminiAcpInstance::spawn(
            &bin_path,
            workspace,
            &self.config.subprocess_env,
            crash_tx,
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
        if let Some(session_id) = resume_session_id {
            match instance.resume_session(session_id, workspace).await {
                Ok(_) => {
                    resumed = true;
                    provider_session_id = Some(session_id.to_string());
                }
                Err(error) => {
                    warn!(
                        target: "minos_agent_runtime::manager",
                        thread_id,
                        session_id,
                        error = %error,
                        "gemini ACP session/resume failed; starting a fresh session",
                    );
                }
            }
        }
        if !resumed {
            let response = instance
                .new_session(workspace)
                .await
                .map_err(|error| anyhow::anyhow!("gemini ACP session/new failed: {error}"))?;
            provider_session_id = Some(response.session_id);
        }
        let provider_session_id = provider_session_id
            .ok_or_else(|| anyhow::anyhow!("gemini ACP session setup did not return session id"))?;
        crate::gemini_driver::spawn_acp_pump(
            instance.client.clone(),
            thread_id.to_string(),
            self.events_tx.clone(),
        );
        self.gemini_instances
            .lock()
            .await
            .insert(thread_id.to_string(), instance);
        Ok(provider_session_id)
    }

    async fn ensure_opencode_instance(
        &self,
        workspace: &Path,
    ) -> anyhow::Result<Arc<Mutex<crate::opencode_driver::OpencodeServerInstance>>> {
        let mut map = self.opencode_instances.lock().await;
        if let Some(existing) = map.get(workspace) {
            return Ok(existing.clone());
        }
        let bin = self
            .config
            .opencode_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(AgentName::Opencode.bin_name()));
        let port = pick_free_port(self.config.opencode_port_range.clone())?;
        let password = uuid::Uuid::new_v4().to_string();
        let config = crate::opencode_driver::OpencodeServerConfig {
            opencode_bin: bin,
            port,
            password,
            subprocess_env: self.config.subprocess_env.clone(),
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
            self.threads.clone(),
            self.manager_tx.clone(),
            self.events_tx.clone(),
        );
        map.insert(workspace.to_path_buf(), instance.clone());
        Ok(instance)
    }

    async fn ensure_instance(
        &self,
        workspace: &Path,
        policies: Option<&SessionPolicies>,
    ) -> anyhow::Result<Arc<AppServerInstance>> {
        let mut guard = self.instances.lock().await;
        if let Some(existing) = guard.get(workspace) {
            return Ok(existing.clone());
        }
        if guard.len() >= self.caps.max_instances {
            self.lru_evict(&mut guard).await?;
        }
        let inst = self.spawn_instance(workspace, policies).await?;
        guard.insert(workspace.to_path_buf(), inst.clone());
        Ok(inst)
    }

    #[allow(clippy::too_many_lines)]
    async fn spawn_instance(
        &self,
        workspace: &Path,
        policies: Option<&SessionPolicies>,
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
            let inst = build_fake_instance(workspace_buf.clone(), client, crash_tx);
            let pump_client = inst.client.clone();
            let pump_events = self.events_tx.clone();
            let pump_threads = self.threads.clone();
            let pump_workspace = workspace_buf.clone();
            let pump_crash = inst.crash_signal.clone();
            tokio::spawn(event_pump_loop(
                pump_client,
                pump_events,
                pump_threads,
                self.pending_approvals.clone(),
                self.manager_tx.clone(),
                pump_workspace,
                self.config.approval_request_timeout,
                pump_crash,
            ));

            let watcher_inst = inst.clone();
            let watcher_threads = self.threads.clone();
            let watcher_mgr_tx = self.manager_tx.clone();
            tokio::spawn(async move {
                let _ = crash_rx.recv().await;
                let affected = watcher_inst.thread_ids().await;
                let tg = watcher_threads.lock().await;
                for tid in &affected {
                    if let Some(h) = tg.get(tid) {
                        let _ = h.transition(ThreadState::Suspended {
                            reason: PauseReason::CodexCrashed,
                        });
                    }
                }
                drop(tg);
                let _ = watcher_mgr_tx.send(ManagerEvent::InstanceCrashed {
                    workspace: watcher_inst.workspace.clone(),
                    affected_threads: affected,
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
        let args = build_codex_spawn_args(&listen_arg, &workspace_display, &spawn_policies);
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
            crash_tx.clone(),
        ));

        // Spawn the event pump. It owns the client handle for inbound reads
        // and forwards every notification verbatim into the manager's
        // `events_tx` broadcast.
        let pump_client = client.clone();
        let pump_events = self.events_tx.clone();
        let pump_threads = self.threads.clone();
        let pump_workspace = workspace_buf.clone();
        let pump_crash = crash_tx.clone();
        tokio::spawn(event_pump_loop(
            pump_client,
            pump_events,
            pump_threads,
            self.pending_approvals.clone(),
            self.manager_tx.clone(),
            pump_workspace,
            self.config.approval_request_timeout,
            pump_crash,
        ));

        // Spawn the crash watcher. When the codex child exits or the WS pump
        // signals end-of-stream, we mark all threads on this instance as
        // Suspended { CodexCrashed } and broadcast InstanceCrashed.
        let watcher_inst = inst.clone();
        let watcher_threads = self.threads.clone();
        let watcher_mgr_tx = self.manager_tx.clone();
        tokio::spawn(async move {
            let _ = crash_rx.recv().await;
            let affected = watcher_inst.thread_ids().await;
            let tg = watcher_threads.lock().await;
            for tid in &affected {
                if let Some(h) = tg.get(tid) {
                    let _ = h.transition(ThreadState::Suspended {
                        reason: PauseReason::CodexCrashed,
                    });
                }
            }
            drop(tg);
            let _ = watcher_mgr_tx.send(ManagerEvent::InstanceCrashed {
                workspace: watcher_inst.workspace.clone(),
                affected_threads: affected,
            });
        });

        Ok(inst)
    }

    async fn lru_evict(
        &self,
        map: &mut HashMap<PathBuf, Arc<AppServerInstance>>,
    ) -> anyhow::Result<()> {
        let mut candidates: Vec<(PathBuf, std::time::Instant)> = Vec::new();
        let tg = self.threads.lock().await;
        for (ws, inst) in map.iter() {
            let tids = inst.thread_ids().await;
            let any_running = tids.iter().any(|t| {
                tg.get(t)
                    .is_some_and(|h| matches!(h.current_state(), ThreadState::Running { .. }))
            });
            if !any_running {
                candidates.push((ws.clone(), *inst.last_activity_at.lock().await));
            }
        }
        drop(tg);
        candidates.sort_by_key(|(_, t)| *t);
        let victim = candidates.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("TooManyInstances: every instance has a Running thread")
        })?;
        let inst = map.remove(&victim.0).expect("victim was in map");
        let tids = inst.thread_ids().await;
        let workspace = inst.workspace.clone();
        let tg = self.threads.lock().await;
        for tid in &tids {
            if let Some(h) = tg.get(tid) {
                let _ = h.transition(ThreadState::Suspended {
                    reason: PauseReason::InstanceReaped,
                });
            }
        }
        drop(tg);
        let _ = self.manager_tx.send(ManagerEvent::InstanceCrashed {
            workspace,
            affected_threads: tids,
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
        self.instances.lock().await.keys().cloned().collect()
    }

    /// Test-only count of currently tracked threads.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn thread_count(&self) -> usize {
        self.threads.lock().await.len()
    }

    /// Test-only state snapshot for a single thread.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn thread_state(&self, thread_id: &str) -> Option<ThreadState> {
        self.threads
            .lock()
            .await
            .get(thread_id)
            .map(ThreadHandle::current_state)
    }

    /// Test-only helper: run one pass of the reaper synchronously. Production
    /// code spawns the periodic loop in [`AgentManager::spawn_reaper`].
    #[doc(hidden)]
    pub async fn tick_reaper_once(&self) {
        let mut to_reap: Vec<PathBuf> = Vec::new();
        {
            let ig = self.instances.lock().await;
            for (ws, inst) in ig.iter() {
                let last = *inst.last_activity_at.lock().await;
                let idle = last.elapsed() >= self.caps.idle_timeout;
                let tids = inst.thread_ids().await;
                let tg = self.threads.lock().await;
                let any_running = tids.iter().any(|t| {
                    tg.get(t)
                        .is_some_and(|h| matches!(h.current_state(), ThreadState::Running { .. }))
                });
                drop(tg);
                if idle && !any_running {
                    to_reap.push(ws.clone());
                }
            }
        }
        for ws in to_reap {
            self.reap_instance(&ws).await;
        }
    }

    async fn reap_instance(&self, ws: &Path) {
        Self::reap_static(&self.instances, &self.threads, &self.manager_tx, ws).await;
    }

    pub async fn send_user_message(&self, thread_id: &str, text: String) -> anyhow::Result<()> {
        let handle = self
            .threads
            .lock()
            .await
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        match handle.current_state() {
            ThreadState::Idle => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let new_state = ThreadState::Running {
                    turn_started_at_ms: now_ms,
                };
                handle.transition(new_state.clone())?;
                let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
                    thread_id: thread_id.to_string(),
                    old: ThreadState::Idle,
                    new: new_state,
                    at_ms: now_ms,
                });
                self.synth_user_message_ingest(thread_id, &text, handle.agent);
                match handle.agent {
                    AgentName::Codex => {
                        let workspace = handle.workspace.clone();
                        let inst = self
                            .instances
                            .lock()
                            .await
                            .get(&workspace)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("instance for workspace gone"))?;
                        inst.touch().await;
                        let turn_id = inst.send_user_message(thread_id, &text).await?;
                        handle.set_active_turn_id_if_absent(turn_id);
                    }
                    AgentName::Claude => {
                        self.start_claude_turn(thread_id, &text, &handle).await?;
                    }
                    AgentName::Opencode => {
                        let oc_session_id = self
                            .ensure_opencode_session_for_thread(thread_id, &handle)
                            .await?;
                        let workspace = handle.workspace.clone();
                        let instance = self
                            .opencode_instances
                            .lock()
                            .await
                            .get(&workspace)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("opencode instance not found"))?;
                        instance
                            .lock()
                            .await
                            .send_prompt(&oc_session_id, &text)
                            .await?;
                    }
                    AgentName::Gemini => {
                        self.spawn_gemini_prompt_task(thread_id.to_string(), text, handle.clone())
                            .await?;
                    }
                }
                Ok(())
            }
            ThreadState::Running { .. } => match handle.agent {
                AgentName::Codex => self.steer_turn(thread_id, text).await,
                AgentName::Opencode => self.send_opencode_prompt(thread_id, &text, &handle).await,
                AgentName::Claude => self.send_claude_prompt(thread_id, &text, &handle).await,
                AgentName::Gemini => anyhow::bail!("gemini turn is already running"),
            },
            ThreadState::Suspended { .. } => {
                match handle.agent {
                    AgentName::Codex => return self.implicit_resume(thread_id, text).await,
                    AgentName::Claude => {
                        self.resume_claude_thread(thread_id, &text, &handle).await?
                    }
                    AgentName::Opencode => {
                        self.resume_opencode_thread(thread_id, &handle).await?;
                    }
                    AgentName::Gemini => self.resume_gemini_thread(thread_id, &handle).await?,
                }
                let now_ms = chrono::Utc::now().timestamp_millis();
                let new_state = ThreadState::Running {
                    turn_started_at_ms: now_ms,
                };
                handle.transition(new_state.clone())?;
                let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
                    thread_id: thread_id.to_string(),
                    old: ThreadState::Idle,
                    new: new_state,
                    at_ms: now_ms,
                });
                self.synth_user_message_ingest(thread_id, &text, handle.agent);
                match handle.agent {
                    AgentName::Claude => self.start_claude_turn(thread_id, &text, &handle).await,
                    AgentName::Opencode => {
                        let oc_session_id = self
                            .ensure_opencode_session_for_thread(thread_id, &handle)
                            .await?;
                        let workspace = handle.workspace.clone();
                        let instance = self
                            .opencode_instances
                            .lock()
                            .await
                            .get(&workspace)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("opencode instance not found"))?;
                        let result = instance
                            .lock()
                            .await
                            .send_prompt(&oc_session_id, &text)
                            .await;
                        result
                    }
                    AgentName::Gemini => {
                        self.spawn_gemini_prompt_task(thread_id.to_string(), text, handle)
                            .await
                    }
                    AgentName::Codex => unreachable!("codex suspended branch returns above"),
                }
            }
            other => anyhow::bail!("send_user_message rejected: state={other:?}"),
        }
    }

    async fn start_claude_turn(
        &self,
        thread_id: &str,
        text: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<()> {
        let cli_path = PathBuf::from(AgentName::Claude.bin_name());
        let provider_session_id = match provider_resume_session_id(thread_id, handle) {
            Some(session_id) => session_id.to_string(),
            None => {
                let session_id = uuid::Uuid::new_v4().to_string();
                self.set_thread_provider_session_id(thread_id, session_id.clone())
                    .await;
                session_id
            }
        };
        let has_runtime_session = self.claude_sessions.lock().await.contains_key(thread_id);
        let has_persisted_history = handle.last_seq.load(std::sync::atomic::Ordering::SeqCst) > 0;
        let resume_sid =
            (has_runtime_session || has_persisted_history).then_some(provider_session_id.as_str());
        let session_id = resume_sid.is_none().then_some(provider_session_id.as_str());
        let session = crate::claude_driver::ClaudeNdjsonSession::start_turn(
            &cli_path,
            &handle.workspace,
            thread_id.to_string(),
            text,
            session_id,
            resume_sid,
            self.threads.clone(),
            self.manager_tx.clone(),
            self.events_tx.clone(),
            &self.config.subprocess_env,
        )
        .await?;
        self.claude_sessions
            .lock()
            .await
            .insert(thread_id.to_string(), session);
        Ok(())
    }

    async fn send_claude_prompt(
        &self,
        thread_id: &str,
        text: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<()> {
        self.synth_user_message_ingest(thread_id, text, handle.agent);
        self.start_claude_turn(thread_id, text, handle).await
    }

    async fn ensure_opencode_session_for_thread(
        &self,
        thread_id: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<String> {
        if let Some(existing) = self
            .opencode_session_map
            .lock()
            .await
            .get(thread_id)
            .cloned()
        {
            return Ok(existing);
        }

        let workspace = handle.workspace.clone();
        let instance = self.ensure_opencode_instance(&workspace).await?;
        let session_id = match provider_resume_session_id(thread_id, handle) {
            Some(session_id) => session_id.to_string(),
            None => instance.lock().await.create_session().await?,
        };
        self.opencode_session_map
            .lock()
            .await
            .insert(thread_id.to_string(), session_id.clone());
        self.set_thread_provider_session_id(thread_id, session_id.clone())
            .await;
        Ok(session_id)
    }

    async fn send_opencode_prompt(
        &self,
        thread_id: &str,
        text: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<()> {
        let oc_session_id = self
            .ensure_opencode_session_for_thread(thread_id, handle)
            .await?;
        let workspace = handle.workspace.clone();
        let instance = self
            .opencode_instances
            .lock()
            .await
            .get(&workspace)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("opencode instance not found"))?;
        self.synth_user_message_ingest(thread_id, text, handle.agent);
        let result = instance
            .lock()
            .await
            .send_prompt(&oc_session_id, text)
            .await;
        result
    }

    async fn set_thread_provider_session_id(&self, thread_id: &str, provider_session_id: String) {
        if let Some(handle) = self.threads.lock().await.get_mut(thread_id) {
            handle.codex_session_id = Some(provider_session_id);
        }
    }

    fn transition_resumed_thread_to_idle(
        &self,
        thread_id: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<()> {
        let old = handle.current_state();
        handle.transition(ThreadState::Resuming)?;
        let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.to_string(),
            old,
            new: ThreadState::Resuming,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        handle.transition(ThreadState::Idle)?;
        let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.to_string(),
            old: ThreadState::Resuming,
            new: ThreadState::Idle,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        Ok(())
    }

    async fn resume_claude_thread(
        &self,
        thread_id: &str,
        _text: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<()> {
        self.transition_resumed_thread_to_idle(thread_id, handle)
    }

    async fn resume_opencode_thread(
        &self,
        thread_id: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<()> {
        self.ensure_opencode_session_for_thread(thread_id, handle)
            .await?;
        self.transition_resumed_thread_to_idle(thread_id, handle)
    }

    async fn resume_gemini_thread(
        &self,
        thread_id: &str,
        handle: &ThreadHandle,
    ) -> anyhow::Result<()> {
        let provider_session_id = self
            .ensure_gemini_instance_for_thread(
                thread_id,
                &handle.workspace,
                provider_resume_session_id(thread_id, handle),
            )
            .await?;
        self.set_thread_provider_session_id(thread_id, provider_session_id)
            .await;
        self.transition_resumed_thread_to_idle(thread_id, handle)
    }

    async fn spawn_gemini_prompt_task(
        &self,
        thread_id: String,
        text: String,
        handle: ThreadHandle,
    ) -> anyhow::Result<()> {
        let instance = self
            .gemini_instances
            .lock()
            .await
            .get(&thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("gemini ACP instance not found: {thread_id}"))?;
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
            let _ = events_tx.send(RawIngest {
                agent: AgentName::Gemini,
                thread_id: thread_id.clone(),
                payload,
                ts_ms: current_unix_ms(),
            });
            mark_thread_idle_with_tx(&thread_id, &handle, &manager_tx);
        });
        Ok(())
    }

    pub async fn steer_turn(&self, thread_id: &str, text: String) -> anyhow::Result<()> {
        let handle = self
            .threads
            .lock()
            .await
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        if !matches!(handle.current_state(), ThreadState::Running { .. }) {
            let state = handle.current_state();
            anyhow::bail!("steer_turn rejected: state={state:?}");
        }
        let expected_turn_id = handle
            .active_turn_id()
            .ok_or_else(|| anyhow::anyhow!("steer_turn rejected: missing active turn id"))?;
        let workspace = handle.workspace.clone();
        let inst = self
            .instances
            .lock()
            .await
            .get(&workspace)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("instance for workspace gone"))?;
        inst.touch().await;
        self.synth_user_message_ingest(thread_id, &text, handle.agent);
        let turn_id = inst.steer_turn(thread_id, &expected_turn_id, &text).await?;
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
    fn synth_user_message_ingest(&self, thread_id: &str, text: &str, agent: AgentName) {
        let item_id = uuid::Uuid::new_v4().to_string();
        let payload = match agent {
            AgentName::Gemini => serde_json::json!({
                "kind": "user_message",
                "messageId": item_id,
                "text": text,
                "threadId": thread_id,
            }),
            _ => serde_json::json!({
                "method": "item/started",
                "params": {
                    "item": {
                        "type": "userMessage",
                        "id": item_id,
                        "content": [{"type": "text", "text": text}],
                    },
                    "threadId": thread_id,
                    "turnId": "",
                }
            }),
        };
        let ingest = RawIngest {
            agent,
            thread_id: thread_id.to_string(),
            payload,
            ts_ms: current_unix_ms(),
        };
        if let Err(e) = self.events_tx.send(ingest) {
            tracing::debug!(
                target: "minos_agent_runtime::manager",
                error = %e,
                thread_id,
                "synth_user_message_ingest broadcast failed (no subscribers)",
            );
        }
    }

    async fn implicit_resume(&self, thread_id: &str, text: String) -> anyhow::Result<()> {
        let handle = self
            .threads
            .lock()
            .await
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let from_state = handle.current_state();
        handle.transition(ThreadState::Resuming)?;
        let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.to_string(),
            old: from_state,
            new: ThreadState::Resuming,
            at_ms: chrono::Utc::now().timestamp_millis(),
        });
        let workspace = handle.workspace.clone();
        let codex_session_id = handle.codex_session_id.clone();

        let inst = self.ensure_instance(&workspace, None).await?;
        if let Some(sid) = codex_session_id {
            inst.add_thread(thread_id.to_string()).await;
            inst.start_thread_resume(thread_id, &sid).await?;
        } else {
            let _ = handle.transition(ThreadState::Closed {
                reason: crate::state_machine::CloseReason::TerminalError,
            });
            anyhow::bail!("resume failed: no codex_session_id");
        }
        handle.transition(ThreadState::Idle)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let new_state = ThreadState::Running {
            turn_started_at_ms: now_ms,
        };
        handle.transition(new_state.clone())?;
        let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.to_string(),
            old: ThreadState::Idle,
            new: new_state,
            at_ms: now_ms,
        });
        inst.touch().await;
        // Same synth-then-forward pattern as the Idle path; resume races
        // shouldn't change persistence semantics.
        self.synth_user_message_ingest(thread_id, &text, handle.agent);
        let turn_id = inst.send_user_message(thread_id, &text).await?;
        handle.set_active_turn_id_if_absent(turn_id);
        Ok(())
    }

    pub async fn interrupt_thread(&self, thread_id: &str) -> anyhow::Result<()> {
        let handle = self
            .threads
            .lock()
            .await
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        if !matches!(
            handle.current_state(),
            ThreadState::Running { .. } | ThreadState::Idle
        ) {
            let s = handle.current_state();
            anyhow::bail!("interrupt rejected: state={s:?}");
        }
        match handle.agent {
            AgentName::Codex => {
                let workspace = handle.workspace.clone();
                if let Some(inst) = self.instances.lock().await.get(&workspace).cloned() {
                    let _ = inst.interrupt_turn(thread_id).await;
                }
            }
            AgentName::Claude => {
                if let Some(session) = self.claude_sessions.lock().await.get_mut(thread_id) {
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
                    .get(thread_id)
                    .cloned();
                if let Some(oc_sid) = oc_session_id {
                    let workspace = handle.workspace.clone();
                    let instance = self
                        .opencode_instances
                        .lock()
                        .await
                        .get(&workspace)
                        .cloned();
                    if let Some(inst) = instance {
                        let _ = inst.lock().await.abort_session(&oc_sid).await;
                    }
                }
            }
            AgentName::Gemini => {
                if let Some(instance) = self.gemini_instances.lock().await.get(thread_id).cloned() {
                    let _ = instance.cancel().await;
                }
            }
        }
        let from_state = handle.current_state();
        handle.set_active_turn_id(None);
        handle.transition(ThreadState::Suspended {
            reason: PauseReason::UserInterrupt,
        })?;
        let _ = self.manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.to_string(),
            old: from_state,
            new: ThreadState::Suspended {
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
        let inst = self.ensure_instance(&canon, None).await?;
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
        let inst = self.ensure_instance(&canon, None).await?;
        inst.touch().await;
        inst.write_host_skill_config(&path, enabled).await
    }

    pub async fn close_thread(&self, thread_id: &str) -> anyhow::Result<()> {
        let handle = self
            .threads
            .lock()
            .await
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        if matches!(handle.current_state(), ThreadState::Closed { .. }) {
            return Ok(());
        }
        handle.transition(ThreadState::Closed {
            reason: crate::state_machine::CloseReason::UserClose,
        })?;
        let workspace = handle.workspace.clone();
        match handle.agent {
            AgentName::Codex => {
                if let Some(inst) = self.instances.lock().await.get(&workspace).cloned() {
                    inst.remove_thread(thread_id).await;
                }
            }
            AgentName::Claude => {
                if let Some(session) = self.claude_sessions.lock().await.remove(thread_id) {
                    session.close(&self.events_tx).await;
                }
            }
            AgentName::Opencode => {
                self.opencode_session_map.lock().await.remove(thread_id);
            }
            AgentName::Gemini => {
                if let Some(instance) = self.gemini_instances.lock().await.remove(thread_id) {
                    let _ = instance.close_session().await;
                }
            }
        }
        let _ = self.manager_tx.send(ManagerEvent::ThreadClosed {
            thread_id: thread_id.to_string(),
            reason: crate::state_machine::CloseReason::UserClose,
        });
        Ok(())
    }

    pub async fn resolve_approval(
        &self,
        request_id: &str,
        thread_id: &str,
        decision: Value,
    ) -> anyhow::Result<()> {
        let Some(pending) = self
            .pending_approvals
            .get(request_id)
            .map(|entry| entry.value().clone())
        else {
            return Ok(());
        };

        if pending.thread_id != thread_id {
            anyhow::bail!(
                "approval request thread mismatch: expected {}, got {thread_id}",
                pending.thread_id,
            );
        }

        let reply = crate::approvals::validate_decision(&pending.request, &decision)?;
        let Some((_, pending)) = self.pending_approvals.remove(request_id) else {
            return Ok(());
        };
        pending
            .client
            .reply(pending.codex_request_id, reply)
            .await
            .map_err(|error| anyhow::anyhow!("approval reply failed: {error}"))
    }

    /// Shut every codex instance down with a polite SIGTERM to its process
    /// group, wait `grace` for them to exit, and then escalate to a
    /// group-wide SIGKILL. Drops every instance from the map. Used by
    /// [`crate::manager::AgentManager`] callers (the daemon shutdown path
    /// in C20).
    ///
    /// `process.rs` puts each codex child in its own process group via
    /// `setpgid(0, 0)` in `pre_exec`, which is what makes the
    /// `kill(-pgid, sig)` call below propagate to whatever shell helpers /
    /// model-invocation subprocesses codex itself forked. Without that
    /// signalling-by-group, only codex's main pid was reaped and its
    /// subprocesses were reparented to launchd on macOS, surviving
    /// `daemon.stop()`.
    pub async fn shutdown_instances(&self, grace: std::time::Duration) {
        let mut g = self.instances.lock().await;

        // Snapshot every group leader pid up front so the signalling phase
        // can release the instances lock before sleeping, and so we still
        // know which groups to kill if `inst.child` was somehow drained
        // between phases (defence-in-depth).
        let mut pgids: Vec<i32> = Vec::with_capacity(g.len());
        for inst in g.values() {
            if let Some(child) = inst.child.lock().await.as_ref() {
                if let Some(pid) = child.id() {
                    if let Ok(pid_i32) = i32::try_from(pid) {
                        pgids.push(pid_i32);
                    }
                }
            }
        }

        // Phase 1: polite SIGTERM to each codex process group. The negative
        // pid argument is the POSIX convention for "signal the group whose
        // leader has this pid" — we set the leader = the codex pid in
        // `process.rs::spawn`.
        #[cfg(unix)]
        for &pgid in &pgids {
            // SAFETY: kill(2) is async-signal-safe and re-entrant; passing a
            // negative pid is the documented "signal the group" form. The
            // worst case is errno = ESRCH when the group is already gone,
            // which we intentionally ignore via `let _`.
            let _ = unsafe { libc::kill(-pgid, libc::SIGTERM) };
        }

        tokio::time::sleep(grace).await;

        // Phase 2: SIGKILL the same groups as a backstop for any straggler
        // subprocess that ignored SIGTERM. The wait below then reaps the
        // codex leader itself.
        #[cfg(unix)]
        for &pgid in &pgids {
            // SAFETY: same as the SIGTERM call above — negative-pid kill(2)
            // is the documented group-signal form.
            let _ = unsafe { libc::kill(-pgid, libc::SIGKILL) };
        }

        for (_, inst) in std::mem::take(&mut *g) {
            let child_opt = inst.child.lock().await.take();
            drop(inst);
            if let Some(mut child) = child_opt {
                // `kill().await` sends SIGKILL to the leader and awaits its
                // exit (reaping any zombie). Kept as belt-and-braces for the
                // non-Unix path where we did not signal by group above.
                let _ = child.kill().await;
            }
        }
    }

    pub async fn list_threads(&self) -> Vec<crate::store_facing::ThreadSnapshot> {
        let g = self.threads.lock().await;
        g.values()
            .map(|h| crate::store_facing::ThreadSnapshot {
                thread_id: h.thread_id.clone(),
                workspace: h.workspace.clone(),
                state: h.current_state(),
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct StartAgentOutcome {
    pub thread_id: String,
    pub cwd: PathBuf,
    pub provider_session_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionPolicies {
    pub approval_policy: Option<String>,
    pub sandbox_policy: Option<String>,
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
        threads: Mutex::new(HashSet::new()),
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

fn current_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn provider_resume_session_id<'a>(thread_id: &str, handle: &'a ThreadHandle) -> Option<&'a str> {
    handle
        .codex_session_id
        .as_deref()
        .filter(|session_id| *session_id != thread_id)
}

fn mark_thread_idle_with_tx(
    thread_id: &str,
    handle: &ThreadHandle,
    manager_tx: &broadcast::Sender<ManagerEvent>,
) {
    let old = handle.current_state();
    if !matches!(old, ThreadState::Running { .. }) {
        return;
    }
    if handle.transition(ThreadState::Idle).is_ok() {
        let _ = manager_tx.send(ManagerEvent::ThreadStateChanged {
            thread_id: thread_id.to_string(),
            old,
            new: ThreadState::Idle,
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

fn build_codex_spawn_args(
    listen_arg: &str,
    workspace_display: &str,
    policies: &ResolvedSessionPolicies,
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
    args
}

fn jsonrpc_id_key(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn request_thread_id(params: &Value) -> Option<String> {
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

fn broadcast_ingest(events_tx: &broadcast::Sender<RawIngest>, ingest: RawIngest) {
    if let Err(error) = events_tx.send(ingest) {
        tracing::debug!(
            target: "minos_agent_runtime::manager",
            error = %error,
            "events_tx broadcast send failed (no subscribers)",
        );
    }
}

fn approval_request_ingest(
    agent: AgentName,
    thread_id: String,
    request_id: String,
    turn_id: String,
    method: String,
    params: Value,
    timeout: Duration,
) -> RawIngest {
    let payload_thread_id = thread_id.clone();
    RawIngest {
        agent,
        thread_id,
        payload: serde_json::json!({
            "method": "approval/request",
            "params": {
                "request_id": request_id,
                "thread_id": payload_thread_id,
                "turn_id": turn_id,
                "method": method,
                "params": params,
                "timeout_ms": u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            }
        }),
        ts_ms: current_unix_ms(),
    }
}

fn approval_timeout_ingest(agent: AgentName, thread_id: String, request_id: String) -> RawIngest {
    let payload_thread_id = thread_id.clone();
    RawIngest {
        agent,
        thread_id,
        payload: serde_json::json!({
            "method": "approval/timeout",
            "params": {
                "thread_id": payload_thread_id,
                "request_id": request_id,
                "reason": "timeout",
            }
        }),
        ts_ms: current_unix_ms(),
    }
}

fn spawn_approval_timeout(
    pending_approvals: PendingApprovals,
    events_tx: broadcast::Sender<RawIngest>,
    timeout: Duration,
    request_id: String,
    thread_id: String,
    agent: AgentName,
) {
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        let Some((_, pending)) = pending_approvals.remove(&request_id) else {
            return;
        };

        let elapsed_ms = pending.created_at.elapsed().as_millis();
        if let Some(reply) = crate::approvals::auto_reject(&pending.request) {
            if let Err(error) = pending.client.reply(pending.codex_request_id, reply).await {
                warn!(
                    target: "minos_agent_runtime::manager",
                    error = %error,
                    request_id,
                    thread_id,
                    elapsed_ms,
                    "approval timeout reply failed",
                );
            }
        } else {
            warn!(
                target: "minos_agent_runtime::manager",
                request_id,
                thread_id,
                elapsed_ms,
                "approval timeout fired for non-approval request",
            );
        }

        broadcast_ingest(
            &events_tx,
            approval_timeout_ingest(agent, thread_id, request_id),
        );
    });
}

/// Long-running event-pump task per instance: drains every inbound frame from
/// the codex WS and forwards `Notification` payloads as `RawIngest` records
/// keyed by the notification's `params.threadId`.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn event_pump_loop(
    client: Arc<CodexClient>,
    events_tx: broadcast::Sender<RawIngest>,
    threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
    pending_approvals: PendingApprovals,
    manager_tx: broadcast::Sender<ManagerEvent>,
    _workspace: PathBuf,
    approval_request_timeout: Duration,
    crash_tx: tokio::sync::mpsc::Sender<()>,
) {
    while let Some(inbound) = client.next_inbound().await {
        match inbound {
            Inbound::Notification { method, params } => {
                let thread_id = params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let Some(thread_id) = thread_id else {
                    continue;
                };
                if method == "turn/started" {
                    let turn_id = params
                        .get("turn")
                        .and_then(|turn| turn.get("id"))
                        .or_else(|| params.get("turnId"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(turn_id) = turn_id {
                        let tg = threads.lock().await;
                        if let Some(handle) = tg.get(&thread_id) {
                            handle.set_active_turn_id(Some(turn_id));
                        }
                    }
                }
                if method == "turn/completed" {
                    let maybe_transition = {
                        let tg = threads.lock().await;
                        tg.get(&thread_id).and_then(|handle| {
                            handle.set_active_turn_id(None);
                            let old = handle.current_state();
                            if matches!(old, ThreadState::Running { .. } | ThreadState::Resuming) {
                                handle.transition(ThreadState::Idle).ok()?;
                                Some((old, ThreadState::Idle))
                            } else {
                                None
                            }
                        })
                    };
                    if let Some((old, new)) = maybe_transition {
                        let _ = manager_tx.send(ManagerEvent::ThreadStateChanged {
                            thread_id: thread_id.clone(),
                            old,
                            new,
                            at_ms: current_unix_ms(),
                        });
                    }
                }
                // Look up agent kind for the thread; default to Codex if absent
                // (notifications can race the manager's bookkeeping).
                let agent = threads
                    .lock()
                    .await
                    .get(&thread_id)
                    .map_or(AgentName::Codex, |h| h.agent);
                let payload = serde_json::json!({ "method": method, "params": params });
                let ingest = RawIngest {
                    agent,
                    thread_id,
                    payload,
                    ts_ms: current_unix_ms(),
                };
                broadcast_ingest(&events_tx, ingest);
            }
            Inbound::ServerRequest { id, method, params } => {
                let envelope = serde_json::json!({ "method": method, "params": params });
                match serde_json::from_value::<minos_codex_protocol::ServerRequest>(envelope) {
                    Ok(req) if crate::approvals::is_approval_request(&req) => {
                        let Some(thread_id) = request_thread_id(&params) else {
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

                        let agent = threads
                            .lock()
                            .await
                            .get(&thread_id)
                            .map_or(AgentName::Codex, |h| h.agent);
                        let request_id = jsonrpc_id_key(&id);
                        let turn_id = request_turn_id(&params);

                        pending_approvals.insert(
                            request_id.clone(),
                            PendingApproval {
                                thread_id: thread_id.clone(),
                                codex_request_id: id.clone(),
                                request: req,
                                client: client.clone(),
                                created_at: Instant::now(),
                            },
                        );

                        broadcast_ingest(
                            &events_tx,
                            approval_request_ingest(
                                agent,
                                thread_id.clone(),
                                request_id.clone(),
                                turn_id,
                                method.clone(),
                                params.clone(),
                                approval_request_timeout,
                            ),
                        );
                        spawn_approval_timeout(
                            pending_approvals.clone(),
                            events_tx.clone(),
                            approval_request_timeout,
                            request_id,
                            thread_id,
                            agent,
                        );
                    }
                    Ok(_req) => {
                        warn!(
                            target: "minos_agent_runtime::manager",
                            method = %method,
                            "non-approval server request received; forwarding as synthetic notification",
                        );
                        if let Some(thread_id) = request_thread_id(&params) {
                            let agent = threads
                                .lock()
                                .await
                                .get(&thread_id)
                                .map_or(AgentName::Codex, |h| h.agent);
                            let synthetic_method = format!("server_request/{method}");
                            let payload = serde_json::json!({
                                "method": synthetic_method,
                                "params": params,
                            });
                            broadcast_ingest(
                                &events_tx,
                                RawIngest {
                                    agent,
                                    thread_id,
                                    payload,
                                    ts_ms: current_unix_ms(),
                                },
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
                        if let Some(thread_id) = request_thread_id(&params) {
                            let agent = threads
                                .lock()
                                .await
                                .get(&thread_id)
                                .map_or(AgentName::Codex, |h| h.agent);
                            let synthetic_method = format!("server_request/{method}");
                            let payload = serde_json::json!({
                                "method": synthetic_method,
                                "params": params,
                            });
                            broadcast_ingest(
                                &events_tx,
                                RawIngest {
                                    agent,
                                    thread_id,
                                    payload,
                                    ts_ms: current_unix_ms(),
                                },
                            );
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
/// `thread/start` JSON-RPC and returns the thread id (which doubles as the
/// codex session id for resume purposes per spec §6.1).
pub(crate) async fn rpc_start_thread(
    client: &CodexClient,
    cwd: &Path,
    timeout: Duration,
) -> anyhow::Result<StartThreadResult> {
    let cwd_str = cwd.display().to_string();
    let start_params = ThreadStartParams {
        cwd: Some(cwd_str),
        ..Default::default()
    };
    let resp: ThreadStartResponse = tokio::time::timeout(timeout, client.call_typed(start_params))
        .await
        .map_err(|_| anyhow::anyhow!("thread/start timeout"))?
        .map_err(|e| anyhow::anyhow!("thread/start failed: {e}"))?;
    let thread_id = resp.thread.id;
    Ok(StartThreadResult {
        codex_session_id: thread_id.clone(),
        thread_id,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct StartThreadResult {
    pub thread_id: String,
    pub codex_session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentRuntimeConfig;
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

    fn fake_thread_start_reply(thread_id: &str) -> serde_json::Value {
        json!({
            "approvalPolicy": "never",
            "approvalsReviewer": "user",
            "cwd": "/tmp",
            "instructionSources": [],
            "model": "fake",
            "modelProvider": "fake",
            "sandbox": { "type": "dangerFullAccess" },
            "thread": {
                "id": thread_id,
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

    fn command_approval_params(thread_id: &str, turn_id: &str) -> serde_json::Value {
        json!({
            "itemId": "item-1",
            "threadId": thread_id,
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

        let args = build_codex_spawn_args("ws://127.0.0.1:9999", "/tmp/ws", &resolved);
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
        let args = build_codex_spawn_args("ws://127.0.0.1:9999", "/tmp/ws", &resolved);

        assert_eq!(resolved, ResolvedSessionPolicies::default());
        assert!(!has_arg(&args, "approval_policy=on_request"));
        assert!(!has_arg(&args, "sandbox_policy=workspace_write"));
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
        let snap = mgr.list_threads().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].workspace, ws);
        assert!(matches!(
            mgr.thread_state(&resp.thread_id).await,
            Some(ThreadState::Idle)
        ));
        assert_eq!(
            mgr.open_workspaces().await,
            vec![std::path::PathBuf::from("/w-test")]
        );
        fake.stop().await;
    }

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
        let thread_id = started.thread_id.clone();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(&thread_id, "ping".into())
            .await
            .unwrap();

        let user = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("synthetic Gemini user message should arrive")
            .expect("ingest stream should stay open");
        assert_eq!(user.thread_id, thread_id);
        assert_eq!(
            user.payload.get("kind").and_then(Value::as_str),
            Some("user_message")
        );
        assert_eq!(
            user.payload.get("text").and_then(Value::as_str),
            Some("ping")
        );
        assert!(user
            .payload
            .get("messageId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty()));

        let chunk = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.thread_id == thread_id
                    && ingest.payload.get("kind").and_then(Value::as_str)
                        == Some("acp_notification")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("fake Gemini ACP notification should arrive");

        assert_eq!(
            chunk.payload["params"]["update"]["content"]["text"],
            "gemini says hi"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(mgr.thread_state(&thread_id).await, Some(ThreadState::Idle)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Gemini prompt task should return thread to idle");
        assert!(matches!(
            mgr.thread_state(&thread_id).await,
            Some(ThreadState::Idle)
        ));

        mgr.close_thread(&thread_id).await.unwrap();
    }

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
        let thread_id = "gemini-resume-thread";
        mgr.register_persisted_thread(
            thread_id.into(),
            tmp.path().to_path_buf(),
            AgentName::Gemini,
            Some("resume-session".into()),
            ThreadState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            4,
        )
        .await
        .unwrap();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(thread_id, "continue".into())
            .await
            .unwrap();

        let chunk = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.thread_id == thread_id
                    && ingest.payload.get("kind").and_then(Value::as_str)
                        == Some("acp_notification")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("resumed Gemini ACP notification should arrive");

        assert_eq!(
            chunk.payload["params"]["update"]["content"]["text"],
            "resumed gemini"
        );
        assert_eq!(
            mgr.thread_provider_session_id(thread_id).await.as_deref(),
            Some("resume-session")
        );
        mgr.close_thread(thread_id).await.unwrap();
    }

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
        mgr.send_user_message(&started.thread_id, "first claude".into())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.thread_id == started.thread_id
                    && ingest.payload.get("type").and_then(Value::as_str) == Some("result")
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
        let thread_id = "claude-resume-thread";
        mgr.register_persisted_thread(
            thread_id.into(),
            tmp.path().to_path_buf(),
            AgentName::Claude,
            Some(provider_session_id.into()),
            ThreadState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            9,
        )
        .await
        .unwrap();

        let mut rx = mgr.ingest_stream();
        mgr.send_user_message(thread_id, "continue claude".into())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = rx.recv().await.expect("ingest stream should stay open");
                if ingest.thread_id == thread_id
                    && ingest.payload.get("type").and_then(Value::as_str) == Some("result")
                {
                    break;
                }
            }
        })
        .await
        .expect("fake Claude result should arrive");

        let args = tokio::time::timeout(Duration::from_secs(2), async {
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
            mgr.thread_state(thread_id).await,
            Some(ThreadState::Idle)
        ));
    }

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
        let thread_id = "claude-running-thread";
        mgr.register_persisted_thread(
            thread_id.into(),
            tmp.path().to_path_buf(),
            AgentName::Claude,
            Some(provider_session_id.into()),
            ThreadState::Running {
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
                Some(thread_id.into()),
                "answer while running".into(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(outcome.session_id, thread_id);

        let args = tokio::time::timeout(Duration::from_secs(2), async {
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
                if ingest.thread_id == thread_id
                    && ingest.payload.get("method").and_then(Value::as_str) == Some("item/started")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("synthetic Claude user message should arrive");
        assert_eq!(
            user.payload["params"]["item"]["content"][0]["text"],
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
        let thread_id = "opencode-running-thread";
        mgr.register_persisted_thread(
            thread_id.into(),
            workspace.clone(),
            AgentName::Opencode,
            Some("sess_running".into()),
            ThreadState::Running {
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
            },
            child: None,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            base_url: format!("http://{addr}"),
            auth_header: "Basic test".into(),
        };
        mgr.opencode_instances
            .lock()
            .await
            .insert(workspace, Arc::new(Mutex::new(instance)));

        let mut rx = mgr.ingest_stream();
        let outcome = mgr
            .dispatch_message(
                AgentName::Opencode,
                "/unused".into(),
                Some(thread_id.into()),
                "running answer".into(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(outcome.session_id, thread_id);

        let request = request_rx.await.unwrap();
        assert!(request.starts_with("POST /session/sess_running/prompt_async "));
        assert!(request.contains(r#""text":"running answer""#));

        let user = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("synthetic opencode user message should arrive")
            .expect("ingest stream should stay open");
        assert_eq!(user.thread_id, thread_id);
        assert_eq!(
            user.payload.get("method").and_then(Value::as_str),
            Some("item/started")
        );
        assert_eq!(
            user.payload["params"]["item"]["content"][0]["text"],
            "running answer"
        );
    }

    #[tokio::test]
    async fn implicit_resume_from_suspended() {
        let tmp = tempfile::tempdir().unwrap();
        let (fake, url) = FakeCodexBackend::install().await;
        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(url);
        let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-resume".into())
            .await
            .unwrap();
        mgr.interrupt_thread(&started.thread_id).await.unwrap();
        assert!(matches!(
            mgr.thread_state(&started.thread_id).await,
            Some(ThreadState::Suspended {
                reason: PauseReason::UserInterrupt
            })
        ));

        mgr.send_user_message(&started.thread_id, "resume".into())
            .await
            .unwrap();
        assert!(matches!(
            mgr.thread_state(&started.thread_id).await,
            Some(ThreadState::Running { .. })
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

        mgr.send_user_message(&started.thread_id, "first".into())
            .await
            .unwrap();
        let first_turn_id = mgr
            .threads
            .lock()
            .await
            .get(&started.thread_id)
            .and_then(ThreadHandle::active_turn_id)
            .expect("turn/start should record an active turn id");

        mgr.send_user_message(&started.thread_id, "second".into())
            .await
            .unwrap();

        let second_turn_id = mgr
            .threads
            .lock()
            .await
            .get(&started.thread_id)
            .and_then(ThreadHandle::active_turn_id)
            .expect("turn/steer should preserve an active turn id");
        assert_eq!(second_turn_id, first_turn_id);
        assert!(matches!(
            mgr.thread_state(&started.thread_id).await,
            Some(ThreadState::Running { .. })
        ));

        fake.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::too_many_lines)]
    async fn turn_notifications_update_active_turn_id_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let thread_id = "thr-turn-lifecycle";
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
                        "id": thread_id,
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
                    "threadId": thread_id,
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
                    "threadId": thread_id,
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
        assert_eq!(started.thread_id, thread_id);

        let mut ingest_rx = mgr.ingest_stream();

        mgr.send_user_message(thread_id, "hello".into())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest broadcast should stay open");
                if ingest.thread_id == thread_id
                    && ingest
                        .payload
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
            .threads
            .lock()
            .await
            .get(thread_id)
            .and_then(ThreadHandle::active_turn_id);
        assert_eq!(turn_id.as_deref(), Some("turn-from-notification"));

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = mgr.thread_state(thread_id).await;
                let turn_id = mgr
                    .threads
                    .lock()
                    .await
                    .get(thread_id)
                    .and_then(ThreadHandle::active_turn_id);
                if matches!(state, Some(ThreadState::Idle)) && turn_id.is_none() {
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
            mgr.thread_state(&outcome.session_id).await,
            Some(ThreadState::Running { .. })
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
                Some(started.thread_id.clone()),
                "hello".into(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.session_id, started.thread_id);
        assert!(matches!(
            mgr.thread_state(&outcome.session_id).await,
            Some(ThreadState::Running { .. })
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
        mgr.send_user_message(&started.thread_id, "first".into())
            .await
            .unwrap();
        let first_turn_id = mgr
            .threads
            .lock()
            .await
            .get(&started.thread_id)
            .and_then(ThreadHandle::active_turn_id)
            .expect("turn/start should record turn id before steer");

        let outcome = mgr
            .dispatch_message(
                AgentKind::Codex,
                "/unused".into(),
                Some(started.thread_id.clone()),
                "second".into(),
                None,
            )
            .await
            .unwrap();

        let second_turn_id = mgr
            .threads
            .lock()
            .await
            .get(&started.thread_id)
            .and_then(ThreadHandle::active_turn_id)
            .expect("turn/steer should preserve turn id");
        assert_eq!(outcome.session_id, started.thread_id);
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
        mgr.interrupt_thread(&started.thread_id).await.unwrap();

        let outcome = mgr
            .dispatch_message(
                AgentKind::Codex,
                "/unused".into(),
                Some(started.thread_id.clone()),
                "resume".into(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.session_id, started.thread_id);
        assert!(matches!(
            mgr.thread_state(&outcome.session_id).await,
            Some(ThreadState::Running { .. })
        ));

        fake.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn approval_requests_are_forwarded_as_ingest_and_tracked() {
        let tmp = tempfile::tempdir().unwrap();
        let thread_id = "thr-approval-forward";
        let turn_id = "turn-approval-forward";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(thread_id),
            },
            Step::EmitServerRequest {
                method: "item/commandExecution/requestApproval".into(),
                params: command_approval_params(thread_id, turn_id),
            },
            Step::Sleep { ms: 100 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        cfg.approval_request_timeout = Duration::from_secs(5);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        let started = mgr
            .start_agent(AgentKind::Codex, "/w-approval-forward".into())
            .await
            .unwrap();
        assert_eq!(started.thread_id, thread_id);

        let ingest = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest.payload.get("method").and_then(Value::as_str) == Some("approval/request")
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
        assert_eq!(ingest.thread_id, thread_id);
        assert_eq!(ingest.payload["params"]["request_id"], json!(request_id));
        assert_eq!(ingest.payload["params"]["thread_id"], json!(thread_id));
        assert_eq!(ingest.payload["params"]["turn_id"], json!(turn_id));
        assert_eq!(
            ingest.payload["params"]["method"],
            json!("item/commandExecution/requestApproval")
        );
        assert!(mgr.pending_approvals.contains_key(&request_id));

        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn approval_requests_time_out_and_send_typed_reject() {
        let tmp = tempfile::tempdir().unwrap();
        let thread_id = "thr-approval-timeout";
        let turn_id = "turn-approval-timeout";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(thread_id),
            },
            Step::EmitServerRequest {
                method: "item/commandExecution/requestApproval".into(),
                params: command_approval_params(thread_id, turn_id),
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
        cfg.approval_request_timeout = Duration::from_millis(50);
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let mut ingest_rx = mgr.ingest_stream();

        mgr.start_agent(AgentKind::Codex, "/w-approval-timeout".into())
            .await
            .unwrap();

        let timed_out = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ingest = ingest_rx
                    .recv()
                    .await
                    .expect("ingest stream should stay open");
                if ingest.payload.get("method").and_then(Value::as_str) == Some("approval/timeout")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("approval/timeout ingest should arrive");

        let request_id = timed_out.payload["params"]["request_id"]
            .as_str()
            .expect("timeout ingest should carry request_id")
            .to_string();
        assert_eq!(timed_out.thread_id, thread_id);
        assert_eq!(timed_out.payload["params"]["reason"], json!("timeout"));
        assert!(!mgr.pending_approvals.contains_key(&request_id));

        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_approval_replies_to_codex_and_clears_pending_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let thread_id = "thr-approval-decision";
        let turn_id = "turn-approval-decision";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_start_reply(thread_id),
            },
            Step::EmitServerRequest {
                method: "item/commandExecution/requestApproval".into(),
                params: command_approval_params(thread_id, turn_id),
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
        cfg.approval_request_timeout = Duration::from_secs(5);
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
                if ingest.payload.get("method").and_then(Value::as_str) == Some("approval/request")
                {
                    break ingest;
                }
            }
        })
        .await
        .expect("approval/request ingest should arrive");
        let request_id = approval_request.payload["params"]["request_id"]
            .as_str()
            .expect("approval/request ingest should carry request_id")
            .to_string();

        mgr.resolve_approval(&request_id, thread_id, json!({ "decision": "decline" }))
            .await
            .unwrap();
        assert!(!mgr.pending_approvals.contains_key(&request_id));

        server.stop().await;
    }
}
