//! Request and response payload types.

use minos_domain::{AgentDescriptor, AgentName, DeviceId, PairingToken};
use minos_ui_protocol::{SessionEndReason, UiEventMessage};
use serde::{Deserialize, Serialize};

/// Response body for `GET /v1/me/peer` — the backend's view of the
/// host caller's currently paired mobile peer. Returned by the
/// authenticated device-secret rail (`X-Device-Id` + `X-Device-Secret`)
/// so a freshly reconnected daemon can refresh its in-memory peer mirror
/// without reading anything from local disk.
///
/// On `200`, the body carries the mobile peer's `device_id`, display
/// name, and the most-recent account-host pairing timestamp (epoch ms).
/// On `404` with `error.code == "not_paired"`, the caller has no row in
/// `account_host_pairings` — the response body uses the standard
/// `{ "error": { "code": ..., "message": ... } }` envelope shared by
/// every other `/v1/*` route.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MePeerResponse {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
    pub paired_at_ms: i64,
}

/// Response body for `GET /v1/me/macs`. iOS callers receive every Mac
/// paired to their `account_id`. `paired_via_device_id` is the mobile
/// device that performed the scan — recorded for audit; not used for
/// routing.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MeHostsResponse {
    pub hosts: Vec<HostSummary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HostSummary {
    pub host_device_id: DeviceId,
    pub host_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: DeviceId,
    #[serde(default)]
    pub online: bool,
}

/// Response body for `GET /v1/me/peers`. Host callers receive every
/// mobile/account pair currently associated with their `host_device_id`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MePeersResponse {
    pub peers: Vec<HostPeerSummary>,
}

/// One mobile/account row connected to a host.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HostPeerSummary {
    pub mobile_device_id: DeviceId,
    pub mobile_device_name: String,
    pub account_email: String,
    pub paired_at_ms: i64,
    pub last_active_at_ms: i64,
    pub online: bool,
}

/// Bearer-authenticated mobile account profile.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MyProfileResponse {
    pub account_id: String,
    pub email: String,
    pub minos_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Minimal user directory card exposed to the mobile app.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UserSummary {
    pub account_id: String,
    pub minos_id: String,
    pub display_name: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SearchUsersResponse {
    pub users: Vec<UserSummary>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SearchUsersRequest {
    pub minos_id: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SetMinosIdRequest {
    pub minos_id: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SetDisplayNameRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateFriendRequestRequest {
    pub target_minos_id: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestStatus {
    Pending,
    Accepted,
    Rejected,
    Canceled,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FriendRequestSummary {
    pub request_id: String,
    pub from: UserSummary,
    pub to: UserSummary,
    pub status: FriendRequestStatus,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<i64>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FriendRequestsResponse {
    pub incoming: Vec<FriendRequestSummary>,
    pub outgoing: Vec<FriendRequestSummary>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FriendSummary {
    pub account_id: String,
    pub minos_id: String,
    pub display_name: String,
    pub created_at_ms: i64,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FriendsResponse {
    pub friends: Vec<FriendSummary>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Direct,
    Group,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub kind: ConversationKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterpart: Option<UserSummary>,
    pub member_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    pub last_message_at_ms: i64,
    pub unread_count: u32,
    pub unread_mention_count: u32,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationsResponse {
    pub conversations: Vec<ConversationSummary>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EnsureDirectConversationRequest {
    pub friend_account_id: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateGroupConversationRequest {
    pub title: String,
    pub member_account_ids: Vec<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationResponse {
    pub conversation_id: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationMembersResponse {
    pub members: Vec<UserSummary>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationReadResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_at_ms: Option<i64>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChatMessageReplySummary {
    pub message_id: String,
    pub sender: UserSummary,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recalled_at_ms: Option<i64>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChatMessageSummary {
    pub message_id: String,
    pub conversation_id: String,
    pub sender: UserSummary,
    pub text: String,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ChatMessageReplySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recalled_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentioned_account_ids: Vec<String>,
    /// Distinguishes user-sent messages from agent-sent messages.
    /// Defaults to "user" for backward compatibility.
    #[serde(default = "default_sender_type")]
    pub sender_type: SenderType,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListChatMessagesResponse {
    pub messages: Vec<ChatMessageSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before_ts_ms: Option<i64>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListChatMessagesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_ts_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SendChatMessageRequest {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
}

// ─── Agent in Group Chat ───────────────────────────────────────────────

/// The type of sender for a chat message.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SenderType {
    User,
    Agent,
}

fn default_sender_type() -> SenderType {
    SenderType::User
}

/// Request to register a new agent under the caller's account.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RegisterAgentRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub runtime_agent: String,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
}

/// Request to update an existing agent owned by the caller.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateAgentRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub runtime_agent: String,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
}

/// Summary of a registered agent.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AgentSummary {
    pub agent_id: String,
    pub owner_account_id: String,
    pub name: String,
    pub description: String,
    pub runtime_agent: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Response for listing agents owned by the caller.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListAgentsResponse {
    pub agents: Vec<AgentSummary>,
}

/// Request to add an agent to a group conversation.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AddAgentToGroupRequest {
    pub agent_id: String,
}

/// Request to remove an agent from a group conversation.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RemoveAgentFromGroupRequest {
    pub agent_id: String,
}

/// Request to add a user member to an existing group conversation.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AddGroupMemberRequest {
    pub member_account_id: String,
}

/// Request to remove a user member from an existing group conversation.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RemoveGroupMemberRequest {
    pub member_account_id: String,
}

/// Response listing agent members of a conversation.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationAgentMembersResponse {
    pub agents: Vec<AgentSummary>,
}

/// Request for an agent to send a message in a group conversation.
/// The agent_id identifies which agent is "speaking".
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SendAgentMessageRequest {
    pub agent_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairRequest {
    pub device_id: DeviceId,
    pub name: String,
    /// One-shot pairing token presented in the QR. Validated server-side
    /// against the daemon's currently-active token before any state is
    /// mutated; spec §6.4. Required by MVP.
    pub token: PairingToken,
}

/// Result of `POST /v1/pairings` (consume). iOS no longer receives a
/// device secret — the rail is bearer-only post ADR-0020. Mac-side
/// pair state is delivered separately via `EventKind::Paired`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairResponse {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
}

/// Request body for `POST /v1/pairing/consume`. Distinct from
/// [`PairRequest`] because the HTTP route derives `device_id` from the
/// `X-Device-Id` header, not the body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairConsumeRequest {
    pub token: PairingToken,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub version: String,
    pub uptime_secs: u64,
}

/// Account-side request to target one paired host for a CLI scan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListHostClisRequest {
    pub host_installation_id: String,
}

pub type ListClisResponse = Vec<AgentDescriptor>;

/// Parameters for the `list_host_skills` RPC. `workspace` is optional for
/// mobile clients that still rely on the daemon's default workspace.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListHostSkillsRequest {
    pub workspace: String,
    #[serde(default)]
    pub force_reload: bool,
}

/// Account-side request to inspect skills on one paired host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListHostSkillsCommandRequest {
    pub host_installation_id: String,
    pub workspace: String,
    #[serde(default)]
    pub force_reload: bool,
}

/// Parameters for listing host-side workspace directories. `root` is optional;
/// hosts default it to the current user's home directory and constrain custom
/// roots to that home tree.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListHostWorkspacesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default)]
    pub limit: u32,
}

/// Account-side request to inspect workspace directories on one paired host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListHostWorkspacesCommandRequest {
    pub host_installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default)]
    pub limit: u32,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostWorkspaceSummary {
    pub path: String,
    pub display_name: String,
    pub is_git_repo: bool,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListHostWorkspacesResponse {
    pub root: String,
    pub workspaces: Vec<HostWorkspaceSummary>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostSkillSummary {
    pub name: String,
    pub path: String,
    pub description: String,
    pub enabled: bool,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostSkillError {
    pub path: String,
    pub message: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostSkillsEntry {
    pub cwd: String,
    pub errors: Vec<HostSkillError>,
    pub skills: Vec<HostSkillSummary>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListHostSkillsResponse {
    pub data: Vec<HostSkillsEntry>,
}

/// Parameters for the `write_host_skill_config` RPC. The path comes from
/// a prior `list_host_skills` result.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteHostSkillConfigRequest {
    pub workspace: String,
    pub path: String,
    pub enabled: bool,
}

/// Account-side request to update one host skill toggle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteHostSkillConfigCommandRequest {
    pub host_installation_id: String,
    pub workspace: String,
    pub path: String,
    pub enabled: bool,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteHostSkillConfigResponse {
    pub effective_enabled: bool,
}

/// Which codex driver to spawn — selectable at start time so dev/test
/// surfaces can compare the two paths side-by-side without rebuilding.
/// `Jsonl` is the production default (`codex exec --json` per turn); `Server`
/// spawns `codex app-server --listen ws://…` and connects via WebSocket.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum AgentLaunchMode {
    #[default]
    #[serde(rename = "jsonl")]
    Jsonl,
    #[serde(rename = "server")]
    Server,
}

/// Parameters for the `start_agent` RPC. See spec §5.2.
///
/// # Profile resolution (latest-only)
///
/// 1. When `profile_id` is set: load the host agent profile. `agent` **must**
///    equal `profile.runtime_agent` (clear error on mismatch). Launch fields
///    start from the profile's model / reasoning_effort / instructions.
/// 2. Explicit `model` / `reasoning_effort` / `instructions` on the request
///    override the corresponding profile fields when provided (non-empty).
///    Precedence: **explicit request > profile > None**.
/// 3. When `profile_id` is absent: only the explicit request fields apply
///    (same as pre-profile behavior).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartAgentRequest {
    pub agent: AgentName,
    /// Workspace directory the codex app-server child should treat as its
    /// `cwd`. Multi-session manager keys instances by workspace, so two
    /// `start_agent` calls for the same workspace share an instance and
    /// distinct calls for different workspaces spawn distinct codex children.
    /// Carried as a string for FFI portability (UniFFI does not lift `PathBuf`).
    pub workspace: String,
    /// Optional driver selector. Absent ⇒ `Server` post-Phase-C, preserving
    /// the wire shape for clients that pre-date the field. The `Jsonl` variant
    /// is retained for compatibility but no longer drives a JSONL exec path —
    /// it is silently treated as `Server`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<AgentLaunchMode>,
    /// Host agent profile to bind at create time. See struct-level resolution order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// Fixed model id for this session (create-time only; not mid-session switch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Fixed reasoning effort when the runtime supports it (e.g. low/medium/high).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Extra system / developer instructions for this session (create-time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Result of a successful `start_agent` RPC — carries the codex `session_id`
/// as `session_id` and the resolved workspace path. See spec §5.2.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartAgentResponse {
    pub session_id: String,
    pub cwd: String,
}

/// Parameters for the `send_user_message` RPC. `session_id` must match the
/// active session's id; see spec §5.2 and §5.4.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendUserMessageRequest {
    pub session_id: String,
    pub text: String,
}

/// Server → Host. Unified dispatch payload for agent-bound chat messages.
/// `session_id = None` instructs the host to auto-create a session before
/// sending `text`; otherwise the existing session should receive the message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDispatchRequest {
    pub agent: AgentName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

/// Host → Server response for [`AgentDispatchRequest`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDispatchResponse {
    pub session_id: String,
}

/// Mobile → Server → Host. User resolution for a pending approval request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalDecisionRequest {
    pub request_id: String,
    pub session_id: String,
    pub decision: serde_json::Value,
}

/// Parameters for the `interrupt_session` RPC. See spec §5.2.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterruptSessionRequest {
    pub session_id: String,
}

/// Parameters for the `close_session` RPC. See spec §5.2.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloseSessionRequest {
    pub session_id: String,
}

/// Parameters for the `get_session` RPC. See spec §5.2.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetSessionParams {
    pub session_id: String,
}

/// Parameters for local `resume_session`. Reattach only by default; optional
/// `auto_continue` injects a one-shot CONTINUE prompt when the store flag is set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResumeSessionRequest {
    pub session_id: String,
    /// When true, after reattach, if `needs_continue` is set, inject CONTINUE once.
    /// Send paths must leave this false so user text wins.
    #[serde(default)]
    pub auto_continue: bool,
}

/// Mirror of `minos_agent_runtime::SessionState` published over the wire for
/// the host's JSON-RPC surface. Kept structurally identical to the runtime
/// enum (same `tag = "kind"` / `snake_case` shape) so the two serialise
/// interchangeably across the relay. Not exposed to UniFFI — the FFI surface
/// uses `minos_agent_runtime::SessionState` directly so Swift sees one
/// canonical `SessionState` type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SessionState {
    Starting,
    Idle,
    Running { turn_started_at_ms: i64 },
    Suspended { reason: PauseReason },
    Resuming,
    Closed { reason: CloseReason },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    UserInterrupt,
    CodexCrashed,
    DaemonRestart,
    InstanceReaped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    UserClose,
    TerminalError,
}

/// Response from the `get_session` RPC. Wraps the existing `SessionSummary`
/// metadata with the live `SessionState` snapshot so the mobile UI can both
/// render the history list entry and decide whether to draw the running
/// indicator without a second round-trip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetSessionResponse {
    pub thread: SessionSummary,
    pub state: SessionState,
}

/// Deep-link QR payload minted by the Mac and scanned by iOS. Carries a
/// display name for the host, a short-lived one-shot `pairing_token`, and
/// its RFC-3339-ish epoch-ms expiry. The backend URL lives in the mobile
/// client's compile-time build config (see `minos_mobile::build_config`);
/// it is not a transit value and never enters the QR payload, durable
/// storage, or the post-pair business logic.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PairingQrPayload {
    #[serde(default = "default_pairing_qr_version")]
    pub v: u8,
    pub host_display_name: String,
    #[serde(alias = "token")]
    pub pairing_token: String,
    #[serde(default)]
    pub expires_at_ms: i64,
}

const fn default_pairing_qr_version() -> u8 {
    2
}

/// Parameters for `request_pairing_qr` — the Mac tells the backend which
/// display name to embed so the iPhone UI can say "Pair with <name>?".
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RequestPairingQrParams {
    pub host_display_name: String,
}

/// Response from `request_pairing_qr`; wraps a [`PairingQrPayload`] for
/// the Mac to render directly as a QR code.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RequestPairingQrResponse {
    pub qr_payload: PairingQrPayload,
}

/// Compact summary of one persisted session, returned by `list_sessions`
/// for the mobile history list.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
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
    /// Host should offer a one-shot continue turn after process-death recovery.
    #[serde(default)]
    pub needs_continue: bool,
}

/// Parameters for `list_sessions`. `before_ts_ms` paginates older entries;
/// `agent` filters by CLI kind.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListSessionsParams {
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_ts_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentName>,
}

/// Response from `list_sessions`; `next_before_ts_ms` is set iff there is
/// a strictly older page the caller can request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_before_ts_ms: Option<i64>,
}

/// Parameters for `read_session`. `from_seq` resumes from after the given
/// sequence; if omitted, the backend returns the oldest `limit` events.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReadSessionParams {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_seq: Option<u64>,
    pub limit: u32,
}

/// Response from `read_session`. `next_seq` is set iff more events exist
/// past the returned window. `session_end_reason` is set iff the session is
/// closed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReadSessionResponse {
    pub ui_events: Vec<UiEventMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_end_reason: Option<SessionEndReason>,
}

/// Parameters for `get_session_last_seq` (host-only helper).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GetSessionLastSeqParams {
    pub session_id: String,
}

/// Response from `get_session_last_seq`; `last_seq` is `0` when the thread
/// is unknown or empty.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GetSessionLastSeqResponse {
    pub last_seq: u64,
}

// ─── Projects ──────────────────────────────────────────────────────────

/// Summary of a project for list views.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    pub project_id: String,
    pub name: String,
    pub workspace_slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub thread_count: u32,
}

/// Request to create a new project.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectRequest {
    pub name: String,
    pub workspace_slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
}

/// Response from creating a project.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectResponse {
    pub project: ProjectSummary,
}

/// Request to update a project's name.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateProjectRequest {
    pub project_id: String,
    pub name: String,
}

/// Request to delete a project.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DeleteProjectRequest {
    pub project_id: String,
}

/// Request to attach an existing backend thread to a project.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AssignProjectThreadRequest {
    pub project_id: String,
    pub session_id: String,
}

/// Response from listing projects.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListProjectsResponse {
    pub projects: Vec<ProjectSummary>,
}

fn default_conversation_progress() -> String {
    "todo".to_string()
}

/// Local TUI conversation list item. This is separate from the social/cloud
/// `ConversationSummary` type near the top of this file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalConversationSummary {
    pub conversation_id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message_count: u32,
    pub agent_session_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participating_agents: Vec<AgentName>,
    /// Roster with optional peer-facing briefs (SSOT for multi-agent coordination).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roster: Vec<ConversationRosterMember>,
    /// User priority: `high` | `medium` | `low`. Absent = unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Workflow progress: `todo` | `in_progress` | `in_review` | `done`.
    #[serde(default = "default_conversation_progress")]
    pub progress: String,
    /// Git branch for this conversation work unit (live when refreshed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Linked worktree path when the conversation uses an isolated worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// How this conversation binds to git: `inherit` | `worktree`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_mode: Option<String>,
    /// Cached dirty flag from last git status refresh (working tree + untracked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_dirty: Option<bool>,
    /// Cached short/full HEAD from last git status refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_head: Option<String>,
    /// Live agent sessions in starting/running/resuming (list-time aggregate).
    #[serde(default)]
    pub running_count: u32,
    /// Live agent sessions needing human attention (suspended / approval).
    #[serde(default)]
    pub needs_attention_count: u32,
}

/// Agent/thread mention attached to a local conversation message.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationMention {
    pub agent: AgentName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_short_id: Option<String>,
}

/// One actor on a local conversation message reaction.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalReactionActor {
    pub actor_id: String,
    /// `user` | `agent`
    pub actor_kind: String,
    pub display_name: String,
}

/// Aggregated reaction group for one emoji on a message.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalReactionGroup {
    pub emoji: String,
    pub count: u32,
    /// True when the host local user (`actor_id = "local"`) is among actors.
    pub reacted_by_me: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<LocalReactionActor>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalConversationMessage {
    pub message_seq: i64,
    pub message_id: String,
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub created_at_ms: i64,
    pub sender_role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentName>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<ConversationMention>,
    /// Aggregated emoji reactions (durable local daemon). Empty when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<LocalReactionGroup>,
    /// Structured git milestone when this message embeds a git activity payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_activity: Option<GitActivity>,
}

/// Idempotent toggle: if local actor already reacted with `emoji`, remove; else add.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ToggleConversationMessageReactionParams {
    pub message_id: String,
    pub emoji: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ToggleConversationMessageReactionResponse {
    pub message_id: String,
    pub conversation_id: String,
    pub reactions: Vec<LocalReactionGroup>,
}

/// Stable host-local actor for desktop single-user reactions.
pub const LOCAL_REACTION_ACTOR_ID: &str = "local";
pub const LOCAL_REACTION_ACTOR_KIND: &str = "user";
pub const LOCAL_REACTION_DISPLAY_NAME: &str = "You";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationsParams {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_updated_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationsResponse {
    pub conversations: Vec<LocalConversationSummary>,
}

// ── Host-local git (conversation work units) ──────────────────────────────

/// Structured git milestone posted into a conversation timeline.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitActivity {
    WorktreeCreated {
        branch: String,
        worktree_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_branch: Option<String>,
    },
    CommitsMade {
        count: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        subjects: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        head: Option<String>,
    },
    PrOpened {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        number: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    ChecksFailed {
        summary: String,
    },
    ReadyForReview {
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        head: Option<String>,
    },
    Merged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        merge_commit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
    },
}

/// Resolve which checkout to inspect: conversation work unit, project, or raw path.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitStatusParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Absolute path override (advanced). Ignored when conversation_id is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// When true and conversation_id is set, persist branch/dirty/head back to the row.
    #[serde(default)]
    pub refresh_conversation: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitStatusResponse {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_head: Option<String>,
    pub dirty: bool,
    pub has_untracked: bool,
    pub ahead_count: u32,
    pub behind_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub is_linked_worktree: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<LocalConversationSummary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitDiffParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Base ref (default HEAD for worktree diff, or merge-base style left side).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Head ref; omit or `WORKTREE` for working-tree diff against base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitDiffFile {
    pub path: String,
    pub status: String,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitDiffResponse {
    pub path: String,
    pub base: String,
    pub head: String,
    pub files: Vec<GitDiffFile>,
    pub patch: String,
    pub truncated: bool,
    pub file_count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitCreateWorktreeParams {
    pub conversation_id: String,
    /// When true, replace an existing conversation worktree binding.
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitCreateWorktreeResponse {
    pub conversation: LocalConversationSummary,
    pub created: bool,
    pub branch: String,
    pub worktree_path: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitRemoveWorktreeParams {
    pub conversation_id: String,
    /// When true, also delete the worktree directory from disk.
    #[serde(default = "default_true")]
    pub delete_files: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitRemoveWorktreeResponse {
    pub conversation: LocalConversationSummary,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitEnsureIdentityParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitEnsureIdentityResponse {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub complete: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitPushBranchParams {
    pub conversation_id: String,
    /// Remote name (default `origin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// When true, pass `--set-upstream`.
    #[serde(default = "default_true")]
    pub set_upstream: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitPushBranchResponse {
    pub branch: String,
    pub remote: String,
    pub head: Option<String>,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitOpenPullRequestParams {
    pub conversation_id: String,
    /// PR title (default: conversation title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// PR body markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Base branch (default: remote HEAD / main).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// When true, create as draft.
    #[serde(default)]
    pub draft: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitOpenPullRequestResponse {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

/// Post a structured git milestone into a conversation (daemon / MCP).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PostGitUpdateParams {
    pub conversation_id: String,
    pub activity: GitActivity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentName>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PostGitUpdateResponse {
    pub message_seq: i64,
    pub message_id: String,
    pub body: String,
}

/// One roster member at conversation create (runtime + optional brief).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationAgentSpec {
    /// Runtime agent label (`codex` / `claude` / …).
    pub agent: String,
    /// Optional short peer-facing role description (≤500 chars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
}

/// Durable roster row returned on conversation summaries / list_conversation_roster.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConversationRosterMember {
    pub agent: AgentName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    pub joined_at_ms: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateConversationParams {
    pub project_id: String,
    pub title: String,
    /// Optional user priority at create: `high` | `medium` | `low`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Runtime agent roster for this conversation.
    /// Only members may be @mentioned or started in the conversation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<ConversationAgentSpec>,
    /// Git isolation mode: `worktree` (default when project is a git repo) or
    /// `inherit` (use project workspace as-is). Unknown values are rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_mode: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationRosterParams {
    pub conversation_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationRosterResponse {
    pub conversation_id: String,
    pub members: Vec<ConversationRosterMember>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateConversationResponse {
    pub conversation: LocalConversationSummary,
}

/// Patch conversation product metadata (title / priority / progress).
/// Omitted fields are left unchanged. For `priority`, send empty string to clear.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateConversationParams {
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `high` | `medium` | `low`, or empty string to clear. Absent = leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// `todo` | `in_progress` | `in_review` | `done`. Absent = leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateConversationResponse {
    pub conversation: LocalConversationSummary,
}

/// Add a runtime agent to a conversation roster (idempotent on agent label).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AddConversationAgentParams {
    pub conversation_id: String,
    /// Runtime agent label (`codex` / `claude` / …).
    pub agent: String,
    /// Optional peer-facing role brief (≤500 chars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AddConversationAgentResponse {
    pub conversation: LocalConversationSummary,
}

/// Remove a runtime agent from a conversation roster.
/// Existing sessions for that agent are closed with reason `roster_removed`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RemoveConversationAgentParams {
    pub conversation_id: String,
    /// Runtime agent label (`codex` / `claude` / …).
    pub agent: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RemoveConversationAgentResponse {
    pub conversation: LocalConversationSummary,
    /// Sessions closed because the agent left the roster.
    pub closed_session_ids: Vec<String>,
    /// Running teamwork delegations cancelled because the agent left.
    pub cancelled_delegation_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationMessagesParams {
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationMessagesResponse {
    pub messages: Vec<LocalConversationMessage>,
    pub has_more: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationAgentSessionsParams {
    pub conversation_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListConversationAgentSessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

/// Start an agent session bound to a conversation.
///
/// Profile resolution matches [`StartAgentRequest`]: when `profile_id` is set,
/// `agent` must match the profile runtime; explicit model/effort/instructions
/// override profile fields; without `profile_id` only explicit fields apply.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct StartAgentInConversationRequest {
    pub conversation_id: String,
    pub agent: AgentName,
    pub workspace: String,
    /// Host agent profile to bind at create time. See [`StartAgentRequest`] resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// Fixed model id for this session (create-time only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Fixed reasoning effort when the runtime supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Extra system / developer instructions for this session (create-time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

// ── Host agent profiles (desktop local-first personalized agents) ─────────

/// One model entry returned by `list_models` for a runtime CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_reasoning_efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListModelsRequest {
    pub runtime: AgentName,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListModelsResponse {
    pub runtime: AgentName,
    pub models: Vec<ModelInfo>,
    /// Discovery path: app_server | acp | cli | static
    pub source: String,
}

/// Host-local personalized agent (fixed runtime + model + effort at create).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub runtime_agent: AgentName,
    pub model: String,
    pub reasoning_effort: String,
    /// Extra system prompt / developer instructions appended at session start.
    #[serde(default)]
    pub instructions: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateAgentProfileRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub runtime_agent: AgentName,
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default)]
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateAgentProfileRequest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Allowed to revise instructions after create (model/runtime/effort stay fixed).
    #[serde(default)]
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteAgentProfileRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListAgentProfilesResponse {
    pub profiles: Vec<AgentProfileSummary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppendConversationMessageParams {
    pub conversation_id: String,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub sender_role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentName>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<ConversationMention>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppendConversationMessageResponse {
    pub message_seq: i64,
}

/// Parameters for listing sessions within a project.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListProjectSessionsParams {
    pub project_id: String,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_ts_ms: Option<i64>,
}

/// Response from listing project sessions.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListProjectSessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pair_request_round_trip() {
        let req = PairRequest {
            device_id: DeviceId::new(),
            name: "iPhone of fan".into(),
            token: PairingToken::generate(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: PairRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn pair_response_round_trip() {
        let resp = PairResponse {
            peer_device_id: DeviceId::new(),
            peer_name: "MacBook".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["peer_device_id"],
            serde_json::to_value(resp.peer_device_id).unwrap()
        );
        assert_eq!(value["peer_name"], serde_json::json!("MacBook"));
        assert!(value.get("your_device_secret").is_none());
        let back: PairResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn pair_response_no_secret_field_round_trip() {
        let resp = PairResponse {
            peer_device_id: DeviceId::new(),
            peer_name: "iPhone".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            value.get("your_device_secret").is_none(),
            "secret must not appear"
        );
        let back: PairResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn me_hosts_response_round_trips() {
        let hosts = MeHostsResponse {
            hosts: vec![HostSummary {
                host_device_id: DeviceId::new(),
                host_display_name: "Mac-mini".into(),
                paired_at_ms: 1_714_000_000_000,
                paired_via_device_id: DeviceId::new(),
                online: true,
            }],
        };
        let json = serde_json::to_string(&hosts).unwrap();
        let back: MeHostsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, hosts);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["hosts"].is_array());
    }

    #[test]
    fn search_users_request_round_trip() {
        let req = SearchUsersRequest {
            minos_id: "fan123".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SearchUsersRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn list_chat_messages_request_round_trip() {
        let req = ListChatMessagesRequest {
            before_ts_ms: Some(42),
            limit: Some(50),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ListChatMessagesRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn local_conversation_message_reactions_default_empty_and_skip() {
        let msg = LocalConversationMessage {
            message_seq: 1,
            message_id: "m1".into(),
            conversation_id: "c1".into(),
            session_id: None,
            created_at_ms: 10,
            sender_role: "user".into(),
            agent: None,
            body: "hi".into(),
            reply_to_message_id: None,
            delegation_id: None,
            mentions: vec![],
            reactions: vec![],
            git_activity: None,
        };
        let value = serde_json::to_value(&msg).unwrap();
        assert!(
            value.get("reactions").is_none(),
            "empty reactions must be omitted"
        );
        let back: LocalConversationMessage = serde_json::from_value(value).unwrap();
        assert!(back.reactions.is_empty());
    }

    #[test]
    fn toggle_reaction_params_round_trip() {
        let req = ToggleConversationMessageReactionParams {
            message_id: "msg-1".into(),
            emoji: "👍".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ToggleConversationMessageReactionParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn toggle_reaction_response_round_trip() {
        let resp = ToggleConversationMessageReactionResponse {
            message_id: "msg-1".into(),
            conversation_id: "c1".into(),
            reactions: vec![LocalReactionGroup {
                emoji: "👍".into(),
                count: 1,
                reacted_by_me: true,
                actors: vec![LocalReactionActor {
                    actor_id: LOCAL_REACTION_ACTOR_ID.into(),
                    actor_kind: LOCAL_REACTION_ACTOR_KIND.into(),
                    display_name: LOCAL_REACTION_DISPLAY_NAME.into(),
                }],
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ToggleConversationMessageReactionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn health_response_round_trip() {
        let resp = HealthResponse {
            version: "0.1.0".into(),
            uptime_secs: 42,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: HealthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn list_host_clis_request_round_trip() {
        let req = ListHostClisRequest {
            host_installation_id: "host-123".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ListHostClisRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn list_host_skills_command_request_round_trip() {
        let req = ListHostSkillsCommandRequest {
            host_installation_id: "host-123".into(),
            workspace: "/tmp/workspace".into(),
            force_reload: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ListHostSkillsCommandRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn write_host_skill_config_command_request_round_trip() {
        let req = WriteHostSkillConfigCommandRequest {
            host_installation_id: "host-123".into(),
            workspace: String::new(),
            path: "/tmp/skill".into(),
            enabled: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: WriteHostSkillConfigCommandRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn start_agent_request_round_trip() {
        let req = StartAgentRequest {
            agent: AgentName::Codex,
            workspace: "/Users/fan/dev".into(),
            mode: None,
            profile_id: None,
            model: None,
            reasoning_effort: None,
            instructions: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        // Default-mode payload omits the optional `mode` field.
        assert!(!json.contains("mode"));
        // Workspace is mandatory post-Phase-C and must serialize.
        assert!(json.contains("workspace"));
        let back: StartAgentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn start_agent_request_with_mode_round_trip() {
        let req = StartAgentRequest {
            agent: AgentName::Codex,
            workspace: "/Users/fan/dev".into(),
            mode: Some(AgentLaunchMode::Server),
            profile_id: None,
            model: None,
            reasoning_effort: None,
            instructions: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"mode\":\"server\""));
        let back: StartAgentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn start_agent_request_with_profile_id_round_trip() {
        let req = StartAgentRequest {
            agent: AgentName::Grok,
            workspace: "/w".into(),
            mode: None,
            profile_id: Some("profile-abc".into()),
            model: Some("override-model".into()),
            reasoning_effort: None,
            instructions: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"profile_id\":\"profile-abc\""));
        let back: StartAgentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn start_agent_request_pre_mode_payload_decodes() {
        let json = r#"{"agent":"codex","workspace":"/w"}"#;
        let req: StartAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent, AgentName::Codex);
        assert_eq!(req.workspace, "/w");
        assert_eq!(req.mode, None);
        assert_eq!(req.profile_id, None);
    }

    #[test]
    fn start_agent_in_conversation_request_with_profile_round_trip() {
        let req = StartAgentInConversationRequest {
            conversation_id: "c1".into(),
            agent: AgentName::Claude,
            workspace: "/w".into(),
            profile_id: Some("profile-1".into()),
            model: None,
            reasoning_effort: Some("high".into()),
            instructions: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: StartAgentInConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn start_agent_response_round_trip() {
        let resp = StartAgentResponse {
            session_id: "thread-abc12".into(),
            cwd: "/Users/fan/.minos/workspaces".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: StartAgentResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn send_user_message_request_round_trip() {
        let req = SendUserMessageRequest {
            session_id: "thread-abc12".into(),
            text: "ping".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SendUserMessageRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn agent_dispatch_request_round_trip() {
        let req = AgentDispatchRequest {
            agent: AgentName::Codex,
            session_id: Some("thread-abc12".into()),
            text: "continue with tests".into(),
            workspace: "/Users/fan/dev/minos".into(),
            approval_policy: Some("on_request".into()),
            sandbox_policy: Some("workspace_write".into()),
            conversation_id: Some("conv-123".into()),
            origin_message_id: Some("msg-456".into()),
            model: Some("gpt-5.4".into()),
            reasoning_effort: Some("high".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: AgentDispatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn agent_dispatch_request_omits_none_fields() {
        let req = AgentDispatchRequest {
            agent: AgentName::Claude,
            session_id: None,
            text: "start a new session".into(),
            workspace: String::new(),
            approval_policy: None,
            sandbox_policy: None,
            conversation_id: None,
            origin_message_id: None,
            model: None,
            reasoning_effort: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["agent"], serde_json::json!("claude"));
        assert_eq!(value["workspace"], serde_json::json!(""));
        assert!(value.get("session_id").is_none());
        assert!(value.get("approval_policy").is_none());
        assert!(value.get("sandbox_policy").is_none());
        assert!(value.get("conversation_id").is_none());
        assert!(value.get("origin_message_id").is_none());
        assert!(value.get("model").is_none());
        assert!(value.get("reasoning_effort").is_none());
    }

    #[test]
    fn agent_dispatch_response_round_trip() {
        let resp = AgentDispatchResponse {
            session_id: "thread-abc12".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: AgentDispatchResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn approval_decision_request_round_trip() {
        let req = ApprovalDecisionRequest {
            request_id: "req-123".into(),
            session_id: "thread-abc12".into(),
            decision: serde_json::json!({
                "decision": "approve",
                "scope": "once",
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ApprovalDecisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }
}

#[cfg(test)]
mod new_type_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pairing_qr_payload_ignores_legacy_unknown_fields() {
        // Legacy QR payloads may still carry `backend_url` and CF Access
        // fields — `serde` ignores unknown fields by default, so this is
        // a forward-compat read of older Mac builds. The struct itself no
        // longer carries them. The legacy display-name alias was dropped
        // in Phase B; new payloads must use `host_display_name`.
        let back: PairingQrPayload = serde_json::from_value(serde_json::json!({
            "backend_url": "wss://minos.fan-nn.top/devices",
            "host_display_name": "Mac",
            "token": "tok"
        }))
        .unwrap();

        assert_eq!(back.v, 2);
        assert_eq!(back.host_display_name, "Mac");
        assert_eq!(back.pairing_token, "tok");
        assert_eq!(back.expires_at_ms, 0);
    }

    #[test]
    fn request_pairing_qr_params_round_trip() {
        let p = RequestPairingQrParams {
            host_display_name: "Fan's Mac".into(),
        };
        let back: RequestPairingQrParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn request_pairing_qr_response_round_trip() {
        let r = RequestPairingQrResponse {
            qr_payload: PairingQrPayload {
                v: 1,
                host_display_name: "Mac".into(),
                pairing_token: "tok".into(),
                expires_at_ms: 42,
            },
        };
        let back: RequestPairingQrResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn thread_summary_round_trip_with_end_reason() {
        let s = SessionSummary {
            session_id: "thr_1".into(),
            agent: AgentName::Codex,
            title: Some("A thread".into()),
            first_ts_ms: 100,
            last_ts_ms: 200,
            message_count: 3,
            ended_at_ms: Some(300),
            end_reason: Some(SessionEndReason::AgentDone),
            parent_session_id: None,
            state: SessionState::Closed {
                reason: CloseReason::UserClose,
            },
            needs_continue: false,
        };
        let back: SessionSummary =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn thread_summary_round_trip_open_thread() {
        let s = SessionSummary {
            session_id: "thr_2".into(),
            agent: AgentName::Claude,
            title: None,
            first_ts_ms: 100,
            last_ts_ms: 200,
            message_count: 1,
            ended_at_ms: None,
            end_reason: None,
            parent_session_id: Some("parent".into()),
            state: SessionState::Idle,
            needs_continue: true,
        };
        let back: SessionSummary =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn list_sessions_params_round_trip_filters() {
        let p = ListSessionsParams {
            limit: 50,
            before_ts_ms: Some(1_000),
            agent: Some(AgentName::Gemini),
        };
        let back: ListSessionsParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn list_sessions_params_round_trip_omits_none_fields() {
        let p = ListSessionsParams {
            limit: 10,
            before_ts_ms: None,
            agent: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(!s.contains("before_ts_ms"));
        assert!(!s.contains("agent"));
    }

    #[test]
    fn list_sessions_response_round_trip() {
        let r = ListSessionsResponse {
            sessions: vec![SessionSummary {
                session_id: "thr_1".into(),
                agent: AgentName::Codex,
                title: None,
                first_ts_ms: 1,
                last_ts_ms: 2,
                message_count: 0,
                ended_at_ms: None,
                end_reason: None,
                parent_session_id: None,
                state: SessionState::Idle,
                needs_continue: false,
            }],
            next_before_ts_ms: Some(1),
        };
        let back: ListSessionsResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn read_session_params_round_trip() {
        let p = ReadSessionParams {
            session_id: "thr_1".into(),
            from_seq: Some(10),
            limit: 100,
        };
        let back: ReadSessionParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn read_session_response_round_trip() {
        let r = ReadSessionResponse {
            ui_events: vec![UiEventMessage::TextDelta {
                message_id: "msg_1".into(),
                text: "Hi".into(),
            }],
            next_seq: Some(2),
            session_end_reason: Some(SessionEndReason::AgentDone),
        };
        let back: ReadSessionResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn get_session_last_seq_params_round_trip() {
        let p = GetSessionLastSeqParams {
            session_id: "thr_1".into(),
        };
        let back: GetSessionLastSeqParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn get_session_last_seq_response_round_trip() {
        let r = GetSessionLastSeqResponse { last_seq: 42 };
        let back: GetSessionLastSeqResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn me_peer_response_round_trip() {
        let r = MePeerResponse {
            peer_device_id: DeviceId::new(),
            peer_name: "fan's iPhone".into(),
            paired_at_ms: 1_726_500_000_000,
        };
        let json = serde_json::to_string(&r).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["peer_device_id"],
            serde_json::to_value(r.peer_device_id).unwrap()
        );
        assert_eq!(value["peer_name"], serde_json::json!("fan's iPhone"));
        assert_eq!(
            value["paired_at_ms"],
            serde_json::json!(1_726_500_000_000_i64)
        );
        let back: MePeerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
