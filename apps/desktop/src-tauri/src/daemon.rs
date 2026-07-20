//! Local JSON-RPC client for `minos-daemon` (same contract as `minos-tui` DaemonBackend).
//!
//! Connect path mirrors TUI:
//! 1. Try discovery file / explicit URL
//! 2. On miss or connect failure, start a managed in-process daemon with local RPC
//! 3. Re-discover and connect

use anyhow::{anyhow, Context, Result};
use futures::{StreamExt, TryStreamExt};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use minos_daemon::local_rpc::LocalRpcConfig;
use minos_daemon::{DaemonHandle, LocalState, RelayConfig};
use minos_domain::AgentName;
use minos_protocol::local_rpc::{
    LocalConversationEvent, LocalIngestFrame, LocalManagerEvent, ReadThreadRawHistoryResponse,
};
use minos_protocol::{
    AppendConversationMessageParams, ApprovalDecisionRequest, CreateConversationParams,
    CreateProjectRequest, ListClisResponse, ListConversationAgentSessionsParams,
    ListConversationMessagesParams, ListConversationsParams, ListProjectsResponse,
    LocalConversationMessage, LocalConversationSummary, ProjectSummary, ReadThreadParams,
    SendUserMessageRequest, StartAgentInConversationRequest, StartAgentResponse, ThreadState,
    ThreadSummary, UpdateConversationParams,
};
use minos_ui_protocol::UiEventMessage;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

/// Frontend event channels (push path, TUI-parity).
pub const EVENT_INGEST: &str = "daemon://ingest";
pub const EVENT_MANAGER: &str = "daemon://manager";
pub const EVENT_CONVERSATION: &str = "daemon://conversation";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDto {
    pub connected: bool,
    pub endpoint: Option<String>,
    pub error: Option<String>,
    /// `discovery` | `managed` | `explicit` | `error`
    pub source: String,
    /// True when this process owns a managed DaemonHandle.
    pub managed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub workspace_path: String,
    pub conversation_count: u32,
    pub running_agents: u32,
    pub needs_attention: u32,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDto {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub preview: String,
    pub updated_at: String,
    pub updated_at_ms: i64,
    pub message_count: u32,
    pub agent_session_count: u32,
    pub participating_agents: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    pub progress: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    pub running_count: u32,
    pub approval_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub id: String,
    /// Durable timeline sort key (`chat_messages.message_seq`). UI uses ASC.
    pub message_seq: i64,
    pub role: String,
    pub agent: Option<String>,
    pub session_id: Option<String>,
    pub body: String,
    pub time: String,
    pub created_at_ms: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<MentionDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionDto {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_short_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    pub id: String,
    pub conversation_id: String,
    pub conversation_title: Option<String>,
    pub agent: String,
    pub short_id: String,
    pub status: String,
    pub model: String,
    pub parent_id: Option<String>,
    pub summary: String,
    pub message_count: u32,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub needs_continue: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptItemDto {
    pub id: String,
    /// assistant | user | tool | tool_result | reasoning | status | error | approval | question
    pub kind: String,
    pub role: Option<String>,
    pub text: String,
    /// Optional secondary line (tool args detail, collapsed by UI).
    pub detail: Option<String>,
    pub title: Option<String>,
    pub ts_ms: i64,
    pub seq: u64,
    pub message_id: Option<String>,
    /// Pending approval / question request id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Underlying method / channel (e.g. `x.ai/exit_plan_mode`, `opencode/question`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_method: Option<String>,
    /// Structured options for question / single-select prompts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<TranscriptOptionDto>>,
    /// For OpenCode permission: wire response token when accepting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approve_response: Option<String>,
    /// For OpenCode permission: wire response token when declining.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decline_response: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptOptionDto {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPageDto {
    pub thread_id: String,
    pub items: Vec<TranscriptItemDto>,
    pub next_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliDto {
    pub agent: String,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAgentResultDto {
    pub thread_id: String,
    pub cwd: String,
}

/// Live ingest delta for the webview (assembled transcript items + approval hint).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestEventDto {
    pub thread_id: String,
    pub seq: u64,
    pub agent: String,
    pub ts_ms: i64,
    pub items: Vec<TranscriptItemDto>,
    pub has_pending_approval: bool,
}

/// Manager lifecycle event (thread status) for the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ManagerEventDto {
    ThreadAdded {
        thread_id: String,
        agent: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_thread_id: Option<String>,
        workspace: String,
    },
    ThreadStateChanged {
        thread_id: String,
        /// idle | running | suspended | done
        status: String,
        at_ms: i64,
    },
    ThreadClosed {
        thread_id: String,
    },
    InstanceCrashed {
        affected_thread_ids: Vec<String>,
    },
}

/// Conversation timeline dirty signal (frontend re-lists messages).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationEventDto {
    pub conversation_id: String,
    pub message_seq: i64,
}

pub struct DaemonBridge {
    inner: Mutex<BridgeInner>,
    /// Serialize connect/bootstrap so React StrictMode double-mount cannot
    /// start two managed daemons racing on the same discovery file.
    connect_lock: Mutex<()>,
    /// Webview emitter; set once from Tauri `setup`.
    app: Mutex<Option<AppHandle>>,
    /// Bumped on each (re)connect so old subscription pumps exit.
    pump_generation: Arc<AtomicU64>,
}

struct BridgeInner {
    client: Option<Arc<WsClient>>,
    endpoint: Option<String>,
    last_error: Option<String>,
    source: String,
    /// Keep alive for the app lifetime when we started local RPC ourselves.
    managed: Option<Arc<DaemonHandle>>,
    /// Generation of the pumps currently running for this client.
    pumps_running_for: u64,
}

impl DaemonBridge {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BridgeInner {
                client: None,
                endpoint: None,
                last_error: None,
                source: "none".into(),
                managed: None,
                pumps_running_for: 0,
            }),
            connect_lock: Mutex::new(()),
            app: Mutex::new(None),
            pump_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Attach Tauri app handle so connect can start push pumps.
    pub async fn attach_app(&self, app: AppHandle) {
        *self.app.lock().await = Some(app);
        // If already connected (unlikely before setup), arm pumps.
        let client = {
            let guard = self.inner.lock().await;
            guard.client.clone()
        };
        if let Some(client) = client {
            self.ensure_event_pumps(client).await;
        }
    }

    /// Connect to an existing daemon, or start a managed one (TUI parity).
    pub async fn connect(&self, override_url: Option<String>) -> ConnectionDto {
        let _gate = self.connect_lock.lock().await;
        let explicit = override_url.is_some();

        // Already healthy? Reuse (and ensure push pumps are armed).
        {
            let guard = self.inner.lock().await;
            if let (Some(client), Some(_endpoint)) = (&guard.client, &guard.endpoint) {
                if health_ok(client).await {
                    let client = client.clone();
                    let status = Self::status_locked(&guard);
                    drop(guard);
                    self.ensure_event_pumps(client).await;
                    return status;
                }
            }
        }

        // 1) Try discovery / explicit URL (with short retries — server may be warming).
        match resolve_daemon_url(override_url.clone()) {
            Ok(url) => match try_ws_health_retry(&url, 8).await {
                Ok(client) => {
                    let client = Arc::clone(&client);
                    let mut guard = self.inner.lock().await;
                    guard.client = Some(Arc::clone(&client));
                    guard.endpoint = Some(url);
                    guard.last_error = None;
                    guard.source = if explicit {
                        "explicit".into()
                    } else if guard.managed.is_some() {
                        "managed".into()
                    } else {
                        "discovery".into()
                    };
                    let status = Self::status_locked(&guard);
                    drop(guard);
                    self.ensure_event_pumps(client).await;
                    return status;
                }
                Err(error) if explicit => {
                    let mut guard = self.inner.lock().await;
                    guard.client = None;
                    guard.endpoint = Some(url);
                    guard.last_error = Some(error.to_string());
                    guard.source = "explicit".into();
                    return Self::status_locked(&guard);
                }
                Err(error) => {
                    warn!(
                        target: "minos_desktop",
                        error = %error,
                        url = %url,
                        "failed to connect to discovered daemon"
                    );
                    // Stale discovery pointing at a dead port is common after crashes.
                    if !explicit {
                        remove_discovery_file();
                    }
                }
            },
            Err(error) if explicit => {
                let mut guard = self.inner.lock().await;
                guard.client = None;
                guard.endpoint = None;
                guard.last_error = Some(error.to_string());
                guard.source = "explicit".into();
                return Self::status_locked(&guard);
            }
            Err(error) => {
                warn!(
                    target: "minos_desktop",
                    error = %error,
                    "daemon discovery unavailable"
                );
            }
        }

        // 2) If we already own a managed daemon, only reconnect — never start a second one.
        {
            let guard = self.inner.lock().await;
            if guard.managed.is_some() {
                let preferred = guard
                    .endpoint
                    .clone()
                    .or_else(|| guard.managed.as_ref().and_then(|h| h.local_rpc_url()))
                    .or_else(|| resolve_daemon_url(None).ok())
                    .unwrap_or_default();
                drop(guard);
                return self.connect_after_managed(preferred).await;
            }
        }

        // 3) Start managed daemon (in-process) and connect to the bound URL
        // directly — do not re-read discovery (avoids stale/raced ports).
        info!(target: "minos_desktop", "starting managed daemon with local RPC");
        let (handle, bound_url) = match start_managed_daemon().await {
            Ok(pair) => pair,
            Err(error) => {
                let mut guard = self.inner.lock().await;
                guard.client = None;
                guard.endpoint = None;
                guard.last_error = Some(format!("managed daemon start failed: {error}"));
                guard.source = "error".into();
                return Self::status_locked(&guard);
            }
        };

        {
            let mut guard = self.inner.lock().await;
            guard.managed = Some(handle);
            guard.endpoint = Some(bound_url.clone());
        }

        self.connect_after_managed(bound_url).await
    }

    async fn connect_after_managed(&self, preferred_url: String) -> ConnectionDto {
        // Prefer the URL returned by the binder; fall back to discovery only if empty.
        let candidates = {
            let mut urls = Vec::new();
            if !preferred_url.trim().is_empty() {
                urls.push(preferred_url);
            }
            if let Ok(discovered) = resolve_daemon_url(None) {
                if !urls.iter().any(|u| u == &discovered) {
                    urls.push(discovered);
                }
            }
            urls
        };

        if candidates.is_empty() {
            let mut guard = self.inner.lock().await;
            guard.client = None;
            guard.last_error =
                Some("managed daemon started but no local RPC URL is available".into());
            guard.source = "managed".into();
            return Self::status_locked(&guard);
        }

        // Yield so the jsonrpsee accept task can run on this runtime, then retry.
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(30)).await;

        let mut last_error = String::from("unknown");
        for attempt in 1..=30 {
            for url in &candidates {
                match try_ws_health(url).await {
                    Ok(client) => {
                        let client = Arc::clone(&client);
                        let mut guard = self.inner.lock().await;
                        guard.client = Some(Arc::clone(&client));
                        guard.endpoint = Some(url.clone());
                        guard.last_error = None;
                        guard.source = "managed".into();
                        info!(
                            target: "minos_desktop",
                            endpoint = %url,
                            attempt,
                            "connected to managed daemon"
                        );
                        let status = Self::status_locked(&guard);
                        drop(guard);
                        self.ensure_event_pumps(client).await;
                        return status;
                    }
                    Err(error) => {
                        last_error = error.to_string();
                        if attempt == 1 || attempt % 5 == 0 {
                            warn!(
                                target: "minos_desktop",
                                attempt,
                                url = %url,
                                error = %last_error,
                                "managed daemon connect attempt failed"
                            );
                        }
                    }
                }
            }
            // Bounded backoff; total wait ~ a few seconds.
            let delay_ms = (25 * attempt as u64).min(200);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            tokio::task::yield_now().await;
        }

        let mut guard = self.inner.lock().await;
        guard.client = None;
        if guard.endpoint.is_none() {
            guard.endpoint = candidates.into_iter().next();
        }
        guard.last_error = Some(format!(
            "managed daemon started but connect failed after retries: {last_error}"
        ));
        guard.source = "managed".into();
        Self::status_locked(&guard)
    }

    pub async fn status(&self) -> ConnectionDto {
        let guard = self.inner.lock().await;
        Self::status_locked(&guard)
    }

    fn status_locked(guard: &BridgeInner) -> ConnectionDto {
        ConnectionDto {
            connected: guard.client.is_some(),
            endpoint: guard.endpoint.clone(),
            error: guard.last_error.clone(),
            source: guard.source.clone(),
            managed: guard.managed.is_some(),
        }
    }

    /// Start (or restart) JSON-RPC subscription pumps that emit Tauri events.
    /// Mirrors TUI `DaemonBackend` ingest/manager/conversation pumps.
    async fn ensure_event_pumps(&self, client: Arc<WsClient>) {
        let app = { self.app.lock().await.clone() };
        let Some(app) = app else {
            // setup() has not attached AppHandle yet; attach_app will retry.
            return;
        };

        // Bump generation so previous pumps exit their loops.
        let gen = self.pump_generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut guard = self.inner.lock().await;
            guard.pumps_running_for = gen;
        }

        info!(
            target: "minos_desktop",
            generation = gen,
            "starting daemon event subscription pumps"
        );

        let gen_flag = Arc::clone(&self.pump_generation);
        spawn_ingest_pump(app.clone(), Arc::clone(&client), gen, Arc::clone(&gen_flag));
        spawn_manager_pump(app.clone(), Arc::clone(&client), gen, Arc::clone(&gen_flag));
        spawn_conversation_pump(app, client, gen, gen_flag);
    }

    async fn client(&self) -> Result<Arc<WsClient>> {
        let guard = self.inner.lock().await;
        guard.client.clone().ok_or_else(|| {
            anyhow!(guard
                .last_error
                .clone()
                .unwrap_or_else(|| "not connected".into()))
        })
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectDto>> {
        let client = self.client().await?;
        let response: ListProjectsResponse = client
            .request("minos_local_list_projects", ArrayParams::new())
            .await
            .context("minos_local_list_projects")?;
        Ok(response.projects.into_iter().map(map_project).collect())
    }

    /// Create a project for a workspace folder (same RPC as TUI).
    pub async fn create_project(&self, workspace_path: String) -> Result<ProjectDto> {
        let client = self.client().await?;
        let path = PathBuf::from(&workspace_path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "project".into());
        let slug = name.to_lowercase().replace(' ', "-");
        let req = CreateProjectRequest {
            name: name.clone(),
            workspace_slug: slug,
            workspace_path: Some(workspace_path),
        };
        let response: minos_protocol::CreateProjectResponse = client
            .request("minos_local_create_project", [req])
            .await
            .context("minos_local_create_project")?;
        Ok(map_project(response.project))
    }

    pub async fn list_conversations(&self, project_id: String) -> Result<Vec<ConversationDto>> {
        let client = self.client().await?;
        let params = ListConversationsParams {
            project_id,
            before_updated_at_ms: None,
            limit: Some(100),
        };
        let response: minos_protocol::ListConversationsResponse = client
            .request("minos_local_list_conversations", [params])
            .await
            .context("minos_local_list_conversations")?;
        Ok(response
            .conversations
            .into_iter()
            .map(map_conversation)
            .collect())
    }

    pub async fn list_messages(&self, conversation_id: String) -> Result<Vec<MessageDto>> {
        let client = self.client().await?;
        let params = ListConversationMessagesParams {
            conversation_id,
            before_seq: None,
            limit: Some(200),
        };
        let response: minos_protocol::ListConversationMessagesResponse = client
            .request("minos_local_list_conversation_messages", [params])
            .await
            .context("minos_local_list_conversation_messages")?;
        // Daemon returns newest-first (message_seq DESC); UI needs chronological ASC.
        let mut messages: Vec<MessageDto> =
            response.messages.into_iter().map(map_message).collect();
        messages.reverse();
        messages.sort_by_key(|m| m.message_seq);
        Ok(messages)
    }

    pub async fn list_sessions(&self, conversation_id: String) -> Result<Vec<SessionDto>> {
        let client = self.client().await?;
        let params = ListConversationAgentSessionsParams {
            conversation_id: conversation_id.clone(),
        };
        let response: minos_protocol::ListConversationAgentSessionsResponse = client
            .request("minos_local_list_conversation_agent_sessions", [params])
            .await
            .context("minos_local_list_conversation_agent_sessions")?;
        Ok(response
            .threads
            .into_iter()
            .map(|t| map_session(t, &conversation_id, None))
            .collect())
    }

    /// Aggregate agent sessions across all conversations in a project.
    pub async fn list_project_sessions(&self, project_id: String) -> Result<Vec<SessionDto>> {
        let convs = self.list_conversations(project_id).await?;
        let mut out = Vec::new();
        for conv in convs {
            let mut sessions = self.list_sessions(conv.id.clone()).await?;
            for s in &mut sessions {
                s.conversation_title = Some(conv.title.clone());
            }
            out.extend(sessions);
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.last_ts_ms));
        Ok(out)
    }

    pub async fn read_transcript(
        &self,
        thread_id: String,
        from_seq: Option<u64>,
        limit: Option<u32>,
        full: bool,
    ) -> Result<TranscriptPageDto> {
        let client = self.client().await?;
        // One assembler across all pages so TextDelta that spans RPC pages
        // still merges into a single transcript item.
        let mut assembler = TranscriptAssembler::new(thread_id.clone());
        let page_limit: u32 = if full {
            1000
        } else {
            limit.unwrap_or(500).clamp(1, 1000)
        };
        let mut cursor = from_seq;
        let mut pages = 0u32;
        // Safety cap: 1000 * 100 = 100k raw frames max per full open.
        const MAX_FULL_PAGES: u32 = 100;
        loop {
            let req = ReadThreadParams {
                thread_id: thread_id.clone(),
                from_seq: cursor,
                limit: page_limit,
            };
            let response: ReadThreadRawHistoryResponse = client
                .request("minos_local_read_thread_raw_history", [req])
                .await
                .context("minos_local_read_thread_raw_history")?;
            for frame in response.events {
                assembler.ingest_frame(frame.seq, frame.ts_ms, frame.ui_events);
            }
            pages += 1;
            if !full {
                return Ok(TranscriptPageDto {
                    thread_id,
                    items: assembler.finish(),
                    next_seq: response.next_seq,
                });
            }
            let Some(next) = response.next_seq else {
                break;
            };
            if pages >= MAX_FULL_PAGES {
                // Truncated full load — surface next_seq so UI can offer "load more".
                return Ok(TranscriptPageDto {
                    thread_id,
                    items: assembler.finish(),
                    next_seq: Some(next),
                });
            }
            // Daemon from_seq is exclusive; next_seq is the next start seq → from = next - 1.
            cursor = Some(next.saturating_sub(1));
        }
        Ok(TranscriptPageDto {
            thread_id,
            items: assembler.finish(),
            next_seq: None,
        })
    }

    pub async fn create_conversation(
        &self,
        project_id: String,
        title: String,
    ) -> Result<ConversationDto> {
        let client = self.client().await?;
        let params = CreateConversationParams { project_id, title };
        let response: minos_protocol::CreateConversationResponse = client
            .request("minos_local_create_conversation", [params])
            .await
            .context("minos_local_create_conversation")?;
        Ok(map_conversation(response.conversation))
    }

    pub async fn update_conversation(
        &self,
        conversation_id: String,
        title: Option<String>,
        priority: Option<String>,
        progress: Option<String>,
    ) -> Result<ConversationDto> {
        let client = self.client().await?;
        let params = UpdateConversationParams {
            conversation_id,
            title,
            priority,
            progress,
        };
        let response: minos_protocol::UpdateConversationResponse = client
            .request("minos_local_update_conversation", [params])
            .await
            .context("minos_local_update_conversation")?;
        Ok(map_conversation(response.conversation))
    }

    pub async fn append_user_message(&self, conversation_id: String, body: String) -> Result<()> {
        let client = self.client().await?;
        let params = AppendConversationMessageParams {
            conversation_id,
            message_id: format!("msg_{}", Uuid::new_v4()),
            thread_id: None,
            sender_role: "user".into(),
            agent: None,
            body,
            reply_to_message_id: None,
            delegation_id: None,
            mentions: vec![],
        };
        let _: minos_protocol::AppendConversationMessageResponse = client
            .request("minos_local_append_conversation_message", [params])
            .await
            .context("minos_local_append_conversation_message")?;
        Ok(())
    }

    pub async fn list_clis(&self) -> Result<Vec<CliDto>> {
        let client = self.client().await?;
        let response: ListClisResponse = client
            .request("minos_local_list_clis", ArrayParams::new())
            .await
            .context("minos_local_list_clis")?;
        Ok(response
            .into_iter()
            .map(|d| {
                let (installed, status) = match &d.status {
                    minos_domain::AgentStatus::Ok => (true, "ok".to_owned()),
                    minos_domain::AgentStatus::Missing => (false, "missing".to_owned()),
                    minos_domain::AgentStatus::Error { reason } => {
                        (false, format!("error: {reason}"))
                    }
                };
                CliDto {
                    agent: agent_label(d.name),
                    installed,
                    path: d.path,
                    version: d.version,
                    status,
                }
            })
            .collect())
    }

    pub async fn start_agent_in_conversation(
        &self,
        conversation_id: String,
        agent: String,
        workspace: String,
        model: Option<String>,
        reasoning_effort: Option<String>,
        instructions: Option<String>,
    ) -> Result<StartAgentResultDto> {
        let client = self.client().await?;
        let agent = parse_agent_name(&agent).ok_or_else(|| anyhow!("unknown agent: {agent}"))?;
        let req = StartAgentInConversationRequest {
            conversation_id,
            agent,
            workspace,
            model,
            reasoning_effort,
            instructions,
        };
        let response: StartAgentResponse = client
            .request("minos_local_start_agent_in_conversation", [req])
            .await
            .context("minos_local_start_agent_in_conversation")?;
        Ok(StartAgentResultDto {
            thread_id: response.session_id,
            cwd: response.cwd,
        })
    }

    pub async fn list_models(&self, runtime: String) -> Result<minos_protocol::ListModelsResponse> {
        let client = self.client().await?;
        let agent =
            parse_agent_name(&runtime).ok_or_else(|| anyhow!("unknown runtime: {runtime}"))?;
        let req = minos_protocol::ListModelsRequest { runtime: agent };
        client
            .request("minos_local_list_models", [req])
            .await
            .context("minos_local_list_models")
    }

    pub async fn list_agent_profiles(&self) -> Result<minos_protocol::ListAgentProfilesResponse> {
        let client = self.client().await?;
        client
            .request::<minos_protocol::ListAgentProfilesResponse, [(); 0]>(
                "minos_local_list_agent_profiles",
                [],
            )
            .await
            .context("minos_local_list_agent_profiles")
    }

    pub async fn create_agent_profile(
        &self,
        req: minos_protocol::CreateAgentProfileRequest,
    ) -> Result<minos_protocol::AgentProfileSummary> {
        let client = self.client().await?;
        client
            .request("minos_local_create_agent_profile", [req])
            .await
            .context("minos_local_create_agent_profile")
    }

    pub async fn delete_agent_profile(&self, id: String) -> Result<()> {
        let client = self.client().await?;
        let req = minos_protocol::DeleteAgentProfileRequest { id };
        client
            .request("minos_local_delete_agent_profile", [req])
            .await
            .context("minos_local_delete_agent_profile")
    }

    pub async fn send_user_message(&self, thread_id: String, text: String) -> Result<()> {
        let client = self.client().await?;
        let req = SendUserMessageRequest {
            session_id: thread_id,
            text,
        };
        client
            .request::<(), _>("minos_local_send_user_message", [req])
            .await
            .context("minos_local_send_user_message")?;
        Ok(())
    }

    /// Reattach a suspended/persisted thread. When `auto_continue` is true and
    /// the store has `needs_continue`, injects a one-shot CONTINUE prompt.
    pub async fn resume_thread(&self, thread_id: String, auto_continue: bool) -> Result<()> {
        let client = self.client().await?;
        let req = minos_protocol::ResumeThreadRequest {
            thread_id,
            auto_continue,
        };
        let _: StartAgentResponse = client
            .request("minos_local_resume_thread", [req])
            .await
            .context("minos_local_resume_thread")?;
        Ok(())
    }

    pub async fn resolve_approval(
        &self,
        request_id: String,
        thread_id: String,
        decision: serde_json::Value,
    ) -> Result<()> {
        let client = self.client().await?;
        let params = ApprovalDecisionRequest {
            request_id,
            thread_id,
            decision,
        };
        client
            .request::<(), _>("minos_local_approval_decision", [params])
            .await
            .context("minos_local_approval_decision")?;
        Ok(())
    }

    pub async fn respond_opencode_permission(
        &self,
        thread_id: String,
        permission_id: String,
        response: String,
    ) -> Result<()> {
        let client = self.client().await?;
        let params = minos_protocol::local_rpc::RespondOpencodePermissionRequest {
            thread_id,
            permission_id,
            response,
        };
        client
            .request::<(), _>("minos_local_respond_opencode_permission", [params])
            .await
            .context("minos_local_respond_opencode_permission")?;
        Ok(())
    }

    pub async fn respond_opencode_question(
        &self,
        thread_id: String,
        question_id: String,
        answers: Vec<Vec<String>>,
    ) -> Result<()> {
        let client = self.client().await?;
        let params = minos_protocol::local_rpc::RespondOpencodeQuestionRequest {
            thread_id,
            question_id,
            answers,
        };
        client
            .request::<(), _>("minos_local_respond_opencode_question", [params])
            .await
            .context("minos_local_respond_opencode_question")?;
        Ok(())
    }
}

fn parse_agent_name(value: &str) -> Option<AgentName> {
    let normalized = value.to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
}

async fn health_ok(client: &WsClient) -> bool {
    client
        .request::<serde_json::Value, _>("minos_local_health", ArrayParams::new())
        .await
        .is_ok()
}

async fn try_ws_health(url: &str) -> Result<Arc<WsClient>> {
    let client = WsClientBuilder::default()
        .connection_timeout(Duration::from_secs(2))
        .request_timeout(Duration::from_secs(5))
        .build(url)
        .await
        .with_context(|| format!("ws connect {url}"))?;
    let client = Arc::new(client);
    client
        .request::<serde_json::Value, _>("minos_local_health", ArrayParams::new())
        .await
        .context("minos_local_health")?;
    Ok(client)
}

async fn try_ws_health_retry(url: &str, attempts: u32) -> Result<Arc<WsClient>> {
    let mut last = anyhow!("no attempts");
    for i in 0..attempts {
        match try_ws_health(url).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                last = e;
                tokio::time::sleep(Duration::from_millis(40 * (u64::from(i) + 1))).await;
            }
        }
    }
    Err(last)
}

fn remove_discovery_file() {
    if let Some(path) = daemon_discovery_path() {
        match std::fs::remove_file(&path) {
            Ok(()) => info!(
                target: "minos_desktop",
                path = %path.display(),
                "removed stale daemon discovery file"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(
                target: "minos_desktop",
                error = %e,
                path = %path.display(),
                "failed to remove stale discovery file"
            ),
        }
    }
}

async fn start_managed_daemon() -> Result<(Arc<DaemonHandle>, String)> {
    let minos_home = minos_daemon::paths::minos_home().map_err(|e| anyhow!(e.to_string()))?;
    let local_state_path = minos_home.join("local-state.json");
    let local_state = LocalState::load_or_init(&local_state_path)
        .map_err(|e| anyhow!("LocalState::load_or_init: {e}"))?;
    let discovery_path = minos_daemon::paths::run_dir()
        .map_err(|e| anyhow!(e.to_string()))?
        .join("tui-daemon-rpc.json");
    // Clear any stale discovery before bind so external clients cannot attach
    // to a dead port while we start.
    let _ = std::fs::remove_file(&discovery_path);
    let local_rpc_config = LocalRpcConfig {
        addr: "127.0.0.1:0".parse().context("parse local rpc bind addr")?,
        discovery_path: discovery_path.clone(),
    };
    let handle = DaemonHandle::start_with_local_rpc(
        relay_config_from_env(),
        local_state.self_device_id,
        None,
        None,
        default_mac_name(),
        Some(local_rpc_config),
    )
    .await
    .map_err(|e| anyhow!("DaemonHandle::start_with_local_rpc: {e}"))?;
    let url = handle
        .local_rpc_url()
        .ok_or_else(|| anyhow!("managed daemon started without local_rpc_url (internal bug)"))?;
    info!(
        target: "minos_desktop",
        discovery_path = %discovery_path.display(),
        url = %url,
        "started managed daemon for desktop"
    );
    Ok((handle, url))
}

fn relay_config_from_env() -> RelayConfig {
    let backend_url = std::env::var("MINOS_BACKEND_URL")
        .ok()
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .unwrap_or_default();
    RelayConfig::new(backend_url)
}

fn default_mac_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Minos Desktop".into())
}

fn resolve_daemon_url(override_url: Option<String>) -> Result<String> {
    if let Some(url) = override_url {
        return Ok(url);
    }
    let path = daemon_discovery_path()
        .ok_or_else(|| anyhow!("cannot resolve HOME for daemon discovery path"))?;
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read daemon discovery file at {}", path.display()))?;
    let payload: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse daemon discovery at {}", path.display()))?;
    payload
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow!(
                "daemon discovery file at {} does not contain a `url` field",
                path.display()
            )
        })
}

fn daemon_discovery_path() -> Option<PathBuf> {
    // Prefer the same helper as daemon crate when available after managed start;
    // for cold discovery before minos_home exists, fall back to $HOME/.minos.
    minos_daemon::paths::run_dir()
        .ok()
        .map(|d| d.join("tui-daemon-rpc.json"))
        .or_else(|| {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            Some(
                PathBuf::from(home)
                    .join(".minos")
                    .join("run")
                    .join("tui-daemon-rpc.json"),
            )
        })
}

fn map_project(p: ProjectSummary) -> ProjectDto {
    ProjectDto {
        id: p.project_id,
        name: p.name,
        workspace_path: p
            .workspace_path
            .unwrap_or_else(|| format!("~/.minos/projects/{}", p.workspace_slug)),
        conversation_count: p.thread_count,
        running_agents: 0,
        needs_attention: 0,
        updated_at_ms: p.updated_at_ms,
    }
}

fn map_conversation(c: LocalConversationSummary) -> ConversationDto {
    ConversationDto {
        id: c.conversation_id,
        project_id: c.project_id,
        title: c.title,
        preview: c
            .last_message_preview
            .unwrap_or_else(|| "No messages yet".into()),
        updated_at: relative_time(c.updated_at_ms),
        updated_at_ms: c.updated_at_ms,
        message_count: c.message_count,
        agent_session_count: c.agent_session_count,
        participating_agents: c
            .participating_agents
            .into_iter()
            .map(agent_label)
            .collect(),
        priority: c.priority,
        progress: if c.progress.is_empty() {
            "todo".into()
        } else {
            c.progress
        },
        branch: c.branch,
        worktree: c.worktree_path,
        running_count: c.running_count,
        approval_count: c.needs_attention_count,
    }
}

/// Conversation timeline kind for durable `chat_messages` rows.
///
/// Real agent approvals are **session reverse-requests** (permission / plan /
/// opencode question) with a `request_id` on the transcript — never free text
/// on the conversation timeline. Do **not** infer `"approval"` from body
/// substrings like `"approval"` / `"Permission:"`: agent prose about plans
/// ("plan-approval", "needs approval") would false-positive and render a dead
/// Allow/Deny card (TUI never does this).
fn conversation_timeline_kind(_body: &str) -> &'static str {
    "text"
}

fn map_message(m: LocalConversationMessage) -> MessageDto {
    let kind = conversation_timeline_kind(&m.body);
    let mentions = m
        .mentions
        .into_iter()
        .map(|mention| MentionDto {
            agent: agent_label(mention.agent),
            thread_id: mention.thread_id,
            thread_short_id: mention.thread_short_id,
        })
        .collect();
    MessageDto {
        id: m.message_id,
        message_seq: m.message_seq,
        role: m.sender_role,
        agent: m.agent.map(agent_label),
        session_id: m.thread_id,
        body: m.body,
        time: clock_time(m.created_at_ms),
        created_at_ms: m.created_at_ms,
        kind: kind.into(),
        reply_to_message_id: m.reply_to_message_id,
        delegation_id: m.delegation_id,
        mentions,
    }
}

fn map_session(
    t: ThreadSummary,
    conversation_id: &str,
    conversation_title: Option<String>,
) -> SessionDto {
    let short_id = short_thread_id(&t.thread_id);
    let status = thread_status_label(&t);
    let agent = agent_label(t.agent);
    SessionDto {
        id: t.thread_id,
        conversation_id: conversation_id.to_owned(),
        conversation_title,
        agent: agent.clone(),
        short_id,
        status,
        model: "—".into(),
        parent_id: t.parent_thread_id,
        summary: t.title.unwrap_or_else(|| format!("{agent} session")),
        message_count: t.message_count,
        first_ts_ms: t.first_ts_ms,
        last_ts_ms: t.last_ts_ms,
        needs_continue: t.needs_continue,
    }
}

fn short_thread_id(thread_id: &str) -> String {
    let mut end = thread_id.len().min(8);
    while end > 0 && !thread_id.is_char_boundary(end) {
        end -= 1;
    }
    thread_id[..end].to_owned()
}

/// Folds raw UiEventMessage stream into chat-like items (aligned with TUI ChatState).
struct TranscriptAssembler {
    thread_id: String,
    items: Vec<TranscriptItemDto>,
    /// message_id → role for open messages
    open_roles: std::collections::HashMap<String, minos_ui_protocol::MessageRole>,
    counter: u64,
}

impl TranscriptAssembler {
    fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            items: Vec::new(),
            open_roles: std::collections::HashMap::new(),
            counter: 0,
        }
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{}-{prefix}-{}", self.thread_id, self.counter)
    }

    fn ingest_frame(&mut self, seq: u64, ts_ms: i64, events: Vec<UiEventMessage>) {
        for ev in events {
            self.apply(seq, ts_ms, ev);
        }
    }

    fn finish(self) -> Vec<TranscriptItemDto> {
        self.items
    }

    fn role_of(&self, message_id: &str) -> minos_ui_protocol::MessageRole {
        self.open_roles
            .get(message_id)
            .copied()
            .unwrap_or(minos_ui_protocol::MessageRole::Assistant)
    }

    fn tail_text_matches(&self, message_id: &str, kind: &str) -> bool {
        matches!(
            self.items.last(),
            Some(item)
                if item.kind == kind
                    && item.message_id.as_deref() == Some(message_id)
        )
    }

    fn append_text(
        &mut self,
        seq: u64,
        ts_ms: i64,
        message_id: String,
        chunk: String,
        replace: bool,
    ) {
        use minos_ui_protocol::MessageRole;
        if chunk.is_empty() {
            return;
        }
        let role = self.role_of(&message_id);
        let kind = match role {
            MessageRole::User => "user",
            MessageRole::System => "system",
            MessageRole::Assistant => "assistant",
        };
        if self.tail_text_matches(&message_id, kind) {
            if let Some(last) = self.items.last_mut() {
                if replace {
                    last.text = chunk;
                } else {
                    last.text.push_str(&chunk);
                }
                last.ts_ms = ts_ms;
                last.seq = seq;
            }
        } else {
            let id = self.next_id("msg");
            self.items.push(TranscriptItemDto {
                id,
                kind: kind.into(),
                role: Some(kind.into()),
                text: chunk,
                detail: None,
                title: None,
                ts_ms,
                seq,
                message_id: Some(message_id),
                request_id: None,
                approval_method: None,
                options: None,
                approve_response: None,
                decline_response: None,
            });
        }
    }

    fn push_simple(
        &mut self,
        seq: u64,
        ts_ms: i64,
        kind: &str,
        text: String,
        title: Option<String>,
        detail: Option<String>,
        message_id: Option<String>,
    ) {
        let id = self.next_id(kind);
        self.items.push(TranscriptItemDto {
            id,
            kind: kind.into(),
            role: None,
            text,
            detail,
            title,
            ts_ms,
            seq,
            message_id,
            request_id: None,
            approval_method: None,
            options: None,
            approve_response: None,
            decline_response: None,
        });
    }

    fn apply(&mut self, seq: u64, ts_ms: i64, event: UiEventMessage) {
        match event {
            UiEventMessage::MessageStarted {
                message_id, role, ..
            } => {
                self.open_roles.insert(message_id, role);
            }
            UiEventMessage::MessageCompleted { message_id, .. } => {
                self.open_roles.remove(&message_id);
            }
            UiEventMessage::TextDelta { message_id, text } => {
                self.append_text(seq, ts_ms, message_id, text.render_preview(), false);
            }
            UiEventMessage::TextReplace { message_id, text } => {
                self.append_text(seq, ts_ms, message_id, text.render_preview(), true);
            }
            UiEventMessage::ReasoningDelta { message_id, text }
            | UiEventMessage::ReasoningReplace { message_id, text } => {
                let chunk = text.render_preview();
                if chunk.is_empty() {
                    return;
                }
                if self.tail_text_matches(&message_id, "reasoning") {
                    if let Some(last) = self.items.last_mut() {
                        last.text.push_str(&chunk);
                        last.ts_ms = ts_ms;
                        last.seq = seq;
                    }
                } else {
                    let id = self.next_id("think");
                    self.items.push(TranscriptItemDto {
                        id,
                        kind: "reasoning".into(),
                        role: Some("assistant".into()),
                        text: chunk,
                        detail: None,
                        title: Some("Thinking".into()),
                        ts_ms,
                        seq,
                        message_id: Some(message_id),
                        request_id: None,
                        approval_method: None,
                        options: None,
                        approve_response: None,
                        decline_response: None,
                    });
                }
            }
            UiEventMessage::ToolCallPlaced {
                message_id,
                tool_call_id,
                name,
                args_json,
            } => {
                let args = args_json.render_preview();
                // text = bare target (path/cmd); title = tool name. UI derives Grok verbs.
                let target = summarize_tool_args(&name, &args);
                let id = self.next_id("tool");
                self.items.push(TranscriptItemDto {
                    id: format!("{id}:{tool_call_id}"),
                    kind: "tool".into(),
                    role: None,
                    text: target,
                    detail: if args.trim().is_empty() {
                        None
                    } else {
                        Some(truncate_str(&args, 2000))
                    },
                    title: Some(name),
                    ts_ms,
                    seq,
                    message_id: Some(message_id),
                    request_id: None,
                    approval_method: None,
                    options: None,
                    approve_response: None,
                    decline_response: None,
                });
            }
            UiEventMessage::ToolCallCompleted {
                tool_call_id,
                output,
                is_error,
            } => {
                let out = output.render_preview();
                // Keep bare target in `text`; put output into `detail` for expand.
                let mut updated = false;
                for item in self.items.iter_mut().rev() {
                    if item.kind == "tool" && item.id.ends_with(&format!(":{tool_call_id}")) {
                        item.kind = if is_error {
                            "tool_error".into()
                        } else {
                            "tool_result".into()
                        };
                        let detail = truncate_str(&out, 4000);
                        item.detail = if detail.is_empty() {
                            None
                        } else {
                            Some(detail)
                        };
                        item.ts_ms = ts_ms;
                        item.seq = seq;
                        updated = true;
                        break;
                    }
                }
                if !updated {
                    let summary = summarize_tool_output_line(&out, is_error);
                    self.push_simple(
                        seq,
                        ts_ms,
                        if is_error {
                            "tool_error"
                        } else {
                            "tool_result"
                        },
                        summary,
                        Some(tool_call_id),
                        Some(truncate_str(&out, 4000)),
                        None,
                    );
                }
            }
            UiEventMessage::ThreadOpened { title, agent, .. } => {
                self.push_simple(
                    seq,
                    ts_ms,
                    "status",
                    format!(
                        "Session started · {}{}",
                        agent_label(agent),
                        title
                            .as_ref()
                            .map(|t| format!(" · {t}"))
                            .unwrap_or_default()
                    ),
                    None,
                    None,
                    None,
                );
            }
            UiEventMessage::ThreadClosed { .. } => {
                self.push_simple(
                    seq,
                    ts_ms,
                    "status",
                    "Session closed".into(),
                    None,
                    None,
                    None,
                );
            }
            UiEventMessage::Error { code, message, .. } => {
                self.push_simple(
                    seq,
                    ts_ms,
                    "error",
                    if code.is_empty() {
                        message
                    } else {
                        format!("{code}: {message}")
                    },
                    Some("Error".into()),
                    None,
                    None,
                );
            }
            UiEventMessage::SubagentSpawned {
                sub_thread_id,
                agent,
                title,
                prompt,
                ..
            } => {
                self.push_simple(
                    seq,
                    ts_ms,
                    "status",
                    format!(
                        "Subagent {} · #{}{}",
                        agent_label(agent),
                        short_thread_id(&sub_thread_id),
                        title
                            .as_ref()
                            .or(prompt.as_ref())
                            .map(|t| format!(" · {t}"))
                            .unwrap_or_default()
                    ),
                    Some("Subagent".into()),
                    None,
                    None,
                );
            }
            UiEventMessage::SubagentStatusUpdated {
                sub_thread_id,
                status,
            } => {
                let label = match status {
                    minos_ui_protocol::SubagentStatus::Running => "running",
                    minos_ui_protocol::SubagentStatus::Completed => "completed",
                    minos_ui_protocol::SubagentStatus::Failed => "failed",
                    minos_ui_protocol::SubagentStatus::Interrupted => "interrupted",
                };
                self.push_simple(
                    seq,
                    ts_ms,
                    "status",
                    format!("Subagent #{} · {label}", short_thread_id(&sub_thread_id)),
                    Some("Subagent".into()),
                    None,
                    None,
                );
            }
            // Product-critical Raw: user-facing reverse-requests.
            // Other Raw ACP noise is intentionally dropped.
            UiEventMessage::Raw { kind, payload_json } => {
                if kind == "approval/request" {
                    if let Some(item) = approval_item_from_payload(seq, ts_ms, &payload_json) {
                        self.items.push(item);
                    }
                } else if kind == "approval/timeout" {
                    self.push_simple(
                        seq,
                        ts_ms,
                        "status",
                        "Approval timed out".into(),
                        Some("Approval".into()),
                        None,
                        None,
                    );
                } else if kind == "opencode/permission.updated" {
                    if let Some(item) =
                        opencode_permission_item_from_payload(seq, ts_ms, &payload_json)
                    {
                        // Completed updates: clear any open permission card first.
                        if item.kind == "status" {
                            let pid = serde_json::from_str::<serde_json::Value>(&payload_json)
                                .ok()
                                .and_then(|v| {
                                    v.pointer("/properties/id")
                                        .or_else(|| v.get("id"))
                                        .and_then(|x| x.as_str())
                                        .map(str::to_owned)
                                });
                            if let Some(pid) = pid {
                                for existing in self.items.iter_mut().rev() {
                                    if existing.request_id.as_deref() == Some(pid.as_str())
                                        && existing.kind == "approval"
                                    {
                                        existing.kind = "status".into();
                                        existing.text = "Permission resolved".into();
                                        existing.request_id = None;
                                        break;
                                    }
                                }
                            }
                        }
                        self.items.push(item);
                    }
                } else if kind == "opencode/question.asked" {
                    if let Some(item) =
                        opencode_question_item_from_payload(seq, ts_ms, &payload_json)
                    {
                        self.items.push(item);
                    }
                } else if kind == "opencode/question.replied"
                    || kind == "opencode/question.rejected"
                {
                    // Resolve by clearing matching question cards (status line).
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload_json) {
                        let qid = value
                            .pointer("/properties/id")
                            .or_else(|| value.get("id"))
                            .and_then(|v| v.as_str());
                        if let Some(qid) = qid {
                            for item in self.items.iter_mut().rev() {
                                if item.request_id.as_deref() == Some(qid)
                                    && (item.kind == "question" || item.kind == "approval")
                                {
                                    item.kind = "status".into();
                                    item.text = "Question answered".into();
                                    item.request_id = None;
                                    item.options = None;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            UiEventMessage::ThreadTitleUpdated { .. } => {}
        }
    }
}

fn approval_item_from_payload(
    seq: u64,
    ts_ms: i64,
    payload_json: &str,
) -> Option<TranscriptItemDto> {
    let value: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let request_id = value.get("request_id")?.as_str()?.to_owned();
    let method = value
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("approval")
        .to_owned();
    let params = value
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    if method == "x.ai/ask_user_question" {
        return grok_question_item_from_params(seq, ts_ms, request_id, &params);
    }

    let (title, text, detail) = if method == "x.ai/exit_plan_mode" {
        // Keep the full plan body on the item (reviewers must not lose content).
        // Desktop `ApprovalModal` reveals it in coarse IncrementalText windows
        // so opening "View plan" does not paint a 50–200KB <pre> in one frame.
        let plan = params
            .get("planContent")
            .or_else(|| params.get("plan"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        (
            "Plan approval".to_owned(),
            "Grok finished a plan and needs approval to exit plan mode.".to_owned(),
            if plan.is_empty() {
                None
            } else {
                Some(plan.to_owned())
            },
        )
    } else if method == "session/request_permission" {
        let tool = params
            .get("toolCall")
            .and_then(|t| t.get("title").or_else(|| t.get("kind")))
            .and_then(|v| v.as_str())
            .or_else(|| params.get("title").and_then(|v| v.as_str()))
            .unwrap_or("tool");
        (
            "Permission required".to_owned(),
            format!("Agent requests permission: {tool}"),
            Some(truncate_str(&params.to_string(), 2000)),
        )
    } else if method == "item/tool/requestUserInput" {
        let questions = params
            .get("questions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let (prompt, options) = format_questions_prompt("Agent asks for input:", &questions);
        return Some(TranscriptItemDto {
            id: format!("question-{request_id}-{seq}"),
            kind: "question".into(),
            role: None,
            text: prompt,
            detail: None,
            title: Some("Agent question".into()),
            ts_ms,
            seq,
            message_id: None,
            request_id: Some(request_id),
            approval_method: Some(method),
            options,
            approve_response: None,
            decline_response: None,
        });
    } else {
        (
            "Approval required".to_owned(),
            format!("Agent is waiting for approval ({method})"),
            Some(truncate_str(&params.to_string(), 2000)),
        )
    };

    Some(TranscriptItemDto {
        id: format!("approval-{request_id}-{seq}"),
        kind: "approval".into(),
        role: None,
        text,
        detail,
        title: Some(title),
        ts_ms,
        seq,
        message_id: None,
        request_id: Some(request_id),
        approval_method: Some(method),
        options: None,
        approve_response: None,
        decline_response: None,
    })
}

fn grok_question_item_from_params(
    seq: u64,
    ts_ms: i64,
    request_id: String,
    params: &serde_json::Value,
) -> Option<TranscriptItemDto> {
    let questions = params
        .get("questions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let (prompt, options) = format_questions_prompt("Grok asks:", &questions);
    Some(TranscriptItemDto {
        id: format!("question-{request_id}-{seq}"),
        kind: "question".into(),
        role: None,
        text: prompt,
        detail: None,
        title: Some("Grok question".into()),
        ts_ms,
        seq,
        message_id: None,
        request_id: Some(request_id),
        approval_method: Some("x.ai/ask_user_question".into()),
        options,
        approve_response: None,
        decline_response: None,
    })
}

fn opencode_permission_item_from_payload(
    seq: u64,
    ts_ms: i64,
    payload_json: &str,
) -> Option<TranscriptItemDto> {
    let value: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let permission_id = value
        .pointer("/properties/id")
        .or_else(|| value.pointer("/properties/permissionID"))
        .or_else(|| value.pointer("/properties/permissionId"))
        .or_else(|| value.get("id"))
        .or_else(|| value.get("permissionID"))
        .and_then(|v| v.as_str())?
        .to_owned();
    // Completed permission updates are handled by the caller (clear card).
    let status = value
        .pointer("/properties/status")
        .or_else(|| value.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if matches!(
        status.to_ascii_lowercase().as_str(),
        "completed" | "resolved" | "done" | "accepted" | "rejected" | "denied"
    ) {
        return Some(TranscriptItemDto {
            id: format!("approval-oc-perm-done-{permission_id}-{seq}"),
            kind: "status".into(),
            role: None,
            text: "Permission resolved".into(),
            detail: None,
            title: Some("Permission".into()),
            ts_ms,
            seq,
            message_id: None,
            request_id: None,
            approval_method: Some("opencode/permission".into()),
            options: None,
            approve_response: None,
            decline_response: None,
        });
    }
    let title = value
        .pointer("/properties/title")
        .or_else(|| value.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("permission request");
    let description = value
        .pointer("/properties/description")
        .or_else(|| value.get("description"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = if description.is_empty() {
        format!("OpenCode asks for permission: {title}")
    } else {
        format!("OpenCode asks for permission: {title}\n{description}")
    };
    let (approve, decline) = opencode_permission_option_tokens(&value);
    Some(TranscriptItemDto {
        id: format!("approval-oc-perm-{permission_id}-{seq}"),
        kind: "approval".into(),
        role: None,
        text,
        detail: None,
        title: Some("Permission required".into()),
        ts_ms,
        seq,
        message_id: None,
        request_id: Some(permission_id),
        approval_method: Some("opencode/permission".into()),
        options: None,
        approve_response: Some(approve),
        decline_response: Some(decline),
    })
}

fn opencode_permission_option_tokens(value: &serde_json::Value) -> (String, String) {
    let mut approve = "accept".to_owned();
    let mut decline = "reject".to_owned();
    let options = value
        .pointer("/properties/options")
        .or_else(|| value.get("options"))
        .and_then(|v| v.as_array());
    if let Some(options) = options {
        for option in options {
            let label = option
                .get("kind")
                .or_else(|| option.get("name"))
                .or_else(|| option.get("label"))
                .or_else(|| option.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let id = option
                .get("optionId")
                .or_else(|| option.get("optionID"))
                .or_else(|| option.get("id"))
                .or_else(|| option.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if id.is_empty() {
                continue;
            }
            if label.contains("allow")
                || label.contains("approve")
                || label.contains("accept")
                || label.contains("yes")
            {
                approve = id.to_owned();
            } else if label.contains("reject")
                || label.contains("deny")
                || label.contains("decline")
                || label.contains("no")
            {
                decline = id.to_owned();
            }
        }
    }
    (approve, decline)
}

fn opencode_question_item_from_payload(
    seq: u64,
    ts_ms: i64,
    payload_json: &str,
) -> Option<TranscriptItemDto> {
    let value: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let props = value.get("properties").unwrap_or(&value);
    let question_id = props
        .get("id")
        .or_else(|| props.get("requestID"))
        .or_else(|| value.get("id"))
        .and_then(|v| v.as_str())?
        .to_owned();
    let questions = props
        .get("questions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let (prompt, options) = format_questions_prompt("OpenCode asks:", &questions);
    Some(TranscriptItemDto {
        id: format!("question-oc-{question_id}-{seq}"),
        kind: "question".into(),
        role: None,
        text: prompt,
        detail: None,
        title: Some("OpenCode question".into()),
        ts_ms,
        seq,
        message_id: None,
        request_id: Some(question_id),
        approval_method: Some("opencode/question".into()),
        options,
        approve_response: None,
        decline_response: None,
    })
}

/// Build a human prompt and optional single-question option list for UI chips.
fn format_questions_prompt(
    prefix: &str,
    questions: &[serde_json::Value],
) -> (String, Option<Vec<TranscriptOptionDto>>) {
    if questions.is_empty() {
        return (
            format!("{prefix}\nReply with your answer."),
            None,
        );
    }
    let mut lines = vec![prefix.to_owned()];
    let mut single_options: Option<Vec<TranscriptOptionDto>> = None;
    for (i, q) in questions.iter().enumerate() {
        let header = q
            .get("header")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let text = q
            .get("question")
            .or_else(|| q.get("text"))
            .or_else(|| q.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("Question");
        if header.is_empty() {
            lines.push(format!("{}. {text}", i + 1));
        } else {
            lines.push(format!("{}. [{header}] {text}", i + 1));
        }
        let opts: Vec<TranscriptOptionDto> = q
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        let label = o
                            .get("label")
                            .or_else(|| o.get("value"))
                            .or_else(|| o.get("id"))
                            .and_then(|v| v.as_str())?
                            .trim()
                            .to_owned();
                        if label.is_empty() {
                            return None;
                        }
                        let description = o
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        Some(TranscriptOptionDto { label, description })
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (j, opt) in opts.iter().enumerate() {
            let desc = opt
                .description
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default();
            lines.push(format!("   {}) {}{desc}", j + 1, opt.label));
        }
        if questions.len() == 1 && !opts.is_empty() {
            single_options = Some(opts);
        }
    }
    (lines.join("\n"), single_options)
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

/// Bare target for a tool header (path/cmd/pattern — no `file=` labels).
/// Agent-agnostic: consumes unified translator names (`"read: path"`, …).
/// Mirrors minos-tui `translation/tool_summary.rs` essentials.
fn summarize_tool_args(tool_name: &str, args_json: &str) -> String {
    let subject_fallback = tool_subject_from_name(tool_name);
    let Some(value) = parse_tool_args_json(args_json) else {
        if subject_fallback != tool_name.trim() && !subject_fallback.is_empty() {
            return truncate_str(&one_line(subject_fallback), 120);
        }
        return truncate_str(&one_line(args_json), 180);
    };
    if value.is_null() {
        if subject_fallback != tool_name.trim() && !subject_fallback.is_empty() {
            return truncate_str(&one_line(subject_fallback), 120);
        }
        return String::new();
    }

    let kind = ToolKind::from_tool_name(tool_name);
    match kind {
        ToolKind::Read | ToolKind::Edit | ToolKind::List => {
            if let Some(path) = find_tool_path(&value) {
                return truncate_str(&one_line(&path), 120);
            }
        }
        ToolKind::Execute => {
            if let Some(cmd) = find_tool_stringish(&value, &["cmd", "command", "script", "shell"]) {
                return truncate_str(&one_line(&cmd), 120);
            }
        }
        ToolKind::Search => {
            let pattern = find_tool_stringish(
                &value,
                &["pattern", "query", "regex", "search", "grep", "needle"],
            );
            let path = find_tool_path(&value);
            return match (pattern, path) {
                (Some(p), Some(path)) => {
                    truncate_str(&format!("{} in {}", one_line(&p), one_line(&path)), 140)
                }
                (Some(p), None) => truncate_str(&one_line(&p), 120),
                (None, Some(path)) => truncate_str(&one_line(&path), 120),
                (None, None) => String::new(),
            };
        }
        ToolKind::WebSearch | ToolKind::WebFetch => {
            if let Some(q) = find_tool_stringish(&value, &["query", "url", "uri", "href", "q"]) {
                return truncate_str(&one_line(&q), 120);
            }
        }
        ToolKind::Skill => {
            if let Some(skill) = find_tool_stringish(
                &value,
                &[
                    "skill",
                    "skill_name",
                    "skillName",
                    "name",
                    "skill_path",
                    "skillPath",
                ],
            ) {
                return truncate_str(&one_line(&skill), 90);
            }
        }
        ToolKind::Other => {}
    }

    if tool_name.to_ascii_lowercase().contains("task")
        || tool_name.eq_ignore_ascii_case("todo")
        || tool_name.eq_ignore_ascii_case("todowrite")
        || tool_name.eq_ignore_ascii_case("todo_write")
    {
        if let Some(task) = find_tool_stringish(
            &value,
            &[
                "task",
                "description",
                "prompt",
                "instructions",
                "question",
                "subagent_type",
                "subagentType",
            ],
        ) {
            return truncate_str(&one_line(&task), 110);
        }
    }

    if let Some(path) = find_tool_path(&value) {
        return truncate_str(&one_line(&path), 120);
    }
    if let Some(cmd) = find_tool_stringish(&value, &["cmd", "command", "script", "shell"]) {
        return truncate_str(&one_line(&cmd), 120);
    }
    if let Some(desc) = find_tool_stringish(&value, &["description", "task", "prompt", "query"]) {
        return truncate_str(&one_line(&desc), 120);
    }
    if subject_fallback != tool_name.trim() && !subject_fallback.is_empty() {
        return truncate_str(&one_line(subject_fallback), 120);
    }

    compact_tool_args_json(args_json).unwrap_or_default()
}

/// Subject after unified `"kind: subject"` tool name, else full name.
fn tool_subject_from_name(name: &str) -> &str {
    if let Some((prefix, rest)) = name.split_once(':') {
        let token = prefix.trim().to_ascii_lowercase();
        if matches!(
            token.as_str(),
            "read"
                | "read_file"
                | "edit"
                | "write"
                | "diff"
                | "execute"
                | "terminal"
                | "bash"
                | "shell"
                | "run"
                | "search"
                | "grep"
                | "list"
                | "list_dir"
                | "web_fetch"
                | "web_search"
                | "fetch"
                | "skill"
                | "other"
        ) {
            let subject = rest.trim();
            if !subject.is_empty() {
                return subject;
            }
        }
    }
    name.trim()
}

fn summarize_tool_output_line(out: &str, is_error: bool) -> String {
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return if is_error {
            "Tool failed".into()
        } else {
            "Tool finished".into()
        };
    }
    if is_diff_like(trimmed) {
        let (add, del) = count_diff_lines(trimmed);
        return format!("+{add}/-{del}");
    }
    let one = trimmed
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    truncate_str(&one_line(one), 100)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Read,
    Edit,
    Execute,
    Search,
    List,
    WebFetch,
    WebSearch,
    Skill,
    Other,
}

impl ToolKind {
    /// Same classification as TUI: prefer unified `"kind: subject"` prefix.
    fn from_tool_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if let Some((prefix, _)) = n.split_once(':') {
            if let Some(kind) = Self::from_kind_token(prefix.trim()) {
                return kind;
            }
        }
        if let Some(kind) = Self::from_kind_token(n.trim()) {
            return kind;
        }
        if n.contains("skill") {
            return Self::Skill;
        }
        if n.contains("web_search") || n == "websearch" {
            return Self::WebSearch;
        }
        if n.contains("web_fetch") || n.contains("webfetch") || n == "fetch" {
            return Self::WebFetch;
        }
        if n.contains("list_dir")
            || n.contains("listdir")
            || n.contains("list_directory")
            || n == "ls"
            || n.contains("glob_file")
        {
            return Self::List;
        }
        // Edit before search: names like `search_replace` contain "search".
        if n.contains("write")
            || n.contains("edit")
            || n.contains("apply_patch")
            || n.contains("str_replace")
            || n.contains("search_replace")
            || n.contains("create_file")
            || n.contains("delete_file")
        {
            return Self::Edit;
        }
        if n.contains("grep")
            || n.contains("search")
            || n.contains("glob")
            || n.contains("find")
            || n.contains("rg")
        {
            return Self::Search;
        }
        if n.contains("read") || n == "cat" || n.ends_with("_read") {
            return Self::Read;
        }
        if n.contains("bash")
            || n.contains("shell")
            || n.contains("exec")
            || n.contains("command")
            || n == "run_terminal_command"
            || n == "run"
        {
            return Self::Execute;
        }
        Self::Other
    }

    fn from_kind_token(token: &str) -> Option<Self> {
        match token {
            "read" | "read_file" | "readfile" | "cat" => Some(Self::Read),
            "edit" | "write" | "diff" | "search_replace" | "apply_patch" | "str_replace" => {
                Some(Self::Edit)
            }
            "execute" | "terminal" | "bash" | "shell" | "run" | "command" => Some(Self::Execute),
            "search" | "grep" | "glob" | "find" | "rg" => Some(Self::Search),
            "list" | "list_dir" | "listdir" | "list_directory" | "ls" => Some(Self::List),
            "web_fetch" | "webfetch" | "fetch" => Some(Self::WebFetch),
            "web_search" | "websearch" => Some(Self::WebSearch),
            "skill" => Some(Self::Skill),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

fn parse_tool_args_json(args_json: &str) -> Option<serde_json::Value> {
    let trimmed = args_json.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn compact_tool_args_json(args_json: &str) -> Option<String> {
    let value = parse_tool_args_json(args_json)?;
    if value.is_null() {
        return Some(String::new());
    }
    serde_json::to_string(&value)
        .ok()
        .map(|text| truncate_str(&one_line(&text), 500))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_diff_like(text: &str) -> bool {
    text.contains("diff --git")
        || text.contains("\n@@")
        || text.starts_with("@@")
        || text.contains("*** Begin Patch")
        || text.contains("*** Update File:")
        || text.contains("*** Add File:")
        || text.contains("*** Delete File:")
        || text.contains("*** End Patch")
        || text
            .lines()
            .any(|line| line.starts_with("+++ ") || line.starts_with("--- "))
}

fn count_diff_lines(text: &str) -> (usize, usize) {
    let add = text
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .count();
    let del = text
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("---"))
        .count();
    (add, del)
}

fn find_tool_path(value: &serde_json::Value) -> Option<String> {
    find_tool_stringish(
        value,
        &[
            "file_path",
            "filePath",
            "filepath",
            "path",
            "absolute_path",
            "absolutePath",
            "relative_path",
            "relativePath",
            "target_file",
            "targetFile",
            "file",
            "uri",
        ],
    )
}

fn find_tool_stringish(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    find_tool_stringish_inner(value, keys, 0)
}

fn find_tool_stringish_inner(
    value: &serde_json::Value,
    keys: &[&str],
    depth: usize,
) -> Option<String> {
    if depth > 4 {
        return None;
    }
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_to_summary_text) {
                    return Some(found);
                }
            }
            for child_key in [
                "input",
                "args",
                "arguments",
                "params",
                "tool_input",
                "toolInput",
                "metadata",
                "state",
            ] {
                if let Some(found) = map
                    .get(child_key)
                    .and_then(|child| find_tool_stringish_inner(child, keys, depth + 1))
                {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|child| find_tool_stringish_inner(child, keys, depth + 1))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_tool_stringish_inner(child, keys, depth + 1)),
        _ => None,
    }
}

fn value_to_summary_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => (!text.trim().is_empty()).then(|| text.to_owned()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
        serde_json::Value::Array(items) => {
            let values = items
                .iter()
                .filter_map(value_to_summary_text)
                .collect::<Vec<_>>();
            (!values.is_empty()).then(|| values.join(","))
        }
        serde_json::Value::Object(map) => {
            for key in [
                "name",
                "path",
                "file_path",
                "filePath",
                "description",
                "task",
                "prompt",
            ] {
                if let Some(text) = map.get(key).and_then(value_to_summary_text) {
                    return Some(text);
                }
            }
            None
        }
        serde_json::Value::Null => None,
    }
}

fn agent_label(agent: AgentName) -> String {
    match agent {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
        AgentName::Gemini => "gemini",
        AgentName::Opencode => "opencode",
        AgentName::Grok => "grok",
    }
    .into()
}

fn thread_status_label(t: &ThreadSummary) -> String {
    use minos_protocol::{PauseReason, ThreadState};
    if t.ended_at_ms.is_some() {
        return "done".into();
    }
    match &t.state {
        ThreadState::Starting | ThreadState::Resuming => "running".into(),
        ThreadState::Idle => "idle".into(),
        ThreadState::Running { .. } => "running".into(),
        // Between-turn daemon death was historically stored as
        // Suspended{DaemonRestart} + needs_continue=0. That is not a user pause —
        // show Idle (ready to reattach). Mid-flight recovery keeps Paused.
        ThreadState::Suspended {
            reason: PauseReason::DaemonRestart,
        } if !t.needs_continue => "idle".into(),
        ThreadState::Suspended { .. } => "suspended".into(),
        ThreadState::Closed { .. } => "done".into(),
    }
}

fn relative_time(ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(ms);
    let delta = (now - ms).max(0);
    let secs = delta / 1000;
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 48 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

/// Fallback wire string only. UI formats `created_at_ms` in the local timezone.
fn clock_time(ms: i64) -> String {
    let secs = ms / 1000;
    let mins_total = secs / 60;
    let minutes = mins_total.rem_euclid(60);
    let hours = (mins_total / 60).rem_euclid(24);
    format!("{hours:02}:{minutes:02}")
}

fn pump_still_current(my_gen: u64, flag: &AtomicU64) -> bool {
    flag.load(Ordering::SeqCst) == my_gen
}

fn thread_state_to_status(state: &ThreadState) -> String {
    match state {
        ThreadState::Starting | ThreadState::Resuming | ThreadState::Running { .. } => {
            "running".into()
        }
        ThreadState::Idle => "idle".into(),
        ThreadState::Suspended { .. } => "suspended".into(),
        ThreadState::Closed { .. } => "done".into(),
    }
}

fn map_manager_event(ev: LocalManagerEvent) -> ManagerEventDto {
    match ev {
        LocalManagerEvent::ThreadAdded {
            thread_id,
            workspace,
            agent,
            parent_thread_id,
        } => ManagerEventDto::ThreadAdded {
            thread_id,
            agent: agent_label(agent),
            parent_thread_id,
            workspace,
        },
        LocalManagerEvent::ThreadStateChanged {
            thread_id,
            new,
            at_ms,
            ..
        } => ManagerEventDto::ThreadStateChanged {
            thread_id,
            status: thread_state_to_status(&new),
            at_ms,
        },
        LocalManagerEvent::ThreadClosed { thread_id, .. } => {
            ManagerEventDto::ThreadClosed { thread_id }
        }
        LocalManagerEvent::InstanceCrashed {
            affected_threads, ..
        } => ManagerEventDto::InstanceCrashed {
            affected_thread_ids: affected_threads,
        },
    }
}

fn frame_to_ingest_dto(frame: LocalIngestFrame) -> IngestEventDto {
    let mut assembler = TranscriptAssembler::new(frame.thread_id.clone());
    assembler.ingest_frame(frame.seq, frame.ts_ms, frame.ui_events);
    let items = assembler.finish();
    let has_pending_approval = items
        .iter()
        .any(|it| {
            (it.kind == "approval" || it.kind == "question") && it.request_id.is_some()
        });
    IngestEventDto {
        thread_id: frame.thread_id,
        seq: frame.seq,
        agent: agent_label(frame.agent),
        ts_ms: frame.ts_ms,
        items,
        has_pending_approval,
    }
}

fn spawn_ingest_pump(app: AppHandle, client: Arc<WsClient>, my_gen: u64, gen_flag: Arc<AtomicU64>) {
    tokio::spawn(async move {
        let sub = match client
            .subscribe::<LocalIngestFrame, ArrayParams>(
                "minos_local_subscribe_ingest",
                ArrayParams::new(),
                "minos_local_unsubscribe_ingest",
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    target: "minos_desktop",
                    error = %e,
                    "ingest subscription failed"
                );
                return;
            }
        };
        let mut stream = sub.into_stream();
        while pump_still_current(my_gen, &gen_flag) {
            match stream.next().await {
                Some(Ok(frame)) => {
                    if !pump_still_current(my_gen, &gen_flag) {
                        break;
                    }
                    let dto = frame_to_ingest_dto(frame);
                    if let Err(e) = app.emit(EVENT_INGEST, &dto) {
                        warn!(
                            target: "minos_desktop",
                            error = %e,
                            "emit daemon://ingest failed"
                        );
                    }
                }
                Some(Err(e)) => {
                    warn!(
                        target: "minos_desktop",
                        error = %e,
                        "ingest subscription error"
                    );
                    break;
                }
                None => {
                    warn!(target: "minos_desktop", "ingest subscription ended");
                    break;
                }
            }
        }
    });
}

fn spawn_manager_pump(
    app: AppHandle,
    client: Arc<WsClient>,
    my_gen: u64,
    gen_flag: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let sub = match client
            .subscribe::<LocalManagerEvent, ArrayParams>(
                "minos_local_subscribe_manager_events",
                ArrayParams::new(),
                "minos_local_unsubscribe_manager_events",
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    target: "minos_desktop",
                    error = %e,
                    "manager subscription failed"
                );
                return;
            }
        };
        let mut stream = sub.into_stream();
        while pump_still_current(my_gen, &gen_flag) {
            match stream.next().await {
                Some(Ok(event)) => {
                    if !pump_still_current(my_gen, &gen_flag) {
                        break;
                    }
                    let dto = map_manager_event(event);
                    if let Err(e) = app.emit(EVENT_MANAGER, &dto) {
                        warn!(
                            target: "minos_desktop",
                            error = %e,
                            "emit daemon://manager failed"
                        );
                    }
                }
                Some(Err(e)) => {
                    warn!(
                        target: "minos_desktop",
                        error = %e,
                        "manager subscription error"
                    );
                    break;
                }
                None => {
                    warn!(target: "minos_desktop", "manager subscription ended");
                    break;
                }
            }
        }
    });
}

fn spawn_conversation_pump(
    app: AppHandle,
    client: Arc<WsClient>,
    my_gen: u64,
    gen_flag: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let sub = match client
            .subscribe::<LocalConversationEvent, ArrayParams>(
                "minos_local_subscribe_conversation_events",
                ArrayParams::new(),
                "minos_local_unsubscribe_conversation_events",
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    target: "minos_desktop",
                    error = %e,
                    "conversation subscription failed"
                );
                return;
            }
        };
        let mut stream = sub.into_stream();
        while pump_still_current(my_gen, &gen_flag) {
            match stream.next().await {
                Some(Ok(LocalConversationEvent::ConversationMessageAppended {
                    conversation_id,
                    message_seq,
                })) => {
                    if !pump_still_current(my_gen, &gen_flag) {
                        break;
                    }
                    let dto = ConversationEventDto {
                        conversation_id,
                        message_seq,
                    };
                    if let Err(e) = app.emit(EVENT_CONVERSATION, &dto) {
                        warn!(
                            target: "minos_desktop",
                            error = %e,
                            "emit daemon://conversation failed"
                        );
                    }
                }
                Some(Err(e)) => {
                    warn!(
                        target: "minos_desktop",
                        error = %e,
                        "conversation subscription error"
                    );
                    break;
                }
                None => {
                    warn!(
                        target: "minos_desktop",
                        "conversation subscription ended"
                    );
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tool_present_tests {
    use super::*;

    #[test]
    fn summarizes_read_path() {
        let target = summarize_tool_args("read_file", r#"{"file_path":"src/main.rs"}"#);
        assert_eq!(target, "src/main.rs");
    }

    #[test]
    fn summarizes_execute_command() {
        let target = summarize_tool_args("run_terminal_command", r#"{"command":"cargo test"}"#);
        assert_eq!(target, "cargo test");
    }

    #[test]
    fn tool_kind_classifies_edit() {
        assert_eq!(ToolKind::from_tool_name("search_replace"), ToolKind::Edit);
    }
}

#[cfg(test)]
mod thread_status_label_tests {
    use super::*;
    use minos_domain::AgentName;
    use minos_protocol::{PauseReason, ThreadState, ThreadSummary};

    fn summary(state: ThreadState, needs_continue: bool) -> ThreadSummary {
        ThreadSummary {
            thread_id: "t1".into(),
            agent: AgentName::Codex,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            end_reason: None,
            parent_thread_id: None,
            state,
            needs_continue,
        }
    }

    #[test]
    fn daemon_restart_without_needs_continue_displays_idle() {
        let s = summary(
            ThreadState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            false,
        );
        assert_eq!(thread_status_label(&s), "idle");
    }

    #[test]
    fn daemon_restart_with_needs_continue_displays_suspended() {
        let s = summary(
            ThreadState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            true,
        );
        assert_eq!(thread_status_label(&s), "suspended");
    }

    #[test]
    fn user_interrupt_displays_suspended_even_without_needs_continue() {
        let s = summary(
            ThreadState::Suspended {
                reason: PauseReason::UserInterrupt,
            },
            false,
        );
        assert_eq!(thread_status_label(&s), "suspended");
    }
}

#[cfg(test)]
mod conversation_message_kind_tests {
    use super::*;

    #[test]
    fn conversation_timeline_does_not_infer_approval_from_body_text() {
        // Agent prose often mentions "approval" / plans without being a reverse-request.
        let bodies = [
            "Permission: apply_patch → src/foo.rs",
            "needs approval to exit plan mode",
            "Updating architecture docs with the plan-approval full-content contract",
            "Regression tests: `plan_approval_keeps_full_plan_content`",
            "Normal agent result with no special tokens",
        ];
        for body in bodies {
            assert_eq!(
                conversation_timeline_kind(body),
                "text",
                "body={body:?} must stay text on conversation timeline"
            );
        }
    }
}

#[cfg(test)]
mod approval_item_tests {
    use super::*;

    #[test]
    fn plan_approval_keeps_full_plan_content() {
        // Regression: previously truncated at 6000 bytes with "…", so "View plan"
        // could not show the complete plan document.
        let long_plan = format!("{}\n## Tail section that must remain", "x".repeat(7000));
        let payload = serde_json::json!({
            "request_id": "req-plan-1",
            "method": "x.ai/exit_plan_mode",
            "params": {
                "planContent": long_plan,
            }
        });
        let item = approval_item_from_payload(1, 1_700_000_000_000, &payload.to_string())
            .expect("approval item");
        assert_eq!(item.kind, "approval");
        assert_eq!(item.approval_method.as_deref(), Some("x.ai/exit_plan_mode"));
        let detail = item.detail.expect("plan detail");
        assert_eq!(detail, long_plan);
        assert!(!detail.ends_with('…'));
        assert!(detail.contains("## Tail section that must remain"));
    }

    #[test]
    fn permission_approval_still_truncates_large_params() {
        let big = "y".repeat(3000);
        let payload = serde_json::json!({
            "request_id": "req-perm-1",
            "method": "session/request_permission",
            "params": {
                "toolCall": { "title": "write_file", "kind": "edit" },
                "blob": big,
            }
        });
        let item = approval_item_from_payload(2, 1_700_000_000_001, &payload.to_string())
            .expect("approval item");
        let detail = item.detail.expect("permission detail");
        assert!(detail.ends_with('…'));
        assert!(detail.chars().count() <= 2001);
    }
}
