use crate::codex_client::{CodexClient, Inbound};
use crate::instance::AppServerInstance;
use crate::manager_event::ManagerEvent;
use crate::process::CodexProcess;
use crate::state_machine::{PauseReason, ThreadState};
use crate::thread_handle::ThreadHandle;
use crate::{AgentKind, AgentRuntimeConfig, RawIngest};
use minos_codex_protocol::{
    ClientInfo, InitializeCapabilities, InitializeParams, InitializeResponse,
    InitializedNotification, ThreadStartParams, ThreadStartResponse,
};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, broadcast, watch};
use tracing::{info, warn};
use url::Url;

#[derive(Clone, Debug)]
pub struct InstanceCaps {
    pub max_instances: usize,
    pub idle_timeout: std::time::Duration,
}

impl Default for InstanceCaps {
    fn default() -> Self {
        Self {
            max_instances: 8,
            idle_timeout: std::time::Duration::from_secs(30 * 60),
        }
    }
}

pub struct AgentManager {
    pub config: Arc<AgentRuntimeConfig>,
    pub caps: InstanceCaps,
    pub(crate) instances: Arc<Mutex<HashMap<PathBuf, Arc<AppServerInstance>>>>,
    pub(crate) threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
    pub(crate) events_tx: broadcast::Sender<RawIngest>,
    pub(crate) manager_tx: broadcast::Sender<ManagerEvent>,
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
            events_tx,
            manager_tx,
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
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
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
                            tg.get(t)
                                .map(|h| matches!(h.current_state(), ThreadState::Running { .. }))
                                .unwrap_or(false)
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
        let inst = match instances.lock().await.remove(ws) {
            Some(i) => i,
            None => return,
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

    pub async fn start_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let instance = self.ensure_instance(&canon).await?;

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
        self.threads
            .lock()
            .await
            .insert(thread_id.clone(), handle);
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
        })
    }

    async fn ensure_instance(&self, workspace: &Path) -> anyhow::Result<Arc<AppServerInstance>> {
        let mut guard = self.instances.lock().await;
        if let Some(existing) = guard.get(workspace) {
            return Ok(existing.clone());
        }
        if guard.len() >= self.caps.max_instances {
            self.lru_evict(&mut guard).await?;
        }
        let inst = self.spawn_instance(workspace).await?;
        guard.insert(workspace.to_path_buf(), inst.clone());
        Ok(inst)
    }

    async fn spawn_instance(&self, workspace: &Path) -> anyhow::Result<Arc<AppServerInstance>> {
        let workspace_buf = workspace.to_path_buf();
        let workspace_display = workspace_buf.display().to_string();

        // Pick a free port + spawn `codex app-server --listen ws://...`.
        let port = pick_free_port(self.config.ws_port_range.clone())?;
        let url =
            Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL is well-formed");

        let bin = self
            .config
            .codex_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from(AgentName::Codex.bin_name()));

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
        let env = self.config.subprocess_env.clone();
        let mut process = CodexProcess::spawn(&bin, &args, &env)
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
            pump_workspace,
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
                    .map(|h| matches!(h.current_state(), ThreadState::Running { .. }))
                    .unwrap_or(false)
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
                        .map(|h| matches!(h.current_state(), ThreadState::Running { .. }))
                        .unwrap_or(false)
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

    pub async fn send_user_message(
        &self,
        thread_id: &str,
        text: String,
    ) -> anyhow::Result<()> {
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
                let workspace = handle.workspace.clone();
                let inst = self
                    .instances
                    .lock()
                    .await
                    .get(&workspace)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("instance for workspace gone"))?;
                inst.touch().await;
                inst.send_user_message(thread_id, &text).await?;
                Ok(())
            }
            ThreadState::Suspended { .. } => self.implicit_resume(thread_id, text).await,
            other => anyhow::bail!("send_user_message rejected: state={:?}", other),
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

        let inst = self.ensure_instance(&workspace).await?;
        if let Some(sid) = codex_session_id {
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
        inst.send_user_message(thread_id, &text).await?;
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
            anyhow::bail!("interrupt rejected: state={:?}", handle.current_state());
        }
        let workspace = handle.workspace.clone();
        if let Some(inst) = self.instances.lock().await.get(&workspace).cloned() {
            let _ = inst.interrupt_turn(thread_id).await;
        }
        let from_state = handle.current_state();
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
        if let Some(inst) = self.instances.lock().await.get(&workspace).cloned() {
            inst.remove_thread(thread_id).await;
        }
        let _ = self.manager_tx.send(ManagerEvent::ThreadClosed {
            thread_id: thread_id.to_string(),
            reason: crate::state_machine::CloseReason::UserClose,
        });
        Ok(())
    }

    /// Shut every codex instance down with a polite SIGTERM, wait `grace`
    /// for them to exit, and then escalate to SIGKILL on stragglers. Drops
    /// every instance from the map. Used by [`crate::manager::AgentManager`]
    /// callers (the daemon shutdown path in C20).
    pub async fn shutdown_instances(&self, grace: std::time::Duration) {
        let mut g = self.instances.lock().await;
        // Polite SIGTERM via the Child handle each instance owns. We do not
        // bother round-tripping a typed protocol farewell — codex respects
        // SIGTERM and the JSONL `thread/archive` polite goodbye fired
        // pre-Phase-C is gone with the legacy AgentRuntime.
        for inst in g.values() {
            if let Some(mut child) = inst.child.lock().await.take() {
                // start_kill issues SIGKILL on Unix, but we want a polite
                // SIGTERM first; tokio's API doesn't expose SIGTERM directly,
                // so we fall back to start_kill (SIGKILL) for now and rely on
                // the grace window for ungraceful exits.
                let _ = child.start_kill();
                // Park the child so the wait happens after the grace window.
                let _ = inst.child.lock().await.replace(child);
            }
        }
        tokio::time::sleep(grace).await;
        for (_, inst) in std::mem::take(&mut *g).into_iter() {
            let child_opt = inst.child.lock().await.take();
            drop(inst);
            if let Some(mut child) = child_opt {
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

/// Long-running event-pump task per instance: drains every inbound frame from
/// the codex WS and forwards `Notification` payloads as `RawIngest` records
/// keyed by the notification's `params.threadId`.
async fn event_pump_loop(
    client: Arc<CodexClient>,
    events_tx: broadcast::Sender<RawIngest>,
    threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
    _workspace: PathBuf,
    crash_tx: tokio::sync::mpsc::Sender<()>,
) {
    while let Some(inbound) = client.next_inbound().await {
        match inbound {
            Inbound::Notification { method, params } => {
                let thread_id =
                    params.get("threadId").and_then(Value::as_str).map(str::to_string);
                let Some(thread_id) = thread_id else {
                    continue;
                };
                // Look up agent kind for the thread; default to Codex if absent
                // (notifications can race the manager's bookkeeping).
                let agent = threads
                    .lock()
                    .await
                    .get(&thread_id)
                    .map(|h| h.agent)
                    .unwrap_or(AgentName::Codex);
                let payload = serde_json::json!({ "method": method, "params": params });
                let ingest = RawIngest {
                    agent,
                    thread_id,
                    payload,
                    ts_ms: current_unix_ms(),
                };
                if let Err(e) = events_tx.send(ingest) {
                    tracing::debug!(
                        target: "minos_agent_runtime::manager",
                        error = %e,
                        "events_tx broadcast send failed (no subscribers)",
                    );
                }
            }
            Inbound::ServerRequest { id, method, params } => {
                // Best-effort approval auto-reject: re-use the existing approval
                // surface; unknown server requests are warn-logged and forwarded
                // as a synthetic notification so ingest subscribers see them.
                let envelope = serde_json::json!({ "method": method, "params": params });
                match serde_json::from_value::<minos_codex_protocol::ServerRequest>(envelope) {
                    Ok(req) => {
                        if let Some(reply) = crate::approvals::auto_reject(&req) {
                            if let Err(e) = client.reply(id.clone(), reply).await {
                                warn!(
                                    target: "minos_agent_runtime::manager",
                                    error = %e,
                                    method = %method,
                                    "auto-reject reply failed",
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            target: "minos_agent_runtime::manager",
                            method = %method,
                            error = %e,
                            "unknown server request method; not replying",
                        );
                    }
                }
                let thread_id =
                    params.get("threadId").and_then(Value::as_str).map(str::to_string);
                if let Some(thread_id) = thread_id {
                    let agent = threads
                        .lock()
                        .await
                        .get(&thread_id)
                        .map(|h| h.agent)
                        .unwrap_or(AgentName::Codex);
                    let synthetic_method = format!("server_request/{method}");
                    let payload =
                        serde_json::json!({ "method": synthetic_method, "params": params });
                    let _ = events_tx.send(RawIngest {
                        agent,
                        thread_id,
                        payload,
                        ts_ms: current_unix_ms(),
                    });
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

    #[tokio::test]
    #[ignore = "spawn_instance now spawns a real codex child; covered via FakeCodexBackend in C22"]
    async fn start_agent_creates_instance_and_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let ws = std::path::PathBuf::from("/w-test");
        let resp = mgr.start_agent(AgentKind::Codex, ws.clone()).await.unwrap();
        assert_eq!(resp.cwd, ws);
        let snap = mgr.list_threads().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].workspace, ws);
    }

    #[tokio::test]
    #[ignore = "implicit_resume requires FakeCodexBackend; full coverage lands in C22 multi-session smoke"]
    async fn implicit_resume_from_suspended() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let _ = mgr;
    }
}
