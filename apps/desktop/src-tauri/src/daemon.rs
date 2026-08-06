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
    LocalConversationEvent, LocalIngestFrame, LocalManagerEvent, ReadSessionRawHistoryResponse,
};
use minos_protocol::{
    AppendConversationMessageParams, ApprovalDecisionRequest, CreateConversationParams,
    CreateProjectRequest, HostApplyLinkTokenParams, HostApplyLinkTokenResponse,
    HostPrepareLinkResponse, HostSignLinkProofParams, HostSignLinkProofResponse, ListClisResponse,
    ListConversationAgentSessionsParams, ListConversationMessagesParams, ListConversationsParams,
    ListProjectsResponse, LocalConversationMessage, LocalConversationSummary, LocalReactionGroup,
    ProjectSummary, ReadSessionParams, RemoveConversationAgentParams, SendUserMessageRequest,
    SessionState, SessionSummary, StartAgentInConversationRequest, StartAgentResponse,
    ToggleConversationMessageReactionParams, ToggleConversationMessageReactionResponse,
    UpdateConversationParams,
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

/// Frontend event channels (push path, TUI-parity).
pub const EVENT_INGEST: &str = "daemon://ingest";
pub const EVENT_MANAGER: &str = "daemon://manager";
pub const EVENT_CONVERSATION: &str = "daemon://conversation";
/// Live push health: pumps arm → live=true; any current-gen pump ends → live=false.
pub const EVENT_PUSH_STATUS: &str = "daemon://push-status";

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
    /// IM **device online**: managed daemon has live `/ws/host` to the hub.
    /// False when disconnected, connecting, or non-managed external daemon
    /// without a handle (cannot observe relay link).
    #[serde(default)]
    pub hub_online: bool,
    /// Local `hit_` present (host can dial hub). Not a product "Link" flag.
    #[serde(default)]
    pub has_host_token: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_dirty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_head: Option<String>,
    pub running_count: u32,
    pub approval_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveConversationAgentDto {
    pub conversation: ConversationDto,
    pub closed_session_ids: Vec<String>,
    pub cancelled_delegation_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionActorDto {
    pub actor_id: String,
    pub actor_kind: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionGroupDto {
    pub emoji: String,
    pub count: u32,
    pub reacted_by_me: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<ReactionActorDto>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<ReactionGroupDto>,
    /// Structured git milestone when present (worktree / PR / commits…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_activity: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusDto {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_head: Option<String>,
    pub dirty: bool,
    pub has_untracked: bool,
    pub ahead_count: u32,
    pub behind_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub is_linked_worktree: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ConversationDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleReactionResultDto {
    pub message_id: String,
    pub conversation_id: String,
    pub reactions: Vec<ReactionGroupDto>,
}

/// One page of conversation messages (tail or older).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePageDto {
    pub messages: Vec<MessageDto>,
    /// True when more messages exist before this page (older history).
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionDto {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_short_id: Option<String>,
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
    /// assistant | user | tool | tool_result | tool_error | reasoning | status | error | approval | question | subagent
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
    pub session_id: String,
    pub items: Vec<TranscriptItemDto>,
    pub next_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliDto {
    pub agent: String,
    pub display_name: String,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub supports_model_selection: bool,
    pub supports_reasoning_effort: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAgentResultDto {
    pub session_id: String,
    pub cwd: String,
}

/// Live ingest delta for the webview (assembled transcript items + approval hint).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestEventDto {
    pub session_id: String,
    pub seq: u64,
    pub agent: String,
    pub ts_ms: i64,
    pub items: Vec<TranscriptItemDto>,
    pub has_pending_approval: bool,
}

/// Manager lifecycle event (session status) for the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ManagerEventDto {
    SessionAdded {
        session_id: String,
        agent: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_session_id: Option<String>,
        workspace: String,
    },
    SessionStateChanged {
        session_id: String,
        /// idle | running | suspended | done
        status: String,
        at_ms: i64,
    },
    SessionClosed {
        session_id: String,
    },
    InstanceCrashed {
        affected_session_ids: Vec<String>,
    },
}

/// Conversation timeline push events (message append or reaction toggle).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ConversationEventDto {
    MessageAppended {
        conversation_id: String,
        message_seq: i64,
    },
    ReactionToggled {
        conversation_id: String,
        message_id: String,
        reactions: Vec<ReactionGroupDto>,
    },
    RosterChanged {
        conversation_id: String,
        members: Vec<RosterMemberDto>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterMemberDto {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    pub joined_at_ms: i64,
}

/// Whether JSON-RPC subscription pumps are currently healthy for the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushStatusDto {
    pub live: bool,
}

/// Host Link prepare material (D02 §7.2 / daemon `host_prepare_link`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPrepareLinkDto {
    pub installation_id: String,
    pub public_key: String,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSignLinkProofDto {
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostApplyLinkTokenDto {
    pub linked: bool,
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
        let hub_online = guard.managed.as_ref().is_some_and(|h| {
            matches!(
                h.current_relay_link(),
                minos_domain::RelayLinkState::Connected
            )
        });
        let has_host_token = minos_daemon::device_secret_store::read()
            .ok()
            .flatten()
            .is_some();
        ConnectionDto {
            connected: guard.client.is_some(),
            endpoint: guard.endpoint.clone(),
            error: guard.last_error.clone(),
            source: guard.source.clone(),
            managed: guard.managed.is_some(),
            hub_online,
            has_host_token,
        }
    }

    /// Graceful teardown of a managed in-process daemon. Provider children
    /// (including OpenCode `serve`) are killed via `DaemonHandle::stop` →
    /// `shutdown_instances`. Call on app exit so processes are not reparented
    /// to launchd and left holding ports 4096..=4106.
    /// Stop the managed in-process daemon. Returns `Err` when stop fails so
    /// updater prepare can refuse to install over live children.
    ///
    /// After success the bridge owns neither a client nor a managed handle;
    /// a later [`Self::connect`] starts a fresh managed daemon (used by
    /// `restore_after_failed_update`).
    pub async fn shutdown_managed(&self) -> Result<(), String> {
        // Drop the WS client first so subscription pumps exit promptly.
        let managed = {
            let mut guard = self.inner.lock().await;
            guard.client = None;
            guard.endpoint = None;
            guard.managed.take()
        };
        // Invalidate discovery so a concurrent connect does not attach to a
        // port that is about to close (or already closed).
        remove_discovery_file();
        // Bump pump generation so any lingering pump tasks exit.
        self.pump_generation.fetch_add(1, Ordering::SeqCst);

        if let Some(handle) = managed {
            match handle.stop().await {
                Ok(()) => {
                    info!(target: "minos_desktop", "managed daemon stopped");
                    Ok(())
                }
                Err(e) => {
                    warn!(
                        target: "minos_desktop",
                        error = %e,
                        "managed daemon stop failed"
                    );
                    Err(e.to_string())
                }
            }
        } else {
            Ok(())
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

        // Optimistic live=true so frontend can drop degraded polls; a pump that
        // fails to subscribe or later ends will emit live=false for this gen.
        emit_push_status(&app, true);

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

    /// List conversation messages (newest-first from daemon, returned ASC).
    /// `before_seq` pages older history; `limit` defaults to 80 (max 500).
    pub async fn list_messages(
        &self,
        conversation_id: String,
        before_seq: Option<i64>,
        limit: Option<u32>,
    ) -> Result<MessagePageDto> {
        let client = self.client().await?;
        let page_limit = limit.unwrap_or(80).clamp(1, 500);
        let params = ListConversationMessagesParams {
            conversation_id,
            before_seq,
            limit: Some(page_limit),
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
        Ok(MessagePageDto {
            messages,
            has_more: response.has_more,
        })
    }

    /// Idempotent toggle of the host local user's reaction on a chat message.
    pub async fn toggle_message_reaction(
        &self,
        message_id: String,
        emoji: String,
    ) -> Result<ToggleReactionResultDto> {
        let client = self.client().await?;
        let params = ToggleConversationMessageReactionParams { message_id, emoji };
        let response: ToggleConversationMessageReactionResponse = client
            .request("minos_local_toggle_conversation_message_reaction", [params])
            .await
            .context("minos_local_toggle_conversation_message_reaction")?;
        Ok(ToggleReactionResultDto {
            message_id: response.message_id,
            conversation_id: response.conversation_id,
            reactions: response
                .reactions
                .into_iter()
                .map(map_reaction_group)
                .collect(),
        })
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
            .sessions
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
        session_id: String,
        from_seq: Option<u64>,
        limit: Option<u32>,
        full: bool,
    ) -> Result<TranscriptPageDto> {
        let client = self.client().await?;
        // One assembler across all pages so TextDelta that spans RPC pages
        // still merges into a single transcript item.
        let mut assembler = TranscriptAssembler::new(session_id.clone());
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
            let req = ReadSessionParams {
                session_id: session_id.clone(),
                from_seq: cursor,
                limit: page_limit,
            };
            let response: ReadSessionRawHistoryResponse = client
                .request("minos_local_read_session_raw_history", [req])
                .await
                .context("minos_local_read_session_raw_history")?;
            for frame in response.events {
                assembler.ingest_frame(frame.seq, frame.ts_ms, frame.ui_events);
            }
            pages += 1;
            if !full {
                return Ok(TranscriptPageDto {
                    session_id,
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
                    session_id,
                    items: assembler.finish(),
                    next_seq: Some(next),
                });
            }
            // Daemon from_seq is exclusive; next_seq is the next start seq → from = next - 1.
            cursor = Some(next.saturating_sub(1));
        }
        Ok(TranscriptPageDto {
            session_id,
            items: assembler.finish(),
            next_seq: None,
        })
    }

    pub async fn create_conversation(
        &self,
        project_id: String,
        title: String,
        priority: Option<String>,
        agents: Vec<(String, Option<String>)>,
        git_mode: Option<String>,
    ) -> Result<ConversationDto> {
        let client = self.client().await?;
        let params = CreateConversationParams {
            project_id,
            title,
            priority,
            agents: agents
                .into_iter()
                .map(|(agent, brief)| minos_protocol::ConversationAgentSpec {
                    agent,
                    brief: brief.filter(|b| !b.trim().is_empty()),
                })
                .collect(),
            git_mode,
        };
        let response: minos_protocol::CreateConversationResponse = client
            .request("minos_local_create_conversation", [params])
            .await
            .context("minos_local_create_conversation")?;
        Ok(map_conversation(response.conversation))
    }

    pub async fn git_get_status(
        &self,
        conversation_id: String,
        refresh_conversation: bool,
    ) -> Result<GitStatusDto> {
        let client = self.client().await?;
        let params = minos_protocol::GitStatusParams {
            conversation_id: Some(conversation_id),
            project_id: None,
            path: None,
            refresh_conversation,
        };
        let response: minos_protocol::GitStatusResponse = client
            .request("minos_local_git_get_status", [params])
            .await
            .context("minos_local_git_get_status")?;
        Ok(GitStatusDto {
            path: response.path,
            branch: response.branch,
            head: response.head,
            short_head: response.short_head,
            dirty: response.dirty,
            has_untracked: response.has_untracked,
            ahead_count: response.ahead_count,
            behind_count: response.behind_count,
            upstream: response.upstream,
            is_linked_worktree: response.is_linked_worktree,
            conversation: response.conversation.map(map_conversation),
        })
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

    pub async fn remove_conversation_agent(
        &self,
        conversation_id: String,
        agent: String,
    ) -> Result<RemoveConversationAgentDto> {
        let client = self.client().await?;
        let params = RemoveConversationAgentParams {
            conversation_id,
            agent,
        };
        let response: minos_protocol::RemoveConversationAgentResponse = client
            .request("minos_local_remove_conversation_agent", [params])
            .await
            .context("minos_local_remove_conversation_agent")?;
        Ok(RemoveConversationAgentDto {
            conversation: map_conversation(response.conversation),
            closed_session_ids: response.closed_session_ids,
            cancelled_delegation_ids: response.cancelled_delegation_ids,
        })
    }

    pub async fn append_user_message(
        &self,
        conversation_id: String,
        message_id: String,
        body: String,
    ) -> Result<i64> {
        let client = self.client().await?;
        let params = AppendConversationMessageParams {
            conversation_id,
            message_id,
            session_id: None,
            sender_role: "user".into(),
            agent: None,
            body,
            reply_to_message_id: None,
            delegation_id: None,
            mentions: vec![],
        };
        let response: minos_protocol::AppendConversationMessageResponse = client
            .request("minos_local_append_conversation_message", [params])
            .await
            .context("minos_local_append_conversation_message")?;
        Ok(response.message_seq)
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
                    display_name: d.display_name,
                    installed,
                    path: d.path,
                    version: d.version,
                    status,
                    supports_model_selection: d.supports_model_selection,
                    supports_reasoning_effort: d.supports_reasoning_effort,
                }
            })
            .collect())
    }

    pub async fn start_agent_in_conversation(
        &self,
        conversation_id: String,
        agent: String,
        workspace: String,
        profile_id: Option<String>,
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
            profile_id,
            model,
            reasoning_effort,
            instructions,
        };
        let response: StartAgentResponse = client
            .request("minos_local_start_agent_in_conversation", [req])
            .await
            .context("minos_local_start_agent_in_conversation")?;
        Ok(StartAgentResultDto {
            session_id: response.session_id,
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

    pub async fn send_user_message(
        &self,
        session_id: String,
        text: String,
        origin_message_id: Option<String>,
    ) -> Result<()> {
        let client = self.client().await?;
        let req = SendUserMessageRequest {
            session_id,
            text,
            origin_message_id,
            attachments: vec![],
        };
        client
            .request::<(), _>("minos_local_send_user_message", [req])
            .await
            .context("minos_local_send_user_message")?;
        Ok(())
    }

    /// Host Link: fetch host installation identity + bootstrap nonce.
    pub async fn host_prepare_link(&self) -> Result<HostPrepareLinkDto> {
        let client = self.client().await?;
        let response: HostPrepareLinkResponse = client
            .request("minos_local_host_prepare_link", ArrayParams::new())
            .await
            .context("minos_local_host_prepare_link")?;
        Ok(HostPrepareLinkDto {
            installation_id: response.installation_id,
            public_key: response.public_key,
            nonce: response.nonce,
        })
    }

    /// Host Link: Ed25519 sign `"{installation_id}:{nonce}:v1/hosts/link"`.
    pub async fn host_sign_link_proof(
        &self,
        installation_id: String,
        nonce: String,
    ) -> Result<HostSignLinkProofDto> {
        let client = self.client().await?;
        let req = HostSignLinkProofParams {
            installation_id,
            nonce,
        };
        let response: HostSignLinkProofResponse = client
            .request("minos_local_host_sign_link_proof", [req])
            .await
            .context("minos_local_host_sign_link_proof")?;
        Ok(HostSignLinkProofDto {
            signature: response.signature,
        })
    }

    /// Host Link: persist `hit_*` and wake `/ws/host` dialer.
    pub async fn host_apply_link_token(
        &self,
        host_installation_token: String,
    ) -> Result<HostApplyLinkTokenDto> {
        let client = self.client().await?;
        let req = HostApplyLinkTokenParams {
            host_installation_token,
        };
        let response: HostApplyLinkTokenResponse = client
            .request("minos_local_host_apply_link_token", [req])
            .await
            .context("minos_local_host_apply_link_token")?;
        Ok(HostApplyLinkTokenDto {
            linked: response.linked,
        })
    }

    /// Reattach a suspended/persisted session. When `auto_continue` is true and
    /// the store has `needs_continue`, injects a one-shot CONTINUE prompt.
    pub async fn resume_session(&self, session_id: String, auto_continue: bool) -> Result<()> {
        let client = self.client().await?;
        let req = minos_protocol::ResumeSessionRequest {
            session_id,
            auto_continue,
        };
        let _: StartAgentResponse = client
            .request("minos_local_resume_session", [req])
            .await
            .context("minos_local_resume_session")?;
        Ok(())
    }

    pub async fn resolve_approval(
        &self,
        request_id: String,
        session_id: String,
        decision: serde_json::Value,
    ) -> Result<()> {
        let client = self.client().await?;
        let params = ApprovalDecisionRequest {
            request_id,
            session_id,
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
        session_id: String,
        permission_id: String,
        response: String,
    ) -> Result<()> {
        let client = self.client().await?;
        let params = minos_protocol::local_rpc::RespondOpencodePermissionRequest {
            session_id,
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
        session_id: String,
        question_id: String,
        answers: Vec<Vec<String>>,
    ) -> Result<()> {
        let client = self.client().await?;
        let params = minos_protocol::local_rpc::RespondOpencodeQuestionRequest {
            session_id,
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
        // Display is formatListActivityTime(updatedAtMs) in the webview — no
        // frozen relative string from the host process (stale the moment it lands).
        updated_at: String::new(),
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
        git_mode: c.git_mode,
        git_dirty: c.git_dirty,
        git_head: c.git_head,
        running_count: c.running_count,
        approval_count: c.needs_attention_count,
    }
}

/// Map a durable conversation message to a timeline kind.
///
/// Returns `"git_activity"` when structured git activity was parsed from the
/// body; otherwise `"text"`. Approval UI is **not** derived here — real
/// approvals are session reverse-requests, not conversation timeline rows.
fn conversation_timeline_kind(role: &str, has_git_activity: bool) -> &'static str {
    if has_git_activity {
        return "git_activity";
    }
    if role.eq_ignore_ascii_case("system") {
        return "system";
    }
    "text"
}

fn map_reaction_group(g: LocalReactionGroup) -> ReactionGroupDto {
    ReactionGroupDto {
        emoji: g.emoji,
        count: g.count,
        reacted_by_me: g.reacted_by_me,
        actors: g
            .actors
            .into_iter()
            .map(|a| ReactionActorDto {
                actor_id: a.actor_id,
                actor_kind: a.actor_kind,
                display_name: a.display_name,
            })
            .collect(),
    }
}

fn map_message(m: LocalConversationMessage) -> MessageDto {
    let git_activity = m
        .git_activity
        .as_ref()
        .and_then(|a| serde_json::to_value(a).ok());
    let kind = conversation_timeline_kind(&m.sender_role, git_activity.is_some());
    let mentions = m
        .mentions
        .into_iter()
        .map(|mention| MentionDto {
            agent: agent_label(mention.agent),
            session_id: mention.session_id,
            session_short_id: mention.session_short_id,
        })
        .collect();
    MessageDto {
        id: m.message_id,
        message_seq: m.message_seq,
        role: m.sender_role,
        agent: m.agent.map(agent_label),
        session_id: m.session_id,
        body: m.body,
        // Empty: webview formats created_at_ms with local timezone.
        time: String::new(),
        created_at_ms: m.created_at_ms,
        kind: kind.into(),
        reply_to_message_id: m.reply_to_message_id,
        delegation_id: m.delegation_id,
        mentions,
        reactions: m.reactions.into_iter().map(map_reaction_group).collect(),
        git_activity,
    }
}

fn map_session(
    t: SessionSummary,
    conversation_id: &str,
    conversation_title: Option<String>,
) -> SessionDto {
    let short_id = short_session_id(&t.session_id);
    let status = thread_status_label(&t);
    let agent = agent_label(t.agent);
    SessionDto {
        id: t.session_id,
        conversation_id: conversation_id.to_owned(),
        conversation_title,
        agent: agent.clone(),
        short_id,
        status,
        model: "—".into(),
        parent_id: t.parent_session_id,
        summary: t.title.unwrap_or_else(|| format!("{agent} session")),
        message_count: t.message_count,
        first_ts_ms: t.first_ts_ms,
        last_ts_ms: t.last_ts_ms,
        needs_continue: t.needs_continue,
    }
}

fn short_session_id(session_id: &str) -> String {
    let mut end = session_id.len().min(8);
    while end > 0 && !session_id.is_char_boundary(end) {
        end -= 1;
    }
    session_id[..end].to_owned()
}

/// Folds raw UiEventMessage stream into chat-like items (aligned with TUI ChatState).
///
/// Timeline policy (desktop parity with mobile + TUI after 2026-07-23):
/// - Text only mutates the **timeline tail** bubble for that segment.
/// - Non-tail `TextReplace` (OpenCode finished-part snapshot after tools) is
///   ignored when content is unchanged; different content appends a **new** row
///   at the end (part segments / post-tool narration) — never rewrites above tools.
/// - OpenCode text parts may use `message_id + RS + part_id` segment keys
///   (see minos-ui-protocol opencode translator).
/// - `task` tools project as a single `subagent` card; raw XML output is not a row.
struct TranscriptAssembler {
    session_id: String,
    items: Vec<TranscriptItemDto>,
    /// message_id → role for open messages (base id, without part suffix)
    open_roles: std::collections::HashMap<String, minos_ui_protocol::MessageRole>,
    counter: u64,
}

impl TranscriptAssembler {
    fn new(session_id: String) -> Self {
        Self {
            session_id,
            items: Vec::new(),
            open_roles: std::collections::HashMap::new(),
            counter: 0,
        }
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{}-{prefix}-{}", self.session_id, self.counter)
    }

    fn ingest_frame(&mut self, seq: u64, ts_ms: i64, events: Vec<UiEventMessage>) {
        for ev in events {
            self.apply(seq, ts_ms, ev);
        }
    }

    fn finish(mut self) -> Vec<TranscriptItemDto> {
        // History keeps raw approval/request frames after the user decided;
        // demote cards that are followed by later agent/user progress.
        demote_resolved_approvals_in_place(&mut self.items);
        self.items
    }

    /// Mark a reverse-request card non-actionable (clears request_id).
    fn resolve_approval_card(&mut self, request_id: &str, text: &str) {
        for item in self.items.iter_mut().rev() {
            if item.request_id.as_deref() == Some(request_id)
                && (item.kind == "approval" || item.kind == "question")
            {
                let is_plan = item.approval_method.as_deref() == Some("x.ai/exit_plan_mode");
                item.kind = "status".into();
                item.text = if is_plan {
                    "Plan approved".into()
                } else {
                    text.into()
                };
                item.request_id = None;
                item.options = None;
                break;
            }
        }
    }

    fn role_of(&self, message_id: &str) -> minos_ui_protocol::MessageRole {
        let base = base_message_id(message_id);
        self.open_roles
            .get(base)
            .or_else(|| self.open_roles.get(message_id))
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

    /// Stable id for a text/reasoning segment so live frame merges hit by id.
    fn stable_text_id(&self, kind: &str, message_id: &str) -> String {
        format!("{}:{kind}:{message_id}", self.session_id)
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

        // 1) Tail match → in-place stream/replace (active bubble).
        if self.tail_text_matches(&message_id, kind) {
            if let Some(last) = self.items.last_mut() {
                if replace {
                    last.text = chunk;
                } else {
                    last.text.push_str(&chunk);
                }
                last.ts_ms = ts_ms;
                last.seq = seq;
                return;
            }
        }

        // 2) Non-tail TextReplace for an existing segment: freeze mid-timeline.
        //    Same body (OpenCode finished-part snapshot) → drop.
        //    Different body → append new row at end (new part_id / post-tool text).
        if replace {
            if let Some(item) =
                self.items.iter().rev().find(|i| {
                    i.kind == kind && i.message_id.as_deref() == Some(message_id.as_str())
                })
            {
                if item.text == chunk {
                    return;
                }
                // Fall through to append a new bubble with the new body.
            }
        }

        // 3) Delta when not tail (tools/status already below) → new segment at end.
        //    Use stable id keyed by segment message_id (OpenCode may encode part_id).
        let id = self.stable_text_id(kind, &message_id);
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            // Stable id already present. Only mutate if it is still the tail.
            if pos + 1 == self.items.len() {
                let existing = &mut self.items[pos];
                if replace {
                    existing.text = chunk;
                } else {
                    existing.text.push_str(&chunk);
                }
                existing.ts_ms = ts_ms;
                existing.seq = seq;
                return;
            }
            // Id exists above later rows — append with a distinct id for the new segment.
            self.counter += 1;
            self.items.push(TranscriptItemDto {
                id: format!("{id}:s{}", self.counter),
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
            return;
        }

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
                self.open_roles
                    .insert(base_message_id(&message_id).to_string(), role);
            }
            UiEventMessage::MessageCompleted { message_id, .. } => {
                self.open_roles.remove(base_message_id(&message_id));
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
                // OpenCode `task` → single subagent card (TUI parity); never a bare tool row.
                if is_task_tool_name(&name) {
                    self.upsert_subagent_from_task(
                        seq,
                        ts_ms,
                        &tool_call_id,
                        None,
                        &name,
                        &args,
                        "running",
                    );
                    return;
                }
                // text = bare target (path/cmd); title = tool name. UI derives Grok verbs.
                let target = summarize_tool_args(&name, &args);
                let tool_id = stable_tool_id(&tool_call_id);
                let args_detail = if args.trim().is_empty() {
                    None
                } else {
                    Some(truncate_str(&args, 2000))
                };
                // Upsert: Grok re-emits ToolCallPlaced for title/kind refine while a
                // progressive ToolCallCompleted may already have flipped the row to
                // tool_result. Never push a second card or demote completed → open.
                if let Some(item) = self.items.iter_mut().rev().find(|i| {
                    i.id == tool_id || i.request_id.as_deref() == Some(tool_call_id.as_str())
                }) {
                    if !name.is_empty() {
                        item.title = Some(name);
                    }
                    if !tool_target_is_useless(&target, item.title.as_deref()) {
                        item.text = target;
                    }
                    // Only refresh args detail while still open; keep result body.
                    if item.kind == "tool" {
                        if args_detail.is_some() {
                            item.detail = args_detail;
                        }
                    }
                    if item.message_id.is_none() {
                        item.message_id = Some(message_id);
                    }
                    item.request_id = Some(tool_call_id);
                    item.ts_ms = ts_ms;
                    item.seq = seq;
                    return;
                }
                self.items.push(TranscriptItemDto {
                    id: tool_id,
                    kind: "tool".into(),
                    role: None,
                    text: target,
                    detail: args_detail,
                    title: Some(name),
                    ts_ms,
                    seq,
                    message_id: Some(message_id),
                    request_id: Some(tool_call_id),
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
                // task tool completion → update subagent card only (no XML tool_result row).
                if self.complete_subagent_for_tool(seq, ts_ms, &tool_call_id, &out, is_error) {
                    return;
                }
                // Keep bare target in `text`; put output into `detail` for expand.
                // Match open *or* already-progressive-completed rows so terminal
                // EditsApplied can refine without pushing a twin tool_result.
                let mut updated = false;
                let tool_id = stable_tool_id(&tool_call_id);
                for item in self.items.iter_mut().rev() {
                    let is_tool_row =
                        matches!(item.kind.as_str(), "tool" | "tool_result" | "tool_error");
                    if is_tool_row
                        && (item.id == tool_id
                            || item.request_id.as_deref() == Some(tool_call_id.as_str())
                            || item.id.ends_with(&format!(":{tool_call_id}")))
                    {
                        item.kind = if is_error {
                            "tool_error".into()
                        } else {
                            "tool_result".into()
                        };
                        // Refresh target from detail/args if it was a useless fallback.
                        if tool_target_is_useless(&item.text, item.title.as_deref()) {
                            if let Some(better) = summarize_tool_args_from_detail(
                                item.title.as_deref(),
                                item.detail.as_deref(),
                            ) {
                                item.text = better;
                            }
                        }
                        let detail = truncate_str(&out, 4000);
                        item.detail = if detail.is_empty() || is_task_xml_output(&detail) {
                            // Never surface raw OpenCode task XML as expand body title source.
                            if is_task_xml_output(&detail) {
                                None
                            } else if detail.is_empty() {
                                None
                            } else {
                                Some(detail)
                            }
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
                    // Orphan completion: if XML task output, project as subagent not tool.
                    if is_task_xml_output(&out) {
                        let sub = sub_session_id_from_task_xml(&out);
                        self.upsert_subagent_from_task(
                            seq,
                            ts_ms,
                            &tool_call_id,
                            sub.as_deref(),
                            "task",
                            "{}",
                            if is_error { "failed" } else { "completed" },
                        );
                        return;
                    }
                    let summary = summarize_tool_output_line(&out, is_error);
                    self.items.push(TranscriptItemDto {
                        id: tool_id,
                        kind: if is_error {
                            "tool_error".into()
                        } else {
                            "tool_result".into()
                        },
                        role: None,
                        text: summary,
                        detail: Some(truncate_str(&out, 4000)).filter(|s| !s.is_empty()),
                        title: Some(tool_call_id.clone()),
                        ts_ms,
                        seq,
                        message_id: None,
                        request_id: Some(tool_call_id),
                        approval_method: None,
                        options: None,
                        approve_response: None,
                        decline_response: None,
                    });
                }
            }
            UiEventMessage::SessionOpened { title, agent, .. } => {
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
            UiEventMessage::SessionClosed { .. } => {
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
                sub_session_id,
                tool_call_id,
                agent,
                model,
                title,
                prompt,
                ..
            } => {
                let desc = title
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| one_line(s))
                    .or_else(|| prompt.as_deref().map(subagent_prompt_summary));
                self.upsert_subagent_card(
                    seq,
                    ts_ms,
                    if tool_call_id.is_empty() {
                        None
                    } else {
                        Some(tool_call_id.as_str())
                    },
                    Some(sub_session_id.as_str()),
                    &agent_label(agent),
                    model.as_deref(),
                    "running",
                    desc,
                );
            }
            UiEventMessage::SubagentStatusUpdated {
                sub_session_id,
                status,
            } => {
                let label = match status {
                    minos_ui_protocol::SubagentStatus::Running => "running",
                    minos_ui_protocol::SubagentStatus::Completed => "completed",
                    minos_ui_protocol::SubagentStatus::Failed => "failed",
                    minos_ui_protocol::SubagentStatus::Interrupted => "interrupted",
                };
                // Prefer existing card's agent/detail; never invent a second row.
                let (agent, desc, tool) = self
                    .find_subagent_index(None, Some(sub_session_id.as_str()))
                    .map(|i| {
                        let it = &self.items[i];
                        (
                            it.title.clone().unwrap_or_else(|| "opencode".into()),
                            it.detail.clone(),
                            it.request_id.clone(),
                        )
                    })
                    .unwrap_or_else(|| ("opencode".into(), None, None));
                self.upsert_subagent_card(
                    seq,
                    ts_ms,
                    tool.as_deref(),
                    Some(sub_session_id.as_str()),
                    &agent,
                    None,
                    label,
                    desc,
                );
            }
            // Product-critical Raw: user-facing reverse-requests.
            // Other Raw ACP noise is intentionally dropped.
            UiEventMessage::Raw { kind, payload_json } => {
                if kind == "approval/request" {
                    if let Some(item) = approval_item_from_payload(seq, ts_ms, &payload_json) {
                        self.items.push(item);
                    }
                } else if kind == "approval/resolved" || kind == "approval/timeout" {
                    // Clear the matching interactive card so history reload does
                    // not re-open a finished plan/permission reverse-request.
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload_json) {
                        let rid = value
                            .pointer("/params/request_id")
                            .or_else(|| value.get("request_id"))
                            .and_then(|v| v.as_str());
                        if let Some(rid) = rid {
                            self.resolve_approval_card(
                                rid,
                                if kind == "approval/timeout" {
                                    "Approval timed out"
                                } else {
                                    "Approval resolved"
                                },
                            );
                        }
                    }
                    if kind == "approval/timeout" {
                        self.push_simple(
                            seq,
                            ts_ms,
                            "status",
                            "Approval timed out".into(),
                            Some("Approval".into()),
                            None,
                            None,
                        );
                    }
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
            UiEventMessage::SessionTitleUpdated { .. } => {}
        }
    }

    /// Locate the single subagent card for this tool/session (if any).
    fn find_subagent_index(
        &self,
        tool_call_id: Option<&str>,
        sub_session_id: Option<&str>,
    ) -> Option<usize> {
        self.items.iter().position(|item| {
            if item.kind != "subagent" {
                return false;
            }
            if let Some(tc) = tool_call_id.filter(|s| !s.is_empty()) {
                if item.request_id.as_deref() == Some(tc)
                    || item.id == stable_subagent_id(Some(tc), None)
                    || item.id.ends_with(&format!(":tool:{tc}"))
                {
                    return true;
                }
            }
            if let Some(sid) = sub_session_id.filter(|s| !s.is_empty()) {
                if item.message_id.as_deref() == Some(sid)
                    || item.id == stable_subagent_id(None, Some(sid))
                    || item.id == format!("subagent:{sid}")
                    || item.id == format!("subagent:ses:{sid}")
                {
                    return true;
                }
            }
            false
        })
    }

    /// One card only: create or in-place update. Always prefer session-scoped id.
    fn upsert_subagent_card(
        &mut self,
        seq: u64,
        ts_ms: i64,
        tool_call_id: Option<&str>,
        sub_session_id: Option<&str>,
        agent_name: &str,
        model: Option<&str>,
        status_label: &str,
        description: Option<String>,
    ) {
        let running = status_label == "running";
        let sid = sub_session_id.filter(|s| !s.is_empty());
        let tc = tool_call_id.filter(|s| !s.is_empty());
        let id_short = sid
            .map(short_session_id)
            .or_else(|| tc.map(short_session_id))
            .unwrap_or_else(|| "sub".into());
        let agent = if agent_name.is_empty() || agent_name == "subagent" {
            "opencode"
        } else {
            agent_name
        };
        let header = format_subagent_header(running, agent, &id_short, model, status_label);
        let card_id = stable_subagent_id(tc, sid);

        if let Some(idx) = self.find_subagent_index(tc, sid).or_else(|| {
            // Single orphan running card (task placed, session id arrives later).
            if sid.is_some() {
                let running_orphans: Vec<usize> = self
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| {
                        it.kind == "subagent"
                            && it.message_id.is_none()
                            && it.text.contains("Running")
                    })
                    .map(|(i, _)| i)
                    .collect();
                if running_orphans.len() == 1 {
                    return Some(running_orphans[0]);
                }
            }
            None
        }) {
            let item = &mut self.items[idx];
            item.id = card_id;
            item.kind = "subagent".into();
            item.text = header;
            // Keep a real agent label; don't clobber "opencode" with placeholder "subagent".
            if agent != "subagent" {
                item.title = Some(agent.to_string());
            } else if item.title.as_deref().unwrap_or("").is_empty() {
                item.title = Some("opencode".into());
            }
            if let Some(d) = description {
                if !d.is_empty() {
                    item.detail = Some(d);
                }
            }
            if let Some(s) = sid {
                item.message_id = Some(s.to_string());
            }
            if let Some(t) = tc {
                item.request_id = Some(t.to_string());
            }
            item.ts_ms = ts_ms;
            item.seq = seq;
            return;
        }

        self.items.push(TranscriptItemDto {
            id: card_id,
            kind: "subagent".into(),
            role: None,
            text: header,
            detail: description.filter(|d| !d.is_empty()),
            title: Some(agent.to_string()),
            ts_ms,
            seq,
            message_id: sid.map(str::to_string),
            request_id: tc.map(str::to_string),
            approval_method: None,
            options: None,
            approve_response: None,
            decline_response: None,
        });
    }

    fn upsert_subagent_from_task(
        &mut self,
        seq: u64,
        ts_ms: i64,
        tool_call_id: &str,
        sub_session_id: Option<&str>,
        _name: &str,
        args_json: &str,
        status_label: &str,
    ) {
        let (description, subagent_type, prompt_summary) = parse_task_tool_fields(args_json);
        let agent_name = subagent_type
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "opencode".into());
        let desc = description.filter(|s| !s.is_empty()).or(prompt_summary);
        self.upsert_subagent_card(
            seq,
            ts_ms,
            if tool_call_id.is_empty() {
                None
            } else {
                Some(tool_call_id)
            },
            sub_session_id,
            &agent_name,
            None,
            status_label,
            desc,
        );
    }

    /// Returns true when completion was absorbed by a subagent card.
    fn complete_subagent_for_tool(
        &mut self,
        seq: u64,
        ts_ms: i64,
        tool_call_id: &str,
        output: &str,
        is_error: bool,
    ) -> bool {
        let from_xml = sub_session_id_from_task_xml(output);
        let idx = self.find_subagent_index(Some(tool_call_id), from_xml.as_deref());
        // Only absorb when we already have a task/subagent card or task XML output.
        if idx.is_none() && !is_task_xml_output(output) {
            return false;
        }
        let status = if is_error { "failed" } else { "completed" };
        let (agent, desc) = idx
            .map(|i| {
                (
                    self.items[i]
                        .title
                        .clone()
                        .unwrap_or_else(|| "opencode".into()),
                    self.items[i].detail.clone(),
                )
            })
            .unwrap_or_else(|| ("opencode".into(), None));
        self.upsert_subagent_card(
            seq,
            ts_ms,
            Some(tool_call_id),
            from_xml.as_deref(),
            &agent,
            None,
            status,
            desc,
        );
        true
    }
}

/// Record separator used by OpenCode translator to bind text events to part_id.
const MESSAGE_PART_SEP: char = '\u{1e}';

fn base_message_id(message_id: &str) -> &str {
    message_id
        .split(MESSAGE_PART_SEP)
        .next()
        .unwrap_or(message_id)
}

fn stable_tool_id(tool_call_id: &str) -> String {
    format!("tool:{tool_call_id}")
}

/// Canonical id: prefer sub_session once known so spawn/status/complete share one row.
fn stable_subagent_id(tool_call_id: Option<&str>, sub_session_id: Option<&str>) -> String {
    if let Some(sid) = sub_session_id.filter(|s| !s.is_empty()) {
        return format!("subagent:{sid}");
    }
    if let Some(tc) = tool_call_id.filter(|s| !s.is_empty()) {
        return format!("subagent:tool:{tc}");
    }
    "subagent:unknown".into()
}

fn is_task_tool_name(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n == "task" || n.ends_with(":task") || n == "subagent" || n.contains("collabagent")
}

fn is_task_xml_output(out: &str) -> bool {
    let t = out.trim_start();
    t.starts_with("<task") || t.contains("<task id=")
}

fn sub_session_id_from_task_xml(output: &str) -> Option<String> {
    let task_tag_start = output.find("<task")?;
    let tag = &output[task_tag_start..];
    let tag_end = tag.find('>').unwrap_or(tag.len());
    let tag = &tag[..tag_end];
    let id_key = "id=\"";
    let id_start = tag.find(id_key)? + id_key.len();
    let id_end = tag[id_start..].find('"')? + id_start;
    let id = tag[id_start..id_end].trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn subagent_prompt_summary(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or("").trim();
    let mut summary: String = first_line.chars().take(100).collect();
    if first_line.chars().count() > 100 {
        summary.push('…');
    }
    summary
}

fn format_subagent_header(
    running: bool,
    agent: &str,
    id_short: &str,
    model: Option<&str>,
    status: &str,
) -> String {
    let verb = if running { "Running" } else { "Ran" };
    let mut s = format!("{verb} subagent {agent} #{id_short}");
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        s.push_str(" · ");
        s.push_str(m);
    }
    s.push_str(" · ");
    s.push_str(status);
    s
}

fn parse_task_tool_fields(args_json: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Some(value) = parse_tool_args_json(args_json) else {
        return (None, None, None);
    };
    let description = find_tool_stringish(
        &value,
        &["description", "title", "_display_title", "display_title"],
    )
    .map(|s| truncate_str(&one_line(&s), 120));
    let subagent_type =
        find_tool_stringish(&value, &["subagent_type", "subagentType", "agent", "type"]);
    let prompt = find_tool_stringish(&value, &["prompt", "instructions", "task"])
        .map(|s| subagent_prompt_summary(&s));
    (description, subagent_type, prompt)
}

fn tool_target_is_useless(target: &str, tool_name: Option<&str>) -> bool {
    let t = target.trim();
    if t.is_empty() || t == "…" || t == "..." {
        return true;
    }
    if let Some(name) = tool_name {
        let n = name.trim().to_ascii_lowercase();
        let tl = t.to_ascii_lowercase();
        if tl == n || tl == tool_subject_from_name(name).to_ascii_lowercase() {
            return true;
        }
    }
    is_markupish_tool_line(t)
}

fn summarize_tool_args_from_detail(
    tool_name: Option<&str>,
    detail: Option<&str>,
) -> Option<String> {
    let detail = detail?;
    let name = tool_name.unwrap_or("tool");
    let target = summarize_tool_args(name, detail);
    if tool_target_is_useless(&target, Some(name)) {
        None
    } else {
        Some(target)
    }
}

fn is_markupish_tool_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('<')
        && (t.starts_with("<task")
            || t.starts_with("<path")
            || t.starts_with("<type")
            || t.starts_with("<content")
            || t.starts_with("<tool")
            || t.starts_with("<?xml"))
}

/// Kinds that prove the agent/user continued past a parked reverse-request.
fn is_approval_progress_kind(kind: &str) -> bool {
    matches!(
        kind,
        "assistant"
            | "text"
            | "reasoning"
            | "tool"
            | "tool_result"
            | "tool_error"
            | "subagent"
            | "user"
    )
}

/// Demote approval/question cards followed by later progress (history repair).
fn demote_resolved_approvals_in_place(items: &mut [TranscriptItemDto]) {
    let mut max_progress_seq: Option<u64> = None;
    for item in items.iter() {
        if is_approval_progress_kind(&item.kind) {
            max_progress_seq = Some(max_progress_seq.map_or(item.seq, |m| m.max(item.seq)));
        }
    }
    let Some(max_progress_seq) = max_progress_seq else {
        return;
    };
    for item in items.iter_mut() {
        if (item.kind == "approval" || item.kind == "question")
            && item.request_id.is_some()
            && item.seq < max_progress_seq
        {
            let is_plan = item.approval_method.as_deref() == Some("x.ai/exit_plan_mode");
            let is_question = item.kind == "question";
            item.kind = "status".into();
            item.text = if is_plan {
                "Plan approved".into()
            } else if is_question {
                "Question answered".into()
            } else {
                "Approval resolved".into()
            };
            item.request_id = None;
            item.options = None;
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
        return (format!("{prefix}\nReply with your answer."), None);
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
///
/// Prefer OpenCode `state.title` / path / command. Never return the bare tool
/// name as the target (avoids "Reading read"). Never return markup first lines.
fn summarize_tool_args(tool_name: &str, args_json: &str) -> String {
    let subject_fallback = tool_subject_from_name(tool_name);
    let Some(value) = parse_tool_args_json(args_json) else {
        let line = one_line(args_json);
        if is_markupish_tool_line(&line) {
            return String::new();
        }
        if subject_fallback != tool_name.trim()
            && !subject_fallback.is_empty()
            && subject_fallback.to_ascii_lowercase() != tool_name.trim().to_ascii_lowercase()
        {
            return truncate_str(&one_line(subject_fallback), 120);
        }
        return String::new();
    };
    if value.is_null() {
        return String::new();
    }

    // OpenCode / translator may inject display title (state.title).
    if let Some(title) = find_tool_stringish(&value, &["_display_title", "display_title", "title"])
    {
        let t = one_line(&title);
        if !t.is_empty()
            && !is_markupish_tool_line(&t)
            && t.to_ascii_lowercase() != tool_name.trim().to_ascii_lowercase()
        {
            return truncate_str(&t, 120);
        }
    }

    let kind = ToolKind::from_tool_name(tool_name);
    let candidate = match kind {
        ToolKind::Read | ToolKind::Edit | ToolKind::List => find_tool_path(&value),
        ToolKind::Execute => find_tool_stringish(
            &value,
            &["cmd", "command", "script", "shell", "description"],
        ),
        ToolKind::Search => {
            let pattern = find_tool_stringish(
                &value,
                &["pattern", "query", "regex", "search", "grep", "needle"],
            );
            let path = find_tool_path(&value);
            match (pattern, path) {
                (Some(p), Some(path)) => Some(format!("{} in {}", one_line(&p), one_line(&path))),
                (Some(p), None) => Some(p),
                (None, Some(path)) => Some(path),
                (None, None) => None,
            }
        }
        ToolKind::WebSearch | ToolKind::WebFetch => {
            find_tool_stringish(&value, &["query", "url", "uri", "href", "q"])
        }
        ToolKind::Skill => find_tool_stringish(
            &value,
            &[
                "skill",
                "skill_name",
                "skillName",
                "name",
                "skill_path",
                "skillPath",
            ],
        ),
        ToolKind::Other => None,
    };

    if let Some(c) = candidate {
        let t = one_line(&c);
        if !t.is_empty()
            && !is_markupish_tool_line(&t)
            && t.to_ascii_lowercase() != tool_name.trim().to_ascii_lowercase()
        {
            return truncate_str(&t, 140);
        }
    }

    if is_task_tool_name(tool_name)
        || tool_name.eq_ignore_ascii_case("todo")
        || tool_name.eq_ignore_ascii_case("todowrite")
        || tool_name.eq_ignore_ascii_case("todo_write")
    {
        if let Some(task) = find_tool_stringish(
            &value,
            &["description", "title", "subagent_type", "subagentType"],
        ) {
            return truncate_str(&one_line(&task), 110);
        }
    }

    if let Some(path) = find_tool_path(&value) {
        let t = one_line(&path);
        if !is_markupish_tool_line(&t) {
            return truncate_str(&t, 120);
        }
    }
    if let Some(cmd) = find_tool_stringish(&value, &["cmd", "command", "script", "shell"]) {
        return truncate_str(&one_line(&cmd), 120);
    }
    if let Some(desc) = find_tool_stringish(&value, &["description", "query"]) {
        let t = one_line(&desc);
        if !is_markupish_tool_line(&t) {
            return truncate_str(&t, 120);
        }
    }
    // Never fall back to the bare tool name ("read") — UI would show "Reading read".
    String::new()
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
    if is_task_xml_output(trimmed) {
        return if is_error {
            "Subagent failed".into()
        } else {
            "Subagent finished".into()
        };
    }
    if is_diff_like(trimmed) {
        let (add, del) = count_diff_lines(trimmed);
        return format!("+{add}/-{del}");
    }
    // Skip OpenCode XML / markup first lines (`<path>`, `<type>`, …).
    let one = trimmed
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !is_markupish_tool_line(l))
        .unwrap_or("");
    if one.is_empty() {
        return if is_error {
            "Tool failed".into()
        } else {
            "Tool finished".into()
        };
    }
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

#[allow(dead_code)] // retained for compact JSON fallback / debugging
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

fn thread_status_label(t: &SessionSummary) -> String {
    use minos_protocol::{PauseReason, SessionState};
    if t.ended_at_ms.is_some() {
        return "done".into();
    }
    match &t.state {
        SessionState::Starting | SessionState::Resuming => "running".into(),
        SessionState::Idle => "idle".into(),
        SessionState::Running { .. } => "running".into(),
        // Between-turn daemon death was historically stored as
        // Suspended{DaemonRestart} + needs_continue=0. That is not a user pause —
        // show Idle (ready to reattach). Mid-flight recovery keeps Paused.
        SessionState::Suspended {
            reason: PauseReason::DaemonRestart,
        } if !t.needs_continue => "idle".into(),
        SessionState::Suspended { .. } => "suspended".into(),
        SessionState::Closed { .. } => "done".into(),
    }
}

fn pump_still_current(my_gen: u64, flag: &AtomicU64) -> bool {
    flag.load(Ordering::SeqCst) == my_gen
}

fn emit_push_status(app: &AppHandle, live: bool) {
    if let Err(e) = app.emit(EVENT_PUSH_STATUS, &PushStatusDto { live }) {
        warn!(
            target: "minos_desktop",
            error = %e,
            live,
            "emit daemon://push-status failed"
        );
    }
}

/// When a pump ends while still current, notify the webview so livePush falls
/// back and degraded quiet polls re-enable. Superseded gens stay silent.
fn emit_push_dead_if_current(app: &AppHandle, my_gen: u64, gen_flag: &AtomicU64) {
    if pump_still_current(my_gen, gen_flag) {
        warn!(
            target: "minos_desktop",
            generation = my_gen,
            "daemon push pump ended; signaling live=false"
        );
        emit_push_status(app, false);
    }
}

fn session_state_to_status(state: &SessionState) -> String {
    match state {
        SessionState::Starting | SessionState::Resuming | SessionState::Running { .. } => {
            "running".into()
        }
        SessionState::Idle => "idle".into(),
        SessionState::Suspended { .. } => "suspended".into(),
        SessionState::Closed { .. } => "done".into(),
    }
}

fn map_manager_event(ev: LocalManagerEvent) -> ManagerEventDto {
    match ev {
        LocalManagerEvent::SessionAdded {
            session_id,
            workspace,
            agent,
            parent_session_id,
        } => ManagerEventDto::SessionAdded {
            session_id,
            agent: agent_label(agent),
            parent_session_id,
            workspace,
        },
        LocalManagerEvent::SessionStateChanged {
            session_id,
            new,
            at_ms,
            ..
        } => ManagerEventDto::SessionStateChanged {
            session_id,
            status: session_state_to_status(&new),
            at_ms,
        },
        LocalManagerEvent::SessionClosed { session_id, .. } => {
            ManagerEventDto::SessionClosed { session_id }
        }
        LocalManagerEvent::InstanceCrashed {
            affected_threads, ..
        } => ManagerEventDto::InstanceCrashed {
            affected_session_ids: affected_threads,
        },
    }
}

fn frame_to_ingest_dto(frame: LocalIngestFrame) -> IngestEventDto {
    let mut assembler = TranscriptAssembler::new(frame.session_id.clone());
    assembler.ingest_frame(frame.seq, frame.ts_ms, frame.ui_events);
    let items = assembler.finish();
    let has_pending_approval = items
        .iter()
        .any(|it| (it.kind == "approval" || it.kind == "question") && it.request_id.is_some());
    IngestEventDto {
        session_id: frame.session_id,
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
                emit_push_dead_if_current(&app, my_gen, &gen_flag);
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
        emit_push_dead_if_current(&app, my_gen, &gen_flag);
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
                emit_push_dead_if_current(&app, my_gen, &gen_flag);
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
        emit_push_dead_if_current(&app, my_gen, &gen_flag);
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
                emit_push_dead_if_current(&app, my_gen, &gen_flag);
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
                    let dto = ConversationEventDto::MessageAppended {
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
                Some(Ok(LocalConversationEvent::ConversationReactionToggled {
                    conversation_id,
                    message_id,
                    reactions,
                })) => {
                    if !pump_still_current(my_gen, &gen_flag) {
                        break;
                    }
                    let dto = ConversationEventDto::ReactionToggled {
                        conversation_id,
                        message_id,
                        reactions: reactions.into_iter().map(map_reaction_group).collect(),
                    };
                    if let Err(e) = app.emit(EVENT_CONVERSATION, &dto) {
                        warn!(
                            target: "minos_desktop",
                            error = %e,
                            "emit daemon://conversation reaction failed"
                        );
                    }
                }
                Some(Ok(LocalConversationEvent::RosterChanged {
                    conversation_id,
                    members,
                })) => {
                    if !pump_still_current(my_gen, &gen_flag) {
                        break;
                    }
                    let dto = ConversationEventDto::RosterChanged {
                        conversation_id,
                        members: members
                            .into_iter()
                            .map(|m| RosterMemberDto {
                                agent: agent_label(m.agent),
                                brief: m.brief,
                                joined_at_ms: m.joined_at_ms,
                            })
                            .collect(),
                    };
                    if let Err(e) = app.emit(EVENT_CONVERSATION, &dto) {
                        warn!(
                            target: "minos_desktop",
                            error = %e,
                            "emit daemon://conversation roster_changed failed"
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
        emit_push_dead_if_current(&app, my_gen, &gen_flag);
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
    use minos_protocol::{PauseReason, SessionState, SessionSummary};

    fn summary(state: SessionState, needs_continue: bool) -> SessionSummary {
        SessionSummary {
            session_id: "t1".into(),
            agent: AgentName::Codex,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            end_reason: None,
            parent_session_id: None,
            state,
            needs_continue,
        }
    }

    #[test]
    fn daemon_restart_without_needs_continue_displays_idle() {
        let s = summary(
            SessionState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            false,
        );
        assert_eq!(thread_status_label(&s), "idle");
    }

    #[test]
    fn daemon_restart_with_needs_continue_displays_suspended() {
        let s = summary(
            SessionState::Suspended {
                reason: PauseReason::DaemonRestart,
            },
            true,
        );
        assert_eq!(thread_status_label(&s), "suspended");
    }

    #[test]
    fn user_interrupt_displays_suspended_even_without_needs_continue() {
        let s = summary(
            SessionState::Suspended {
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
mod transcript_assembler_tests {
    use super::*;
    use minos_ui_protocol::{MessageRole, UiEventMessage};

    #[test]
    fn text_replace_same_body_after_tools_freezes_mid_timeline() {
        // OpenCode: text_delta → tool → text_replace(full same narration snapshot).
        // Must NOT rewrite the bubble above tools, and must NOT twin.
        let mut a = TranscriptAssembler::new("thr-opencode".into());
        let mid = "msg_f83e32698001L2Zjg3C87TJQ45".to_string();
        let body = "现在让我读取 workspace-store";
        a.ingest_frame(
            1,
            1,
            vec![
                UiEventMessage::MessageStarted {
                    message_id: mid.clone(),
                    role: MessageRole::Assistant,
                    started_at_ms: 0,
                },
                UiEventMessage::TextDelta {
                    message_id: mid.clone(),
                    text: body.into(),
                },
                UiEventMessage::ToolCallPlaced {
                    message_id: mid.clone(),
                    tool_call_id: "call_read_1".into(),
                    name: "read".into(),
                    args_json: r#"{"filePath":"workspace-store.ts"}"#.into(),
                },
                UiEventMessage::TextReplace {
                    message_id: mid.clone(),
                    text: body.into(),
                },
            ],
        );
        let items = a.finish();
        let assistants: Vec<_> = items.iter().filter(|i| i.kind == "assistant").collect();
        assert_eq!(
            assistants.len(),
            1,
            "same-body replace after tools must not twin: {:?}",
            items
                .iter()
                .map(|i| (i.kind.as_str(), i.text.as_str()))
                .collect::<Vec<_>>()
        );
        assert_eq!(assistants[0].text, body);
        assert!(items.iter().any(|i| i.kind == "tool"));
        // Tool must sit *after* the frozen assistant row.
        let a_pos = items.iter().position(|i| i.kind == "assistant").unwrap();
        let t_pos = items.iter().position(|i| i.kind == "tool").unwrap();
        assert!(a_pos < t_pos);
    }

    #[test]
    fn text_replace_new_body_after_tools_appends_at_end() {
        let mut a = TranscriptAssembler::new("thr-parts".into());
        let mid = "msg_1".to_string();
        a.ingest_frame(
            1,
            1,
            vec![
                UiEventMessage::MessageStarted {
                    message_id: mid.clone(),
                    role: MessageRole::Assistant,
                    started_at_ms: 0,
                },
                UiEventMessage::TextDelta {
                    message_id: mid.clone(),
                    text: "first segment".into(),
                },
                UiEventMessage::ToolCallPlaced {
                    message_id: mid.clone(),
                    tool_call_id: "c1".into(),
                    name: "read".into(),
                    args_json: r#"{"filePath":"a.ts"}"#.into(),
                },
                // Different body (new part / post-tool narration) → new row at end.
                UiEventMessage::TextReplace {
                    message_id: format!("{mid}\u{1e}prt_2"),
                    text: "second segment after tools".into(),
                },
            ],
        );
        let items = a.finish();
        let kinds: Vec<_> = items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, vec!["assistant", "tool", "assistant"]);
        assert_eq!(items[0].text, "first segment");
        assert_eq!(items[2].text, "second segment after tools");
    }

    #[test]
    fn task_tool_projects_single_subagent_card_not_xml() {
        let mut a = TranscriptAssembler::new("thr-task".into());
        a.ingest_frame(
            1,
            1,
            vec![
                UiEventMessage::ToolCallPlaced {
                    message_id: "m1".into(),
                    tool_call_id: "call_task".into(),
                    name: "task".into(),
                    args_json: r#"{"description":"Explore desktop","prompt":"long prompt here\nline2","subagent_type":"explore"}"#.into(),
                },
                UiEventMessage::SubagentSpawned {
                    parent_session_id: "thr-task".into(),
                    sub_session_id: "ses_072f".into(),
                    tool_call_id: "call_task".into(),
                    agent: minos_domain::AgentName::Opencode,
                    model: None,
                    prompt: Some("Explore the desktop architecture of the project at /long/path very thoroughly...".into()),
                    title: Some("Explore desktop".into()),
                },
                UiEventMessage::ToolCallCompleted {
                    tool_call_id: "call_task".into(),
                    output: r#"<task id="ses_072f" state="completed">done</task>"#.into(),
                    is_error: false,
                },
                UiEventMessage::SubagentStatusUpdated {
                    sub_session_id: "ses_072f".into(),
                    status: minos_ui_protocol::SubagentStatus::Completed,
                },
            ],
        );
        let items = a.finish();
        let subs: Vec<_> = items.iter().filter(|i| i.kind == "subagent").collect();
        assert_eq!(
            subs.len(),
            1,
            "expected one subagent card, got {:?}",
            items
                .iter()
                .map(|i| (i.kind.as_str(), i.text.as_str()))
                .collect::<Vec<_>>()
        );
        assert!(
            !items
                .iter()
                .any(|i| i.kind == "tool" || i.kind == "tool_result"),
            "task must not leave raw tool rows"
        );
        assert!(
            !subs[0].text.contains("<task"),
            "header must not contain XML: {}",
            subs[0].text
        );
        assert!(subs[0].text.contains("ses_072f") || subs[0].text.contains("#ses_072"));
        assert!(
            subs[0]
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("Explore desktop") && d.len() < 200),
            "detail should be short description, got {:?}",
            subs[0].detail
        );
        assert!(subs[0].text.contains("completed") || subs[0].text.starts_with("Ran"));
        assert_eq!(subs[0].id, "subagent:ses_072f");
    }

    #[test]
    fn subagent_status_only_frame_does_not_spawn_second_card_in_full_history() {
        // Simulate: running card (tool id) then later status-only with session id.
        let mut a = TranscriptAssembler::new("thr-task2".into());
        a.ingest_frame(
            1,
            1,
            vec![UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "call_only".into(),
                name: "task".into(),
                args_json: r#"{"description":"Do work","subagent_type":"explore"}"#.into(),
            }],
        );
        a.ingest_frame(
            2,
            2,
            vec![UiEventMessage::SubagentStatusUpdated {
                sub_session_id: "ses_abc".into(),
                status: minos_ui_protocol::SubagentStatus::Completed,
            }],
        );
        let items = a.finish();
        let subs: Vec<_> = items.iter().filter(|i| i.kind == "subagent").collect();
        assert_eq!(
            subs.len(),
            1,
            "running→completed must stay one card: {:?}",
            items
                .iter()
                .map(|i| (i.id.as_str(), i.text.as_str()))
                .collect::<Vec<_>>()
        );
        assert!(subs[0].text.contains("completed") || subs[0].text.starts_with("Ran"));
    }

    #[test]
    fn read_tool_uses_path_not_tool_name_as_target() {
        let mut a = TranscriptAssembler::new("thr-read".into());
        a.ingest_frame(
            1,
            1,
            vec![UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "c_read".into(),
                name: "read".into(),
                args_json: r#"{"filePath":"/Users/x/code/foo.ts","_display_title":"foo.ts"}"#
                    .into(),
            }],
        );
        let items = a.finish();
        let tool = items.iter().find(|i| i.kind == "tool").expect("tool");
        assert_ne!(tool.text, "read");
        assert!(
            tool.text.contains("foo.ts") || tool.text.contains("filePath") || !tool.text.is_empty(),
            "target={}",
            tool.text
        );
        assert!(!tool.text.eq_ignore_ascii_case("read"));
    }

    /// Grok progressive path re-Places with a refined title then Completes in the
    /// same (or later) frame. Must stay one card — no orphan `tool` left open.
    #[test]
    fn tool_place_refine_then_complete_is_single_result() {
        let mut a = TranscriptAssembler::new("thr-sr".into());
        a.ingest_frame(
            1,
            1,
            vec![UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "tc_sr".into(),
                name: "search_replace".into(),
                args_json: r#"{"file_path":"App.java"}"#.into(),
            }],
        );
        a.ingest_frame(
            2,
            2,
            vec![
                UiEventMessage::ToolCallPlaced {
                    message_id: "m1".into(),
                    tool_call_id: "tc_sr".into(),
                    name: "edit: App.java".into(),
                    args_json: r#"{"file_path":"App.java"}"#.into(),
                },
                UiEventMessage::ToolCallCompleted {
                    tool_call_id: "tc_sr".into(),
                    output: "--- a/App.java\n+++ b/App.java\n@@\n-old\n+new\n".into(),
                    is_error: false,
                },
            ],
        );
        // Terminal refine after progressive complete.
        a.ingest_frame(
            3,
            3,
            vec![UiEventMessage::ToolCallCompleted {
                tool_call_id: "tc_sr".into(),
                output: "--- a/App.java\n+++ b/App.java\n@@\n-old\n+new\n+newer\n".into(),
                is_error: false,
            }],
        );
        let items = a.finish();
        let tools: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.kind.as_str(), "tool" | "tool_result" | "tool_error"))
            .collect();
        assert_eq!(
            tools.len(),
            1,
            "expected single tool card, got {:?}",
            tools
                .iter()
                .map(|t| (t.kind.as_str(), t.title.as_deref(), t.text.as_str()))
                .collect::<Vec<_>>()
        );
        assert_eq!(tools[0].kind, "tool_result");
        assert!(
            tools[0]
                .title
                .as_deref()
                .is_some_and(|t| t.contains("edit") || t.contains("App")),
            "refined title kept: {:?}",
            tools[0].title
        );
        assert!(
            tools[0]
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("+newer")),
            "terminal body wins: {:?}",
            tools[0].detail
        );
        assert!(
            !items.iter().any(|i| i.kind == "tool"),
            "no open tool row after complete"
        );
    }

    #[test]
    fn tool_place_after_complete_does_not_reopen() {
        let mut a = TranscriptAssembler::new("thr-reopen".into());
        a.ingest_frame(
            1,
            1,
            vec![
                UiEventMessage::ToolCallPlaced {
                    message_id: "m1".into(),
                    tool_call_id: "tc1".into(),
                    name: "search_replace".into(),
                    args_json: r#"{"file_path":"a.ts"}"#.into(),
                },
                UiEventMessage::ToolCallCompleted {
                    tool_call_id: "tc1".into(),
                    output: "ok".into(),
                    is_error: false,
                },
            ],
        );
        // Late title-only refine (live frame) must not demote tool_result → tool.
        a.ingest_frame(
            2,
            2,
            vec![UiEventMessage::ToolCallPlaced {
                message_id: "m1".into(),
                tool_call_id: "tc1".into(),
                name: "edit: a.ts".into(),
                args_json: r#"{"file_path":"a.ts"}"#.into(),
            }],
        );
        let items = a.finish();
        let tools: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.kind.as_str(), "tool" | "tool_result" | "tool_error"))
            .collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].kind, "tool_result");
        assert_eq!(tools[0].title.as_deref(), Some("edit: a.ts"));
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
