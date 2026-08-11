//! Dart-visible frb surface over `minos_mobile::MobileClient`.
//!
//! This file is the entire frb input: `flutter_rust_bridge_codegen` walks this
//! module (and its siblings under `crate::api`) to emit Dart bindings and the
//! matching `wire_*` handlers in `crate::frb_generated`. Anything added here
//! becomes visible from Dart; internal helpers live outside `crate::api`.
//!
//! The opaque wrapper [`MobileClient`] holds the real
//! `minos_mobile::MobileClient` behind a `RustOpaque` handle — Dart never
//! marshals its fields, only invokes methods on it. Domain enums/structs are
//! mirrored (see the `#[frb(mirror(...))]` blocks below) so pattern-matching
//! works on the Dart side without duplicating the localization table.

use std::path::Path;
use std::sync::OnceLock;

use flutter_rust_bridge::frb;
use minos_mobile::http::AgentSessionSummary as CoreAgentSessionSummary;
use minos_mobile::log_capture::{LogLevel as CoreLogLevel, LogRecord as CoreLogRecord};
use minos_mobile::request_trace::{
    RequestTraceRecord as CoreRequestTraceRecord, RequestTraceStatus as CoreRequestTraceStatus,
    RequestTransport as CoreRequestTransport,
};
use minos_mobile::SocialEventFrame as MobileSocialEventFrame;
use minos_mobile::UiEventFrame as MobileUiEventFrame;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::watch;

// `StreamSink` is defined by the `frb_generated_boilerplate!` macro expanded
// inside `crate::frb_generated`, not at the flutter_rust_bridge crate root.
// We re-route the name through the generated module so unqualified
// `StreamSink<T>` resolves both pre- and post-codegen.
use crate::frb_generated::StreamSink;

// Re-exported `pub use` so `crate::api::minos::TypeName` resolves for the
// generated wire code in `frb_generated.rs`. Mirror declarations below still
// provide the shape metadata the codegen needs.
pub use minos_domain::{
    AgentDescriptor, AgentName, AgentStatus, ConnectionState, ErrorKind, Lang, MinosError,
    PairingState,
};
pub use minos_protocol::{
    AgentSummary, AuthSummary, ChatMessageAttachment, ChatMessageReplySummary, ChatMessageSummary,
    CloseReason, ConversationAgentMembersResponse, ConversationKind, ConversationMembersResponse,
    ConversationParticipantsResponse, ConversationReadResponse, ConversationResponse,
    ConversationSummary, ConversationsResponse, CreateProjectRequest, CreateProjectResponse,
    DeleteProjectRequest, FriendRequestStatus, FriendRequestSummary, FriendRequestsResponse,
    FriendSummary, FriendsResponse, HostSkillError, HostSkillSummary, HostSkillsEntry, HostSummary,
    HostWorkspaceSummary, ListAgentsResponse, ListChatMessagesResponse, ListHostSkillsResponse,
    ListHostWorkspacesResponse, ListProjectSessionsParams, ListProjectSessionsResponse,
    ListProjectsResponse, ListSessionsParams, ListSessionsResponse, MessageSender,
    MyProfileResponse, PauseReason, ProjectSummary, ReactionActor, ReactionGroup,
    ReadSessionParams, ReadSessionResponse, SearchUsersResponse, SenderType, SessionState,
    SessionSummary, StartAgentResponse, ToggleReactionResponse, UpdateProjectRequest, UserSummary,
    WriteHostSkillConfigResponse,
};
pub use minos_ui_protocol::{
    ArtifactRef, DisplayPayload, MessageRole, SessionEndReason, SubagentStatus, UiEventMessage,
};

// ───────────────────────────── opaque client ─────────────────────────────

/// Opaque Dart handle around `minos_mobile::MobileClient`.
///
/// The inner type is not exposed to Dart — all interactions go through the
/// `impl` below. This keeps `Arc<dyn MobilePairingStore>` (and any other
/// non-FFI-safe internals) Rust-side.
#[frb(opaque)]
pub struct MobileClient(minos_mobile::MobileClient);

fn frb_runtime() -> &'static Runtime {
    static FRB_RUNTIME: OnceLock<Runtime> = OnceLock::new();
    FRB_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("minos-frb")
            .build()
            .expect("failed to build minos-ffi-frb tokio runtime")
    })
}

fn spawn_state_forwarder<F>(mut rx: watch::Receiver<ConnectionState>, mut emit: F)
where
    F: FnMut(ConnectionState) -> Result<(), ()> + Send + 'static,
{
    frb_runtime().spawn(async move {
        // Emit the snapshot visible at subscribe time so late subscribers
        // aren't stuck on whatever they last rendered.
        if emit(*rx.borrow_and_update()).is_err() {
            return;
        }
        while rx.changed().await.is_ok() {
            if emit(*rx.borrow()).is_err() {
                break;
            }
        }
    });
}

/// Dart-visible shape of `minos_mobile::UiEventFrame`. Held as a separate
/// type (rather than mirrored) so the `ui` field lands as the mirrored
/// `UiEventMessage` variant on the Dart side.
pub struct UiEventFrame {
    pub session_id: String,
    pub seq: u64,
    pub ui: UiEventMessage,
    pub ts_ms: i64,
}

/// Dart-visible shape of `minos_mobile::SocialEventFrame`.
pub struct SocialEventFrame {
    pub conversation_id: String,
    /// `"message"` | `"reaction_updated"`.
    pub kind: String,
    pub message: ChatMessageSummary,
    /// Durable topic for apply-ack (e.g. `conversation:{id}` / `account:{id}`).
    pub topic: String,
    /// Durable topic_seq; ack after cache commit via `ack_durable_applied`.
    pub topic_seq: i64,
}

/// Durable mobile pairing snapshot mirrored into the iOS keychain.
///
/// Includes the five auth fields (access/refresh tokens + bound account
/// identity) so the Dart-side secure store can rehydrate the full session
/// on cold launch. All five auth fields are persisted as a tuple — either
/// every one is present or all are `None`.
///
/// Device secret is not stored here — the iOS rail is bearer-only.
///
/// Backend URL and CF Access service-token headers were dropped from the
/// snapshot when pairing transitioned to compile-time `build_config` — the
/// transport-edge values never round-trip through durable storage now.
pub struct PersistedPairingState {
    pub device_id: Option<String>,
    pub access_token: Option<String>,
    pub access_expires_at_ms: Option<i64>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
}

impl From<minos_mobile::PersistedPairingState> for PersistedPairingState {
    fn from(state: minos_mobile::PersistedPairingState) -> Self {
        Self {
            device_id: state.device_id,
            access_token: state.access_token,
            access_expires_at_ms: state.access_expires_at_ms,
            refresh_token: state.refresh_token,
            account_id: state.account_id,
            account_email: state.account_email,
        }
    }
}

impl From<PersistedPairingState> for minos_mobile::PersistedPairingState {
    fn from(state: PersistedPairingState) -> Self {
        Self {
            device_id: state.device_id,
            access_token: state.access_token,
            access_expires_at_ms: state.access_expires_at_ms,
            refresh_token: state.refresh_token,
            account_id: state.account_id,
            account_email: state.account_email,
        }
    }
}

/// Dart-visible mirror of `minos_protocol::HostSummary`. One row returned by
/// `GET /v1/hosts`.
pub struct HostSummaryDto {
    pub host_device_id: String,
    pub host_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: String,
    pub online: bool,
}

impl From<HostSummary> for HostSummaryDto {
    fn from(s: HostSummary) -> Self {
        Self {
            host_device_id: s.host_device_id.to_string(),
            host_display_name: s.host_display_name,
            paired_at_ms: s.paired_at_ms,
            paired_via_device_id: s.paired_via_device_id.to_string(),
            online: s.online,
        }
    }
}

impl From<MobileUiEventFrame> for UiEventFrame {
    fn from(f: MobileUiEventFrame) -> Self {
        Self {
            session_id: f.session_id,
            seq: f.seq,
            ui: f.ui,
            ts_ms: f.ts_ms,
        }
    }
}

impl From<MobileSocialEventFrame> for SocialEventFrame {
    fn from(f: MobileSocialEventFrame) -> Self {
        Self {
            conversation_id: f.conversation_id,
            kind: f.kind,
            message: f.message,
            topic: f.topic,
            topic_seq: f.topic_seq,
        }
    }
}

/// Dart-visible auth state frame.
///
/// Defined fresh here rather than mirrored from `minos_mobile::auth` because
/// the inner `RefreshFailed` payload is `Arc<MinosError>` for cheap watch-
/// channel cloning — frb's `#[frb(mirror)]` codegen would have to round-trip
/// the Arc, which is awkward. The `From` impl below unwraps the Arc and
/// clones the inner `MinosError` (cheap, since `MinosError` derives `Clone`)
/// so the Dart side sees a plain typed-error variant.
#[derive(Debug, Clone)]
pub enum AuthStateFrame {
    Unauthenticated,
    Authenticated { account: AuthSummary },
    Refreshing,
    RefreshFailed { error: MinosError },
}

impl From<minos_mobile::auth::AuthStateFrame> for AuthStateFrame {
    fn from(f: minos_mobile::auth::AuthStateFrame) -> Self {
        use minos_mobile::auth::AuthStateFrame as M;
        match f {
            M::Unauthenticated => Self::Unauthenticated,
            M::Authenticated { account } => Self::Authenticated { account },
            M::Refreshing => Self::Refreshing,
            M::RefreshFailed { error } => Self::RefreshFailed {
                error: (*error).clone(),
            },
        }
    }
}

impl MobileClient {
    /// Construct a client backed by the built-in in-memory session store.
    /// Synchronous — no I/O happens until an auth/session method is called.
    #[frb(sync)]
    #[must_use]
    pub fn new(self_name: String) -> Self {
        Self(minos_mobile::MobileClient::new_with_in_memory_store(
            self_name,
        ))
    }

    /// Construct a client preloaded with a durable session snapshot from the
    /// Dart-side secure store.
    #[frb(sync)]
    #[must_use]
    pub fn new_with_persisted_state(self_name: String, state: PersistedPairingState) -> Self {
        Self(minos_mobile::MobileClient::new_with_persisted_state(
            self_name,
            state.into(),
        ))
    }

    /// Reconnect using the durable session snapshot already loaded from the
    /// Dart-side secure store.
    pub async fn resume_persisted_session(&self) -> Result<(), MinosError> {
        self.0.resume_persisted_session().await
    }

    /// Unlink a host installation from the account and clear local active-host
    /// when it matches. Idempotent.
    pub async fn forget_host(&self, host_device_id: String) -> Result<(), MinosError> {
        let host = parse_device_id(&host_device_id)?;
        self.0.forget_host(host).await
    }

    /// List every Mac linked to the caller's account (`GET /v1/hosts`).
    pub async fn list_paired_hosts(&self) -> Result<Vec<HostSummaryDto>, MinosError> {
        let hosts = self.0.list_paired_hosts().await?;
        Ok(hosts.into_iter().map(HostSummaryDto::from).collect())
    }

    pub async fn my_profile(&self) -> Result<MyProfileResponse, MinosError> {
        self.0.my_profile().await
    }



    pub async fn friends(&self) -> Result<FriendsResponse, MinosError> {
        self.0.friends().await
    }

    pub async fn register_agent(
        &self,
        name: String,
        description: String,
        runtime_agent: String,
        model: String,
        workspace_path: Option<String>,
        display_name: Option<String>,
        default_reasoning_effort: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<AgentSummary, MinosError> {
        self.0
            .register_agent(
                name,
                description,
                runtime_agent,
                model,
                workspace_path,
                display_name,
                default_reasoning_effort,
                system_prompt,
            )
            .await
    }

    pub async fn update_agent(
        &self,
        agent_id: String,
        name: String,
        description: String,
        runtime_agent: String,
        model: String,
        workspace_path: Option<String>,
        display_name: Option<String>,
        default_reasoning_effort: Option<String>,
        system_prompt: Option<String>,
        status: Option<String>,
    ) -> Result<AgentSummary, MinosError> {
        self.0
            .update_agent(
                agent_id,
                name,
                description,
                runtime_agent,
                model,
                workspace_path,
                display_name,
                default_reasoning_effort,
                system_prompt,
                status,
            )
            .await
    }

    pub async fn list_agents(&self) -> Result<ListAgentsResponse, MinosError> {
        self.0.list_agents().await
    }





    pub async fn conversations(&self) -> Result<ConversationsResponse, MinosError> {
        self.0.conversations().await
    }

    pub async fn delete_conversation(&self, conversation_id: String) -> Result<(), MinosError> {
        self.0.delete_conversation(conversation_id).await
    }

    pub async fn ensure_direct_conversation(
        &self,
        friend_account_id: String,
    ) -> Result<ConversationResponse, MinosError> {
        self.0.ensure_direct_conversation(friend_account_id).await
    }

    pub async fn create_group_conversation(
        &self,
        title: String,
        member_account_ids: Vec<String>,
    ) -> Result<ConversationResponse, MinosError> {
        self.0
            .create_group_conversation(title, member_account_ids)
            .await
    }

    pub async fn add_group_member(
        &self,
        conversation_id: String,
        member_account_id: String,
    ) -> Result<(), MinosError> {
        self.0
            .add_group_member(conversation_id, member_account_id)
            .await
    }

    pub async fn remove_group_member(
        &self,
        conversation_id: String,
        member_account_id: String,
    ) -> Result<(), MinosError> {
        self.0
            .remove_group_member(conversation_id, member_account_id)
            .await
    }

    pub async fn conversation_members(
        &self,
        conversation_id: String,
    ) -> Result<ConversationMembersResponse, MinosError> {
        self.0.conversation_members(conversation_id).await
    }

    pub async fn list_conversation_agents(
        &self,
        conversation_id: String,
    ) -> Result<ConversationAgentMembersResponse, MinosError> {
        self.0.list_conversation_agents(conversation_id).await
    }

    pub async fn list_conversation_participants(
        &self,
        conversation_id: String,
    ) -> Result<ConversationParticipantsResponse, MinosError> {
        self.0.list_conversation_participants(conversation_id).await
    }

    pub async fn add_agent_to_conversation(
        &self,
        conversation_id: String,
        agent_id: String,
    ) -> Result<(), MinosError> {
        self.0
            .add_agent_to_conversation(conversation_id, agent_id)
            .await
    }

    pub async fn remove_agent_from_conversation(
        &self,
        conversation_id: String,
        agent_id: String,
    ) -> Result<(), MinosError> {
        self.0
            .remove_agent_from_conversation(conversation_id, agent_id)
            .await
    }

    pub async fn mark_conversation_read(
        &self,
        conversation_id: String,
        read_up_to_message_seq: i64,
    ) -> Result<ConversationReadResponse, MinosError> {
        self.0
            .mark_conversation_read(conversation_id, read_up_to_message_seq)
            .await
    }

    pub async fn list_chat_messages(
        &self,
        conversation_id: String,
        before_seq: Option<i64>,
        after_seq: Option<i64>,
        limit: u32,
    ) -> Result<ListChatMessagesResponse, MinosError> {
        self.0
            .list_chat_messages(conversation_id, before_seq, after_seq, limit)
            .await
    }

    /// `mentions_json` is an optional JSON array of wire `MentionTarget` objects.
    pub async fn send_chat_message(
        &self,
        conversation_id: String,
        text: String,
        reply_to_message_id: Option<String>,
        client_message_id: Option<String>,
        mentions_json: Option<String>,
    ) -> Result<ChatMessageSummary, MinosError> {
        let mentions = match mentions_json
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            None => Vec::new(),
            Some(raw) => serde_json::from_str(raw).map_err(|error| MinosError::RpcCallFailed {
                method: "AppendMessage".into(),
                message: format!("invalid mentions json: {error}"),
            })?,
        };
        self.0
            .send_chat_message(
                conversation_id,
                text,
                reply_to_message_id,
                client_message_id,
                mentions,
            )
            .await
    }

    pub async fn recall_chat_message(
        &self,
        conversation_id: String,
        message_id: String,
    ) -> Result<ChatMessageSummary, MinosError> {
        self.0
            .recall_chat_message(conversation_id, message_id)
            .await
    }

    /// Toggle Hub reaction; `client_op_id` is the Intent Outbox id.
    pub async fn toggle_reaction(
        &self,
        conversation_id: String,
        message_id: String,
        emoji: String,
        client_op_id: String,
    ) -> Result<ToggleReactionResponse, MinosError> {
        self.0
            .toggle_reaction(conversation_id, message_id, emoji, client_op_id)
            .await
    }

    /// Override the active Mac the next forward-RPC routes to.
    pub async fn set_active_host(&self, host_device_id: String) -> Result<(), MinosError> {
        let host = parse_device_id(&host_device_id)?;
        self.0.set_active_host(host).await
    }

    /// Read the current active Mac id, or `None` if no pair has been
    /// completed yet.
    pub async fn active_host(&self) -> Result<Option<String>, MinosError> {
        Ok(self.0.active_host().await?.map(|id| id.to_string()))
    }

    /// Open-chat live path: subscribe `conversation:{id}` for full T1 frames.
    pub async fn subscribe_conversation(&self, conversation_id: String) -> Result<(), MinosError> {
        self.0.subscribe_conversation(conversation_id).await
    }

    /// Leave open-chat conversation topic.
    pub async fn unsubscribe_conversation(
        &self,
        conversation_id: String,
    ) -> Result<(), MinosError> {
        self.0.unsubscribe_conversation(conversation_id).await
    }

    /// Advance durable topic cursor after Dart cache/reducer commit.
    pub async fn ack_durable_applied(&self, topic: String, topic_seq: i64) {
        self.0.ack_durable_applied(topic, topic_seq).await;
    }

    /// Export the current pairing snapshot so Dart can mirror it into secure
    /// storage after pairing succeeds.
    pub async fn persisted_pairing_state(&self) -> Result<PersistedPairingState, MinosError> {
        self.0
            .persisted_pairing_state()
            .await
            .map(PersistedPairingState::from)
    }

    /// Current connection state, read from the watch-channel cache. Cheap and
    /// synchronous.
    #[frb(sync)]
    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        self.0.current_state()
    }

    /// Subscribe to connection-state transitions. Emits the current value
    /// immediately, then every subsequent change. The spawned task exits once
    /// the Dart side drops the stream (detected via `sink.add(...).is_err()`).
    pub fn subscribe_state(&self, sink: StreamSink<ConnectionState>) {
        spawn_state_forwarder(self.0.events_stream(), move |state| {
            sink.add(state).map_err(|_| ())
        });
    }

    /// Subscribe to live `UiEventFrame`s fanned out from the backend.
    /// Every frb stream sink gets its own broadcast receiver; lagging
    /// subscribers lose old frames rather than blocking the producer.
    pub fn subscribe_ui_events(&self, sink: StreamSink<UiEventFrame>) {
        let mut rx = self.0.ui_events_stream();
        frb_runtime().spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(frame) => {
                        if sink.add(UiEventFrame::from(frame)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "ui_events_stream lagged");
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Subscribe to live `SocialEventFrame`s fanned out from the backend.
    pub fn subscribe_social_events(&self, sink: StreamSink<SocialEventFrame>) {
        let mut rx = self.0.social_events_stream();
        frb_runtime().spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(frame) => {
                        if sink.add(SocialEventFrame::from(frame)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "social_events_stream lagged");
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // ─────────────────────────── account auth ──────────────────────────────

    /// Exchange a Supabase access token for Minos access/refresh tokens and
    /// adopt the resulting session.
    pub async fn login_with_supabase(
        &self,
        supabase_access_token: String,
    ) -> Result<AuthSummary, MinosError> {
        self.0.login_with_supabase(supabase_access_token).await
    }

    /// Rotate the bearer + refresh tokens. Surfaces `Refreshing` /
    /// `Authenticated` / `RefreshFailed` transitions on the auth-state
    /// stream.
    pub async fn refresh_session(&self) -> Result<(), MinosError> {
        self.0.refresh_session().await
    }

    /// Log out of the current session. Best-effort `stop_agent`, then
    /// revoke the refresh token server-side, then wipe local state.
    pub async fn logout(&self) -> Result<(), MinosError> {
        self.0.logout().await
    }

    // ─────────────────────────── agent dispatch ────────────────────────────

    /// Detect the CLI agents available on the paired runtime.
    pub async fn list_clis(&self) -> Result<Vec<AgentDescriptor>, MinosError> {
        self.0.list_clis().await
    }

    /// Scan host-side skills for the selected runtime host.
    pub async fn list_host_skills(
        &self,
        host_device_id: Option<String>,
        force_reload: bool,
    ) -> Result<ListHostSkillsResponse, MinosError> {
        self.0.list_host_skills(host_device_id, force_reload).await
    }

    pub async fn list_host_workspaces(
        &self,
        host_device_id: Option<String>,
        root: Option<String>,
        limit: u32,
    ) -> Result<ListHostWorkspacesResponse, MinosError> {
        self.0
            .list_host_workspaces(host_device_id, root, limit)
            .await
    }

    /// Enable or disable one host-side skill.
    pub async fn write_host_skill_config(
        &self,
        host_device_id: Option<String>,
        path: String,
        enabled: bool,
    ) -> Result<WriteHostSkillConfigResponse, MinosError> {
        self.0
            .write_host_skill_config(host_device_id, path, enabled)
            .await
    }

    /// Send a follow-up user message to an existing agent session.

    /// Submit a user approval decision for a pending host request.
    ///
    /// `client_request_id` is the Hub Intent Outbox id. When omitted,
    /// the mobile client generates one so the wire body never hardcodes null.

    /// Submit an opencode question answer for a pending host request.

    /// Pause an in-flight turn on the given thread. Best-effort. The thread
    /// transitions to `Suspended { UserInterrupt }` regardless of whether the
    /// codex side acknowledges in time.

    /// Permanently close the given thread. Idempotent.

    // ─────────────────────────── project rpcs ──────────────────────────────

    /// Create a new project on the daemon.

    /// List all projects on the daemon.

    /// Update a project's name.

    /// Delete a project.

    /// List sessions within a project.

    // ─────────────────────────── lifecycle hooks ───────────────────────────

    /// Mark the app as foregrounded. Resets the reconnect backoff so the
    /// next connect attempt happens promptly.
    #[frb(sync)]
    pub fn notify_foregrounded(&self) {
        self.0.notify_foregrounded();
    }

    /// Mark the app as backgrounded. Pauses the reconnect loop so we
    /// don't poke the backend while the OS is freezing the process.
    #[frb(sync)]
    pub fn notify_backgrounded(&self) {
        self.0.notify_backgrounded();
    }

    // ─────────────────────────── auth subscription ─────────────────────────

    /// Subscribe to auth-state transitions. Emits the current cached frame
    /// immediately, then every subsequent change. The spawned task exits
    /// once Dart drops the stream (detected via `sink.add(...).is_err()`).
    pub fn subscribe_auth_state(&self, sink: StreamSink<AuthStateFrame>) {
        let mut rx = self.0.subscribe_auth_state();
        frb_runtime().spawn(async move {
            // Emit the snapshot visible at subscribe time so late subscribers
            // aren't stuck on whatever they last rendered.
            let snapshot = AuthStateFrame::from(rx.borrow_and_update().clone());
            if sink.add(snapshot).is_err() {
                return;
            }
            while rx.changed().await.is_ok() {
                let frame = AuthStateFrame::from(rx.borrow().clone());
                if sink.add(frame).is_err() {
                    break;
                }
            }
        });
    }
}

/// Parse a UUID-shaped device id string emitted from Dart back into a
/// `minos_domain::DeviceId`. Surfaces `MinosError::StoreCorrupt` on
/// malformed input — the Dart side is expected to round-trip the same
/// strings it received from `HostSummaryDto.host_device_id`, so this is a
/// best-effort guard rather than a user-facing error path.
fn parse_device_id(s: &str) -> Result<minos_domain::DeviceId, MinosError> {
    uuid::Uuid::parse_str(s)
        .map(minos_domain::DeviceId)
        .map_err(|e| MinosError::StoreCorrupt {
            path: "device_id".into(),
            message: format!("invalid uuid '{s}': {e}"),
        })
}

// ────────────────────────────── free functions ──────────────────────────────

/// Initialize mobile-side Rust logging with the given directory (supplied by
/// Dart, typically `<Documents>/Minos/Logs`). Idempotent — safe to call once
/// per launch.
pub fn init_logging(log_dir: String) -> Result<(), MinosError> {
    minos_mobile::logging::init(Path::new(&log_dir))
}

/// Localize an `ErrorKind` into user-facing copy. Mirrors the host error adapter's
/// `kind_message` so Dart can render localized error strings without hard-
/// coding them.
#[frb(sync)]
#[must_use]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}

// ───────────────────────────── log capture surface ─────────────────────────────

/// Severity tag mirrored from `minos_mobile::log_capture::LogLevel`.
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<CoreLogLevel> for LogLevel {
    fn from(level: CoreLogLevel) -> Self {
        match level {
            CoreLogLevel::Trace => Self::Trace,
            CoreLogLevel::Debug => Self::Debug,
            CoreLogLevel::Info => Self::Info,
            CoreLogLevel::Warn => Self::Warn,
            CoreLogLevel::Error => Self::Error,
        }
    }
}

#[frb(sync)]
pub fn emit_log(level: LogLevel, target: String, message: String) {
    match level {
        LogLevel::Trace => {
            tracing::trace!(target: "minos_mobile::flutter", ui_target = %target, "{message}")
        }
        LogLevel::Debug => {
            tracing::debug!(target: "minos_mobile::flutter", ui_target = %target, "{message}")
        }
        LogLevel::Info => {
            tracing::info!(target: "minos_mobile::flutter", ui_target = %target, "{message}")
        }
        LogLevel::Warn => {
            tracing::warn!(target: "minos_mobile::flutter", ui_target = %target, "{message}")
        }
        LogLevel::Error => {
            tracing::error!(target: "minos_mobile::flutter", ui_target = %target, "{message}")
        }
    }
}

/// Single tracing event captured by the in-process ring buffer.
pub struct LogRecord {
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub ts_ms: i64,
}

impl From<CoreLogRecord> for LogRecord {
    fn from(record: CoreLogRecord) -> Self {
        Self {
            level: record.level.into(),
            target: record.target,
            message: record.message,
            ts_ms: record.ts_ms,
        }
    }
}

/// Snapshot the records currently held in the ring buffer (oldest first).
/// Pair this with [`subscribe_log_records`] when populating a freshly
/// mounted log panel so prior events are not lost.
#[frb(sync)]
#[must_use]
pub fn recent_log_records() -> Vec<LogRecord> {
    minos_mobile::log_capture::recent()
        .into_iter()
        .map(LogRecord::from)
        .collect()
}

/// Subscribe to the live tail. Each subscriber gets its own broadcast
/// receiver; lagging subscribers drop old records (the producer is never
/// blocked). The spawned task exits when the Dart side drops the stream.
pub fn subscribe_log_records(sink: StreamSink<LogRecord>) {
    let mut rx = minos_mobile::log_capture::subscribe();
    frb_runtime().spawn(async move {
        loop {
            match rx.recv().await {
                Ok(record) => {
                    if sink.add(LogRecord::from(record)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Best-effort tail; the Dart side can re-snapshot
                    // recent_log_records() if it cares about the gap.
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

// ───────────────────────── request trace surface ─────────────────────────

pub enum RequestTraceTransport {
    Http,
    Rpc,
}

impl From<CoreRequestTransport> for RequestTraceTransport {
    fn from(value: CoreRequestTransport) -> Self {
        match value {
            CoreRequestTransport::Http => Self::Http,
            CoreRequestTransport::Rpc => Self::Rpc,
        }
    }
}

pub enum RequestTraceStatus {
    Pending,
    Success,
    Failure,
}

impl From<CoreRequestTraceStatus> for RequestTraceStatus {
    fn from(value: CoreRequestTraceStatus) -> Self {
        match value {
            CoreRequestTraceStatus::Pending => Self::Pending,
            CoreRequestTraceStatus::Success => Self::Success,
            CoreRequestTraceStatus::Failure => Self::Failure,
        }
    }
}

pub struct RequestTraceRecord {
    pub id: u64,
    pub transport: RequestTraceTransport,
    pub method: String,
    pub target: String,
    pub session_id: Option<String>,
    pub request_summary: Option<String>,
    pub response_summary: Option<String>,
    pub error_detail: Option<String>,
    pub status: RequestTraceStatus,
    pub status_code: Option<u16>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub duration_ms: Option<u32>,
}

impl From<CoreRequestTraceRecord> for RequestTraceRecord {
    fn from(record: CoreRequestTraceRecord) -> Self {
        Self {
            id: record.id,
            transport: record.transport.into(),
            method: record.method,
            target: record.target,
            session_id: record.session_id,
            request_summary: record.request_summary,
            response_summary: record.response_summary,
            error_detail: record.error_detail,
            status: record.status.into(),
            status_code: record.status_code,
            started_at_ms: record.started_at_ms,
            completed_at_ms: record.completed_at_ms,
            duration_ms: record.duration_ms,
        }
    }
}

#[frb(sync)]
#[must_use]
pub fn recent_request_traces() -> Vec<RequestTraceRecord> {
    minos_mobile::request_trace::recent()
        .into_iter()
        .map(RequestTraceRecord::from)
        .collect()
}

#[frb(sync)]
pub fn clear_request_traces() {
    minos_mobile::request_trace::clear();
}

pub fn subscribe_request_traces(sink: StreamSink<RequestTraceRecord>) {
    let mut rx = minos_mobile::request_trace::subscribe();
    frb_runtime().spawn(async move {
        loop {
            match rx.recv().await {
                Ok(record) => {
                    if sink.add(RequestTraceRecord::from(record)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

// ─────────────────────────── mirrored domain types ───────────────────────────
//
// frb requires us to re-declare any foreign type we want to expose to Dart.
// The `#[frb(mirror(T))]` attribute tells the codegen "this declaration is the
// shape of `T` from `crate::domain`; emit Dart bindings that encode/decode
// the real `T`". The mirror declarations themselves are never instantiated;
// they exist purely as codegen hints.

#[allow(dead_code)]
#[frb(mirror(ConnectionState))]
pub enum _ConnectionState {
    Disconnected,
    Pairing,
    Connected,
    Reconnecting { attempt: u32 },
}

#[allow(dead_code)]
#[frb(mirror(PairingState))]
pub enum _PairingState {
    Unpaired,
    AwaitingPeer,
    Paired,
}

#[allow(dead_code)]
#[frb(mirror(Lang))]
pub enum _Lang {
    Zh,
    En,
}

#[allow(dead_code)]
#[frb(mirror(AgentName))]
pub enum _AgentName {
    Codex,
    Claude,
    Gemini,
    Opencode,
    Grok,
}

#[allow(dead_code)]
#[frb(mirror(AgentStatus))]
pub enum _AgentStatus {
    Ok,
    Missing,
    Error { reason: String },
}

#[allow(dead_code)]
#[frb(mirror(AgentDescriptor))]
pub struct _AgentDescriptor {
    pub name: AgentName,
    pub display_name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub status: AgentStatus,
    pub supports_model_selection: bool,
    pub supports_reasoning_effort: bool,
}

#[allow(dead_code)]
#[frb(mirror(ErrorKind))]
pub enum _ErrorKind {
    BindFailed,
    ConnectFailed,
    Disconnected,
    PairingTokenInvalid,
    PairingStateMismatch,
    DeviceNotTrusted,
    StoreIo,
    StoreCorrupt,
    CliProbeTimeout,
    CliProbeFailed,
    RpcCallFailed,
    Unauthorized,
    ConnectionStateMismatch,
    EnvelopeVersionUnsupported,
    PeerOffline,
    BackendInternal,
    CodexSpawnFailed,
    CodexConnectFailed,
    CodexProtocolError,
    GeminiSpawnFailed,
    AcpProtocolError,
    AgentAlreadyRunning,
    AgentNotRunning,
    AgentNotSupported,
    AgentSessionIdMismatch,
    IngestSeqConflict,
    SessionNotFound,
    TranslationNotImplemented,
    TranslationFailed,
    PairingQrVersionUnsupported,
    Timeout,
    NotConnected,
    RequestDropped,
    AuthRefreshFailed,
    EmailTaken,
    WeakPassword,
    RateLimited,
    InvalidCredentials,
    AgentStartFailed,
    PairingTokenExpired,
}

#[allow(dead_code)]
#[frb(mirror(MinosError))]
pub enum _MinosError {
    BindFailed { addr: String, message: String },
    ConnectFailed { url: String, message: String },
    Disconnected { reason: String },
    PairingTokenInvalid,
    PairingStateMismatch { actual: PairingState },
    DeviceNotTrusted { device_id: String },
    StoreIo { path: String, message: String },
    StoreCorrupt { path: String, message: String },
    CliProbeTimeout { bin: String, timeout_ms: u64 },
    CliProbeFailed { bin: String, message: String },
    RpcCallFailed { method: String, message: String },
    Unauthorized { reason: String },
    ConnectionStateMismatch { expected: String, actual: String },
    EnvelopeVersionUnsupported { version: u8 },
    PeerOffline { peer_device_id: String },
    BackendInternal { message: String },
    CodexSpawnFailed { message: String },
    CodexConnectFailed { url: String, message: String },
    CodexProtocolError { method: String, message: String },
    GeminiSpawnFailed { message: String },
    AcpProtocolError { method: String, message: String },
    AgentAlreadyRunning,
    AgentNotRunning,
    AgentNotSupported { agent: AgentName },
    AgentSessionIdMismatch,
    IngestSeqConflict { session_id: String, seq: u64 },
    SessionNotFound { session_id: String },
    TranslationNotImplemented { agent: AgentName },
    TranslationFailed { agent: AgentName, message: String },
    PairingQrVersionUnsupported { version: u8 },
    Timeout,
    NotConnected,
    RequestDropped,
    AuthRefreshFailed { message: String },
    EmailTaken,
    WeakPassword,
    RateLimited { retry_after_s: u32 },
    InvalidCredentials,
    AgentStartFailed { reason: String },
    PairingTokenExpired,
}

// ─────────────────────────── mirrored protocol types ──────────────────────────

#[allow(dead_code)]
#[frb(mirror(ListSessionsParams))]
pub struct _ListSessionsParams {
    pub limit: u32,
    pub before_ts_ms: Option<i64>,
    pub agent: Option<AgentName>,
}

#[allow(dead_code)]
#[frb(mirror(ListSessionsResponse))]
pub struct _ListSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    pub next_before_ts_ms: Option<i64>,
}

#[allow(dead_code)]
#[frb(mirror(ReadSessionParams))]
pub struct _ReadSessionParams {
    pub session_id: String,
    pub from_seq: Option<u64>,
    pub limit: u32,
}

#[allow(dead_code)]
#[frb(mirror(ReadSessionResponse))]
pub struct _ReadSessionResponse {
    pub ui_events: Vec<UiEventMessage>,
    pub next_seq: Option<u64>,
    pub session_end_reason: Option<SessionEndReason>,
}

#[allow(dead_code)]
#[frb(mirror(SessionSummary))]
pub struct _SessionSummary {
    pub session_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<SessionEndReason>,
    pub parent_session_id: Option<String>,
    pub state: SessionState,
    pub needs_continue: bool,
}

pub struct AgentSessionSummaryDto {
    pub session_id: String,
    pub conversation_id: String,
    pub agent_id: Option<String>,
    pub agent: Option<AgentName>,
    pub status: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub title: Option<String>,
    pub last_activity_at_ms: i64,
    pub message_count: u32,
    pub end_reason: Option<SessionEndReason>,
}

impl From<CoreAgentSessionSummary> for AgentSessionSummaryDto {
    fn from(value: CoreAgentSessionSummary) -> Self {
        Self {
            session_id: value.session_id,
            conversation_id: value.conversation_id,
            agent_id: value.agent_id,
            agent: value.agent,
            status: value.status,
            started_at_ms: value.started_at_ms,
            ended_at_ms: value.ended_at_ms,
            title: value.title,
            last_activity_at_ms: value.last_activity_at_ms,
            message_count: value.message_count,
            end_reason: value.end_reason,
        }
    }
}

#[allow(dead_code)]
#[frb(mirror(MessageRole))]
pub enum _MessageRole {
    User,
    Assistant,
    System,
}

#[allow(dead_code)]
#[frb(mirror(SessionEndReason))]
pub enum _SessionEndReason {
    UserStopped,
    AgentDone,
    Crashed { message: String },
    Timeout,
    HostDisconnected,
}

#[allow(dead_code)]
#[frb(mirror(ArtifactRef))]
pub struct _ArtifactRef {
    pub session_id: String,
    pub artifact_id: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub media_type: String,
}

#[allow(dead_code)]
#[frb(mirror(DisplayPayload))]
pub enum _DisplayPayload {
    Inline {
        text: String,
    },
    StreamingWindow {
        head: String,
        received_bytes: u64,
        artifact: Option<ArtifactRef>,
    },
    WindowedFinal {
        head: String,
        tail: String,
        omitted_bytes: u64,
        artifact: ArtifactRef,
    },
}

#[allow(dead_code)]
#[frb(mirror(SubagentStatus))]
pub enum _SubagentStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

#[allow(dead_code)]
#[frb(mirror(UiEventMessage))]
pub enum _UiEventMessage {
    SessionOpened {
        session_id: String,
        agent: AgentName,
        title: Option<String>,
        opened_at_ms: i64,
    },
    SessionTitleUpdated {
        session_id: String,
        title: String,
    },
    SessionClosed {
        session_id: String,
        reason: SessionEndReason,
        closed_at_ms: i64,
    },
    MessageStarted {
        message_id: String,
        role: MessageRole,
        started_at_ms: i64,
    },
    MessageCompleted {
        message_id: String,
        finished_at_ms: i64,
    },
    TextDelta {
        message_id: String,
        text: DisplayPayload,
    },
    TextReplace {
        message_id: String,
        text: DisplayPayload,
    },
    ReasoningDelta {
        message_id: String,
        text: DisplayPayload,
    },
    ReasoningReplace {
        message_id: String,
        text: DisplayPayload,
    },
    ToolCallPlaced {
        message_id: String,
        tool_call_id: String,
        name: String,
        args_json: DisplayPayload,
    },
    ToolCallCompleted {
        tool_call_id: String,
        output: DisplayPayload,
        is_error: bool,
    },
    SubagentSpawned {
        parent_session_id: String,
        sub_session_id: String,
        tool_call_id: String,
        agent: AgentName,
        model: Option<String>,
        prompt: Option<String>,
        title: Option<String>,
    },
    SubagentStatusUpdated {
        sub_session_id: String,
        status: SubagentStatus,
    },
    Error {
        code: String,
        message: String,
        message_id: Option<String>,
    },
    Raw {
        kind: String,
        payload_json: String,
    },
}

// ─────────────────────── mirrored auth + agent types ─────────────────────────

#[allow(dead_code)]
#[frb(mirror(AuthSummary))]
pub struct _AuthSummary {
    pub account_id: String,
    pub email: String,
}

#[allow(dead_code)]
#[frb(mirror(MyProfileResponse))]
pub struct _MyProfileResponse {
    pub account_id: String,
    pub email: String,
    pub minos_id: String,
    pub display_name: Option<String>,
}

#[allow(dead_code)]
#[frb(mirror(UserSummary))]
pub struct _UserSummary {
    pub account_id: String,
    pub minos_id: String,
    pub display_name: String,
}

#[allow(dead_code)]
#[frb(mirror(FriendRequestStatus))]
pub enum _FriendRequestStatus {
    Pending,
    Accepted,
    Rejected,
    Canceled,
}

#[allow(dead_code)]
#[frb(mirror(FriendRequestSummary))]
pub struct _FriendRequestSummary {
    pub request_id: String,
    pub from: UserSummary,
    pub to: UserSummary,
    pub status: FriendRequestStatus,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
}

#[allow(dead_code)]
#[frb(mirror(FriendRequestsResponse))]
pub struct _FriendRequestsResponse {
    pub incoming: Vec<FriendRequestSummary>,
    pub outgoing: Vec<FriendRequestSummary>,
}

#[allow(dead_code)]
#[frb(mirror(FriendSummary))]
pub struct _FriendSummary {
    pub account_id: String,
    pub minos_id: String,
    pub display_name: String,
    pub created_at_ms: i64,
}

#[allow(dead_code)]
#[frb(mirror(FriendsResponse))]
pub struct _FriendsResponse {
    pub friends: Vec<FriendSummary>,
}

#[allow(dead_code)]
#[frb(mirror(ConversationKind))]
pub enum _ConversationKind {
    Direct,
    Group,
}

#[allow(dead_code)]
#[frb(mirror(ConversationSummary))]
pub struct _ConversationSummary {
    pub conversation_id: String,
    pub kind: ConversationKind,
    pub title: String,
    pub counterpart: Option<UserSummary>,
    pub member_count: u32,
    pub last_message_preview: Option<String>,
    pub last_message_at_ms: i64,
    pub unread_count: u32,
    pub unread_mention_count: u32,
}

#[allow(dead_code)]
#[frb(mirror(ConversationsResponse))]
pub struct _ConversationsResponse {
    pub conversations: Vec<ConversationSummary>,
}

#[allow(dead_code)]
#[frb(mirror(ConversationResponse))]
pub struct _ConversationResponse {
    pub conversation_id: String,
}

#[allow(dead_code)]
#[frb(mirror(ConversationMembersResponse))]
pub struct _ConversationMembersResponse {
    pub members: Vec<UserSummary>,
}

#[allow(dead_code)]
#[frb(mirror(AgentSummary))]
pub struct _AgentSummary {
    pub agent_id: String,
    pub owner_account_id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub avatar_url: Option<String>,
    pub source: String,
    pub status: String,
    pub runtime_agent: String,
    pub model: String,
    pub default_reasoning_effort: String,
    pub system_prompt: String,
    pub workspace_path: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[allow(dead_code)]
#[frb(mirror(ListAgentsResponse))]
pub struct _ListAgentsResponse {
    pub agents: Vec<AgentSummary>,
}

#[allow(dead_code)]
#[frb(mirror(ConversationAgentMembersResponse))]
pub struct _ConversationAgentMembersResponse {
    pub agents: Vec<AgentSummary>,
}

#[allow(dead_code)]
#[frb(mirror(ConversationParticipantsResponse))]
pub struct _ConversationParticipantsResponse {
    pub humans: Vec<UserSummary>,
    pub agents: Vec<AgentSummary>,
}

#[allow(dead_code)]
#[frb(mirror(ConversationReadResponse))]
pub struct _ConversationReadResponse {
    pub last_read_seq: Option<i64>,
    pub last_read_at_ms: Option<i64>,
}

#[allow(dead_code)]
#[frb(mirror(MessageSender))]
pub enum _MessageSender {
    Account {
        account_id: String,
        minos_id: String,
        display_name: String,
    },
    Bot {
        bot_id: String,
        display_name: String,
        runtime_agent: String,
        name: Option<String>,
        avatar_url: Option<String>,
    },
}

#[allow(dead_code)]
#[frb(mirror(ChatMessageReplySummary))]
pub struct _ChatMessageReplySummary {
    pub message_id: String,
    pub sender: MessageSender,
    pub text: String,
    pub recalled_at_ms: Option<i64>,
}

#[allow(dead_code)]
#[frb(mirror(ReactionActor))]
pub struct _ReactionActor {
    pub actor_id: String,
    pub actor_kind: String,
    pub display_name: String,
}

#[allow(dead_code)]
#[frb(mirror(ReactionGroup))]
pub struct _ReactionGroup {
    pub emoji: String,
    pub count: u32,
    pub reacted_by_me: bool,
    pub actors: Vec<ReactionActor>,
}

#[allow(dead_code)]
#[frb(mirror(ToggleReactionResponse))]
pub struct _ToggleReactionResponse {
    pub message_id: String,
    pub conversation_id: String,
    pub reactions: Vec<ReactionGroup>,
    pub action: String,
}

#[allow(dead_code)]
#[frb(mirror(ChatMessageAttachment))]
pub struct _ChatMessageAttachment {
    pub blob_id: String,
    pub content_type: String,
    pub byte_size: i64,
    pub kind: String,
    pub original_filename: Option<String>,
}

#[allow(dead_code)]
#[frb(mirror(ChatMessageSummary))]
pub struct _ChatMessageSummary {
    pub message_id: String,
    pub conversation_id: String,
    pub sender: MessageSender,
    pub text: String,
    pub created_at_ms: i64,
    pub message_seq: i64,
    pub reply_to: Option<ChatMessageReplySummary>,
    pub recalled_at_ms: Option<i64>,
    pub mentioned_account_ids: Vec<String>,
    pub mentioned_agent_ids: Vec<String>,
    pub sender_type: SenderType,
    pub reactions: Vec<ReactionGroup>,
    pub attachments: Vec<ChatMessageAttachment>,
}

#[allow(dead_code)]
#[frb(mirror(SenderType))]
pub enum _SenderType {
    User,
    Agent,
}

#[allow(dead_code)]
#[frb(mirror(ListChatMessagesResponse))]
pub struct _ListChatMessagesResponse {
    pub messages: Vec<ChatMessageSummary>,
    pub next_before_seq: Option<i64>,
}

#[allow(dead_code)]
#[frb(mirror(StartAgentResponse))]
pub struct _StartAgentResponse {
    pub session_id: String,
    pub cwd: String,
}

#[allow(dead_code)]
#[frb(mirror(HostSkillSummary))]
pub struct _HostSkillSummary {
    pub name: String,
    pub path: String,
    pub description: String,
    pub enabled: bool,
    pub scope: String,
    pub display_name: Option<String>,
    pub short_description: Option<String>,
}

#[allow(dead_code)]
#[frb(mirror(HostSkillError))]
pub struct _HostSkillError {
    pub path: String,
    pub message: String,
}

#[allow(dead_code)]
#[frb(mirror(HostSkillsEntry))]
pub struct _HostSkillsEntry {
    pub cwd: String,
    pub errors: Vec<HostSkillError>,
    pub skills: Vec<HostSkillSummary>,
}

#[allow(dead_code)]
#[frb(mirror(ListHostSkillsResponse))]
pub struct _ListHostSkillsResponse {
    pub data: Vec<HostSkillsEntry>,
}

#[allow(dead_code)]
#[frb(mirror(HostWorkspaceSummary))]
pub struct _HostWorkspaceSummary {
    pub path: String,
    pub display_name: String,
    pub is_git_repo: bool,
}

#[allow(dead_code)]
#[frb(mirror(ListHostWorkspacesResponse))]
pub struct _ListHostWorkspacesResponse {
    pub root: String,
    pub workspaces: Vec<HostWorkspaceSummary>,
}

#[allow(dead_code)]
#[frb(mirror(WriteHostSkillConfigResponse))]
pub struct _WriteHostSkillConfigResponse {
    pub effective_enabled: bool,
}

// ─────────────────────── mirrored project types ──────────────────────────

#[allow(dead_code)]
#[frb(mirror(ProjectSummary))]
pub struct _ProjectSummary {
    pub project_id: String,
    pub name: String,
    pub workspace_slug: String,
    pub workspace_path: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub thread_count: u32,
}

#[allow(dead_code)]
#[frb(mirror(CreateProjectRequest))]
pub struct _CreateProjectRequest {
    pub name: String,
    pub workspace_slug: String,
    pub workspace_path: Option<String>,
}

#[allow(dead_code)]
#[frb(mirror(CreateProjectResponse))]
pub struct _CreateProjectResponse {
    pub project: ProjectSummary,
}

#[allow(dead_code)]
#[frb(mirror(UpdateProjectRequest))]
pub struct _UpdateProjectRequest {
    pub project_id: String,
    pub name: String,
}

#[allow(dead_code)]
#[frb(mirror(DeleteProjectRequest))]
pub struct _DeleteProjectRequest {
    pub project_id: String,
}

#[allow(dead_code)]
#[frb(mirror(ListProjectsResponse))]
pub struct _ListProjectsResponse {
    pub projects: Vec<ProjectSummary>,
}

#[allow(dead_code)]
#[frb(mirror(ListProjectSessionsParams))]
pub struct _ListProjectSessionsParams {
    pub project_id: String,
    pub limit: u32,
    pub before_ts_ms: Option<i64>,
}

#[allow(dead_code)]
#[frb(mirror(ListProjectSessionsResponse))]
pub struct _ListProjectSessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn state_forwarder_spawns_without_current_runtime() {
        let (tx, rx) = watch::channel(ConnectionState::Disconnected);

        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "test must start outside a tokio runtime"
        );

        let (state_tx, state_rx) = mpsc::channel();
        spawn_state_forwarder(rx, move |state| state_tx.send(state).map_err(|_| ()));

        assert_eq!(
            state_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            ConnectionState::Disconnected
        );

        tx.send(ConnectionState::Pairing).unwrap();
        assert_eq!(
            state_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            ConnectionState::Pairing
        );
    }
}
