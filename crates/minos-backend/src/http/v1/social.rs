use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;

/// Maximum time window (in milliseconds) within which a message can be
/// recalled. 5 minutes matches common IM conventions.
const RECALL_WINDOW_MS: i64 = 5 * 60 * 1000;

/// Placeholder text for recalled messages (stored in DB).
#[allow(dead_code)]
const RECALLED_MESSAGE_TEXT: &str = "[message recalled]";

/// Default title for unnamed group conversations.
const DEFAULT_GROUP_TITLE: &str = "Group Chat";
const AGENT_DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);
const GROUP_COMPLETION_TIMEOUT: Duration = Duration::from_mins(5);
const GROUP_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(100);
use axum::{Json, Router};
use minos_domain::AgentName;
use minos_protocol::{
    AddAgentToGroupRequest, AddGroupMemberRequest, AgentDispatchRequest, AgentDispatchResponse,
    AgentSummary, ChatMessageReplySummary, ChatMessageSummary, ConversationAgentMembersResponse,
    ConversationKind, ConversationMembersResponse, ConversationReadResponse, ConversationResponse,
    ConversationSummary, ConversationsResponse, CreateFriendRequestRequest,
    CreateGroupConversationRequest, EnsureDirectConversationRequest, Envelope, EventKind,
    FriendRequestStatus, FriendRequestSummary, FriendRequestsResponse, FriendSummary,
    FriendsResponse, ListAgentsResponse, ListChatMessagesRequest, ListChatMessagesResponse,
    MyProfileResponse, RegisterAgentRequest, RemoveAgentFromGroupRequest, SearchUsersRequest,
    SearchUsersResponse, SendAgentMessageRequest, SendChatMessageRequest, SenderType,
    SetDisplayNameRequest, SetMinosIdRequest, UserSummary,
};
use serde::Serialize;

use crate::auth::bearer;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/me/profile/query", post(get_my_profile))
        .route("/me/profile/minos-id", post(set_minos_id))
        .route("/me/profile/display-name", post(set_display_name))
        .route("/users/search/query", post(search_users_query))
        .route("/friends/query", post(list_friends))
        .route("/friend-requests", post(create_friend_request))
        .route("/friend-requests/query", post(list_friend_requests))
        .route(
            "/friend-requests/:request_id/accept",
            post(accept_friend_request),
        )
        .route(
            "/friend-requests/:request_id/reject",
            post(reject_friend_request),
        )
        .route("/conversations/query", post(list_conversations))
        .route("/conversations/direct", post(ensure_direct_conversation))
        .route("/conversations/group", post(create_group_conversation))
        .route(
            "/conversations/:conversation_id/members/query",
            post(list_conversation_members),
        )
        .route(
            "/conversations/:conversation_id/read",
            post(mark_conversation_read),
        )
        .route(
            "/conversations/:conversation_id/messages",
            post(send_message),
        )
        .route(
            "/conversations/:conversation_id/messages/query",
            post(list_messages_query),
        )
        .route(
            "/conversations/:conversation_id/messages/:message_id/recall",
            post(recall_message),
        )
        // ─── Agent routes ───
        .route("/agents", post(register_agent))
        .route("/agents/query", post(list_agents))
        .route("/agents/:agent_id/delete", post(delete_agent_handler))
        .route(
            "/conversations/:conversation_id/members/add",
            post(add_group_member),
        )
        .route(
            "/conversations/:conversation_id/agents",
            post(list_conversation_agents_handler),
        )
        .route(
            "/conversations/:conversation_id/agents/add",
            post(add_agent_to_group),
        )
        .route(
            "/conversations/:conversation_id/agents/remove",
            post(remove_agent_from_group),
        )
        .route(
            "/conversations/:conversation_id/agents/message",
            post(send_agent_message),
        )
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

fn err(code: &'static str, message: impl Into<String>) -> (StatusCode, Json<ErrorEnvelope>) {
    (
        status_for(code),
        Json(ErrorEnvelope {
            error: ErrorBody {
                code,
                message: message.into(),
            },
        }),
    )
}

fn status_for(code: &str) -> StatusCode {
    match code {
        "unauthorized" => StatusCode::UNAUTHORIZED,
        "not_found" => StatusCode::NOT_FOUND,
        "conflict" => StatusCode::CONFLICT,
        "bad_request" => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn validate_minos_id(minos_id: &str) -> bool {
    let len = minos_id.len();
    (6..=24).contains(&len) && minos_id.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[allow(clippy::unused_async)]
async fn require_account_id(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<ErrorEnvelope>)> {
    let bearer = bearer::require(state, headers).map_err(|e| {
        let (status, message) = e.into_response_tuple();
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: "unauthorized",
                    message,
                },
            }),
        )
    })?;
    Ok(bearer.account_id)
}

fn display_name(profile: &crate::store::social::ProfileRow) -> String {
    if let Some(name) = profile.display_name.as_deref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let email = profile.email.trim();
    match email.split('@').next() {
        Some(head) if !head.is_empty() => head.to_string(),
        _ => profile.minos_id.clone(),
    }
}

fn to_user_summary(profile: &crate::store::social::ProfileRow) -> UserSummary {
    UserSummary {
        account_id: profile.account_id.clone(),
        minos_id: profile.minos_id.clone(),
        display_name: display_name(profile),
    }
}

async fn load_profile(
    state: &BackendState,
    account_id: &str,
) -> Result<crate::store::social::ProfileRow, (StatusCode, Json<ErrorEnvelope>)> {
    crate::store::social::profile_by_account(&state.store, account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", format!("account not found: {account_id}")))
}

async fn get_my_profile(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<MyProfileResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let profile = load_profile(&state, &account_id).await?;
    Ok(Json(MyProfileResponse {
        account_id: profile.account_id,
        email: profile.email,
        minos_id: profile.minos_id,
        display_name: profile.display_name,
    }))
}

async fn set_minos_id(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<SetMinosIdRequest>,
) -> Result<Json<MyProfileResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !validate_minos_id(&req.minos_id) {
        return Err(err(
            "bad_request",
            "minos_id must be 6-24 ASCII letters or digits",
        ));
    }
    crate::store::social::set_minos_id(&state.store, &account_id, &req.minos_id)
        .await
        .map_err(|e| {
            if matches!(
                &e,
                crate::error::BackendError::StoreQuery { operation, message }
                if operation == "social::set_minos_id" && message == "minos_id_taken"
            ) {
                err("conflict", "minos_id already taken")
            } else {
                err("internal", e.to_string())
            }
        })?;
    let profile = load_profile(&state, &account_id).await?;
    Ok(Json(MyProfileResponse {
        account_id: profile.account_id,
        email: profile.email,
        minos_id: profile.minos_id,
        display_name: profile.display_name,
    }))
}

async fn set_display_name(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<SetDisplayNameRequest>,
) -> Result<Json<MyProfileResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let next_display_name = req
        .display_name
        .as_ref()
        .map(|raw| raw.trim().to_string())
        .filter(|trimmed| !trimmed.is_empty());
    if let Some(name) = next_display_name.as_ref() {
        let char_count = name.chars().count();
        if !(1..=48).contains(&char_count) {
            return Err(err(
                "bad_request",
                "display_name must be 1-48 characters after trimming",
            ));
        }
    }
    crate::store::social::set_display_name(&state.store, &account_id, next_display_name.as_deref())
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let profile = load_profile(&state, &account_id).await?;
    Ok(Json(MyProfileResponse {
        account_id: profile.account_id,
        email: profile.email,
        minos_id: profile.minos_id,
        display_name: profile.display_name,
    }))
}

async fn search_users_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(query): Json<SearchUsersRequest>,
) -> Result<Json<SearchUsersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    search_users_inner(state, headers, query).await
}

async fn search_users_inner(
    state: BackendState,
    headers: HeaderMap,
    query: SearchUsersRequest,
) -> Result<Json<SearchUsersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if query.minos_id.trim().is_empty() {
        return Ok(Json(SearchUsersResponse { users: Vec::new() }));
    }
    let users = crate::store::social::search_by_minos_id_prefix(&state.store, &query.minos_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .into_iter()
        .filter(|user| user.account_id != account_id)
        .map(|user| to_user_summary(&user))
        .collect();
    Ok(Json(SearchUsersResponse { users }))
}

async fn create_friend_request(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<CreateFriendRequestRequest>,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let me = load_profile(&state, &account_id).await?;
    let Some(target) = crate::store::social::find_by_minos_id(&state.store, &req.target_minos_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    else {
        return Err(err("not_found", "target user not found"));
    };
    if target.account_id == account_id {
        return Err(err("bad_request", "cannot add yourself"));
    }
    if crate::store::social::are_friends(&state.store, &account_id, &target.account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("conflict", "already friends"));
    }
    if crate::store::social::has_pending_friend_request_between(
        &state.store,
        &account_id,
        &target.account_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("conflict", "friend request already pending"));
    }
    let created_at_ms = chrono::Utc::now().timestamp_millis();
    let request_id = crate::store::social::create_friend_request(
        &state.store,
        &account_id,
        &target.account_id,
        created_at_ms,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(Json(FriendRequestSummary {
        request_id,
        from: to_user_summary(&me),
        to: to_user_summary(&target),
        status: FriendRequestStatus::Pending,
        created_at_ms,
        resolved_at_ms: None,
    }))
}

async fn list_friend_requests(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<FriendRequestsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let incoming_rows =
        crate::store::social::list_incoming_friend_requests(&state.store, &account_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let outgoing_rows =
        crate::store::social::list_outgoing_friend_requests(&state.store, &account_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let incoming = hydrate_friend_requests(&state, incoming_rows).await?;
    let outgoing = hydrate_friend_requests(&state, outgoing_rows).await?;
    Ok(Json(FriendRequestsResponse { incoming, outgoing }))
}

async fn accept_friend_request(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    resolve_request(state, headers, request_id, FriendRequestStatus::Accepted).await
}

async fn reject_friend_request(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    resolve_request(state, headers, request_id, FriendRequestStatus::Rejected).await
}

async fn resolve_request(
    state: BackendState,
    headers: HeaderMap,
    request_id: String,
    status: FriendRequestStatus,
) -> Result<Json<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let Some(existing) = crate::store::social::get_friend_request(&state.store, &request_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    else {
        return Err(err("not_found", "friend request not found"));
    };
    if existing.to_account_id != account_id {
        return Err(err("unauthorized", "not allowed to resolve this request"));
    }
    let resolved_at_ms = chrono::Utc::now().timestamp_millis();
    let changed = crate::store::social::resolve_friend_request(
        &state.store,
        &request_id,
        status,
        resolved_at_ms,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    if !changed {
        return Err(err("conflict", "friend request already resolved"));
    }
    if status == FriendRequestStatus::Accepted {
        crate::store::social::create_friendship(
            &state.store,
            &existing.from_account_id,
            &existing.to_account_id,
            resolved_at_ms,
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    }
    let row = crate::store::social::get_friend_request(&state.store, &request_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "friend request not found"))?;
    let mut hydrated = hydrate_friend_requests(&state, vec![row]).await?;
    Ok(Json(hydrated.remove(0)))
}

async fn list_friends(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<FriendsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let friendships = crate::store::social::list_friendships_for(&state.store, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;

    // Batch-load all friend profiles in one query.
    let friend_ids: Vec<String> = friendships
        .iter()
        .map(|f| {
            if f.account_low_id == account_id {
                f.account_high_id.clone()
            } else {
                f.account_low_id.clone()
            }
        })
        .collect();
    let profiles = crate::store::social::profiles_by_accounts(&state.store, &friend_ids)
        .await
        .map_err(|e| err("internal", e.to_string()))?;

    let mut friends = Vec::with_capacity(friendships.len());
    for friendship in friendships {
        let other_id = if friendship.account_low_id == account_id {
            &friendship.account_high_id
        } else {
            &friendship.account_low_id
        };
        let profile = profiles
            .get(other_id)
            .ok_or_else(|| err("internal", format!("profile not found: {other_id}")))?;
        let friend_display_name = display_name(profile);
        friends.push(FriendSummary {
            account_id: profile.account_id.clone(),
            minos_id: profile.minos_id.clone(),
            display_name: friend_display_name,
            created_at_ms: friendship.created_at_ms,
        });
    }
    Ok(Json(FriendsResponse { friends }))
}

async fn list_conversations(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ConversationsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let rows = crate::store::social::list_conversations_for(&state.store, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let conversations = hydrate_conversations(&state, &account_id, rows).await?;
    Ok(Json(ConversationsResponse { conversations }))
}

async fn ensure_direct_conversation(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<EnsureDirectConversationRequest>,
) -> Result<Json<ConversationResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::are_friends(&state.store, &account_id, &req.friend_account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("conflict", "users are not friends"));
    }
    let conversation = crate::store::social::ensure_direct_conversation(
        &state.store,
        &account_id,
        &account_id,
        &req.friend_account_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(Json(ConversationResponse {
        conversation_id: conversation.conversation_id,
    }))
}

async fn create_group_conversation(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<CreateGroupConversationRequest>,
) -> Result<Json<ConversationResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let title = req.title.trim();
    if title.is_empty() {
        return Err(err("bad_request", "group title is required"));
    }
    for member in &req.member_account_ids {
        if member == &account_id {
            continue;
        }
        if !crate::store::social::are_friends(&state.store, &account_id, member)
            .await
            .map_err(|e| err("internal", e.to_string()))?
        {
            return Err(err(
                "conflict",
                format!("group member is not your friend: {member}"),
            ));
        }
    }
    let conversation = crate::store::social::create_group_conversation(
        &state.store,
        &account_id,
        title,
        &req.member_account_ids,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(Json(ConversationResponse {
        conversation_id: conversation.conversation_id,
    }))
}

async fn list_conversation_members(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationMembersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }

    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?
            .into_iter()
            .map(|profile| to_user_summary(&profile))
            .collect();

    Ok(Json(ConversationMembersResponse { members }))
}

async fn mark_conversation_read(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationReadResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }

    let last_read_at_ms = crate::store::social::mark_conversation_read_to_latest(
        &state.store,
        &conversation_id,
        &account_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;

    Ok(Json(ConversationReadResponse { last_read_at_ms }))
}

async fn list_messages_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(query): Json<ListChatMessagesRequest>,
) -> Result<Json<ListChatMessagesResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    list_messages_inner(state, headers, conversation_id, query).await
}

async fn list_messages_inner(
    state: BackendState,
    headers: HeaderMap,
    conversation_id: String,
    query: ListChatMessagesRequest,
) -> Result<Json<ListChatMessagesResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    let limit = query.limit.unwrap_or(50);
    let mut messages = crate::store::social::list_messages(
        &state.store,
        &conversation_id,
        query.before_ts_ms,
        limit,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    #[allow(clippy::cast_possible_truncation)]
    let next_before_ts_ms = if messages.len() as u32 == limit.min(200) {
        messages.last().map(|message| message.created_at_ms)
    } else {
        None
    };
    messages.reverse();
    let messages = hydrate_messages(&state, messages).await?;
    Ok(Json(ListChatMessagesResponse {
        messages,
        next_before_ts_ms,
    }))
}

#[allow(clippy::too_many_lines)]
async fn send_message(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<SendChatMessageRequest>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let trimmed = req.text.trim().to_string();
    if trimmed.is_empty() {
        return Err(err("bad_request", "message text is required"));
    }
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    let reply_target = match req.reply_to_message_id.as_deref() {
        Some(message_id) => {
            let Some(reply_target) = crate::store::social::get_message(&state.store, message_id)
                .await
                .map_err(|e| err("internal", e.to_string()))?
            else {
                return Err(err("bad_request", "reply target not found"));
            };
            if reply_target.conversation_id != conversation_id {
                return Err(err("bad_request", "reply target not in conversation"));
            }
            Some(reply_target)
        }
        None => None,
    };
    let reply_to_message_id = reply_target.as_ref().map(|row| row.message_id.clone());
    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let mentioned_account_ids = extract_mentioned_account_ids(&trimmed, &account_id, &members);
    let row = crate::store::social::insert_message(
        &state.store,
        &conversation_id,
        &account_id,
        &trimmed,
        chrono::Utc::now().timestamp_millis(),
        reply_to_message_id.as_deref(),
        &mentioned_account_ids,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    let mut hydrated = hydrate_messages(&state, vec![row]).await?;
    let message = hydrated.remove(0);

    let sender_minos_id = members
        .iter()
        .find(|member| member.account_id == account_id)
        .map(|member| member.minos_id.clone())
        .unwrap_or_default();
    let dispatch_plan =
        build_agent_dispatch_plan(&state, &conversation_id, &trimmed, reply_target.as_ref())
            .await?;

    fan_out_social_message(&state, &message).await;

    if let Some(plan) = dispatch_plan {
        match forward_agent_dispatch(
            &state,
            &plan.agent,
            plan.session_id.clone(),
            &plan.forwarded_text,
            &conversation_id,
            &message.message_id,
        )
        .await
        {
            Ok(response) => {
                crate::store::social::bind_session_to_message(
                    &state.store,
                    &message.message_id,
                    &response.session_id,
                )
                .await
                .map_err(|e| err("internal", e.to_string()))?;

                spawn_group_completion_watcher(
                    state.clone(),
                    conversation_id.clone(),
                    message.message_id.clone(),
                    response.session_id,
                    plan.agent,
                    plan.watcher_from_seq,
                    if plan.mention_sender {
                        Some(account_id.clone())
                    } else {
                        None
                    },
                    if plan.mention_sender {
                        Some(sender_minos_id.clone())
                    } else {
                        None
                    },
                );
            }
            Err(error) => {
                let (code, detail) = agent_error_from_backend_error(&error);
                fan_out_agent_error(&state, &account_id, plan.session_id, code, detail);
            }
        }
    }

    Ok(Json(message))
}

async fn recall_message(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }

    let Some(existing) = crate::store::social::get_message(&state.store, &message_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    else {
        return Err(err("not_found", "message not found"));
    };
    if existing.conversation_id != conversation_id {
        return Err(err("not_found", "message not found"));
    }
    if existing.sender_account_id != account_id {
        return Err(err(
            "bad_request",
            "only the sender can recall this message",
        ));
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    if now_ms - existing.created_at_ms > RECALL_WINDOW_MS {
        return Err(err(
            "bad_request",
            "message recall window has expired (5 minutes)",
        ));
    }

    let recalled = crate::store::social::recall_message(
        &state.store,
        &conversation_id,
        &message_id,
        &account_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?
    .ok_or_else(|| err("not_found", "message not found"))?;

    let mut hydrated = hydrate_messages(&state, vec![recalled]).await?;
    let message = hydrated.remove(0);
    fan_out_social_message(&state, &message).await;
    Ok(Json(message))
}

async fn fan_out_social_message(state: &BackendState, message: &ChatMessageSummary) {
    let members = match crate::store::social::list_conversation_members(
        &state.store,
        &message.conversation_id,
    )
    .await
    {
        Ok(members) => members,
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::social",
                conversation_id = %message.conversation_id,
                error = %error,
                "failed to list conversation members for social fan-out"
            );
            return;
        }
    };

    let frame = Envelope::Event {
        version: 1,
        event: EventKind::SocialMessage {
            conversation_id: message.conversation_id.clone(),
            message: message.clone(),
        },
    };

    for account_id in members {
        let _ = state
            .registry
            .broadcast_mobile_account(&account_id, frame.clone());
    }
}

#[derive(Clone)]
struct AgentDispatchPlan {
    agent: crate::store::social::AgentRow,
    session_id: Option<String>,
    forwarded_text: String,
    watcher_from_seq: u64,
    mention_sender: bool,
}

async fn build_agent_dispatch_plan(
    state: &BackendState,
    conversation_id: &str,
    text: &str,
    reply_target: Option<&crate::store::social::ChatMessageRow>,
) -> Result<Option<AgentDispatchPlan>, (StatusCode, Json<ErrorEnvelope>)> {
    let conversation = crate::store::social::get_conversation(&state.store, conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    let agents = crate::store::social::list_conversation_agents(&state.store, conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    if agents.is_empty() {
        return Ok(None);
    }
    let human_members =
        crate::store::social::list_conversation_members(&state.store, conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let mention_sender = human_members.len() > 1;

    if let Some(reply_target) = reply_target {
        if reply_target.sender_type == "agent" {
            if let Some(session_id) = crate::store::social::lookup_session_id_for_message(
                &state.store,
                &reply_target.message_id,
            )
            .await
            .map_err(|e| err("internal", e.to_string()))?
            {
                let agent_id = reply_target
                    .sender_agent_id
                    .as_deref()
                    .unwrap_or(&reply_target.sender_account_id);
                if let Some(agent) = crate::store::social::get_agent(&state.store, agent_id)
                    .await
                    .map_err(|e| err("internal", e.to_string()))?
                {
                    let watcher_from_seq =
                        crate::store::raw_events::last_seq(&state.store, &session_id)
                            .await
                            .map_err(|e| err("internal", e.to_string()))?;
                    return Ok(Some(AgentDispatchPlan {
                        agent,
                        session_id: Some(session_id),
                        forwarded_text: text.to_string(),
                        watcher_from_seq,
                        mention_sender,
                    }));
                }
            }
        }
    }

    if conversation.kind == "group" && human_members.len() == 1 && agents.len() == 1 {
        let session_id = crate::store::social::lookup_latest_session_id_for_conversation(
            &state.store,
            conversation_id,
        )
        .await
        .map_err(|e| err("internal", e.to_string()))?;
        let watcher_from_seq = if let Some(ref session_id) = session_id {
            crate::store::raw_events::last_seq(&state.store, session_id)
                .await
                .map_err(|e| err("internal", e.to_string()))?
        } else {
            0
        };
        return Ok(Some(AgentDispatchPlan {
            agent: agents[0].clone(),
            session_id,
            forwarded_text: text.to_string(),
            watcher_from_seq,
            mention_sender: false,
        }));
    }

    if conversation.kind == "group" {
        if let Some(agent) = first_mentioned_agent(text, &agents) {
            return Ok(Some(AgentDispatchPlan {
                agent: agent.clone(),
                session_id: None,
                forwarded_text: strip_agent_mention_once(text, &agent.agent_id),
                watcher_from_seq: 0,
                mention_sender: true,
            }));
        }
    }

    Ok(None)
}

async fn forward_agent_dispatch(
    state: &BackendState,
    agent: &crate::store::social::AgentRow,
    session_id: Option<String>,
    text: &str,
    conversation_id: &str,
    origin_message_id: &str,
) -> Result<AgentDispatchResponse, crate::error::BackendError> {
    let host_device_id = select_live_host_for_account(state, &agent.owner_account_id).await?;
    let request = AgentDispatchRequest {
        agent: parse_runtime_agent_name(&agent.runtime_agent)?,
        session_id,
        text: text.to_string(),
        workspace: String::new(),
        approval_policy: None,
        sandbox_policy: None,
        conversation_id: Some(conversation_id.to_string()),
        origin_message_id: Some(origin_message_id.to_string()),
    };
    crate::forward_rpc::call_host(
        &state.registry,
        host_device_id,
        "minos_agent_dispatch",
        &request,
        AGENT_DISPATCH_TIMEOUT,
    )
    .await
}

async fn select_live_host_for_account(
    state: &BackendState,
    account_id: &str,
) -> Result<minos_domain::DeviceId, crate::error::BackendError> {
    let hosts =
        crate::store::account_host_pairings::list_hosts_for_account(&state.store, account_id)
            .await?;
    for host in hosts {
        if state.registry.get(host.host_device_id).is_some() {
            return Ok(host.host_device_id);
        }
    }
    Err(crate::error::BackendError::ForwardRpc {
        method: "minos_agent_dispatch".into(),
        message: format!("no live host paired to account {account_id}"),
    })
}

fn parse_runtime_agent_name(runtime_agent: &str) -> Result<AgentName, crate::error::BackendError> {
    match runtime_agent {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        other => Err(crate::error::BackendError::ForwardRpc {
            method: "minos_agent_dispatch".into(),
            message: format!("unsupported runtime agent `{other}`"),
        }),
    }
}

fn first_mentioned_agent(
    text: &str,
    agents: &[crate::store::social::AgentRow],
) -> Option<crate::store::social::AgentRow> {
    let by_id = agents
        .iter()
        .map(|agent| (agent.agent_id.as_str(), agent))
        .collect::<HashMap<_, _>>();
    collect_mention_tokens(text)
        .into_iter()
        .find_map(|token| by_id.get(token).copied().cloned())
}

fn strip_agent_mention_once(text: &str, agent_id: &str) -> String {
    let stripped = text.replacen(&format!("@{agent_id}"), "", 1);
    let normalised = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalised.is_empty() {
        text.to_string()
    } else {
        normalised
    }
}

fn agent_error_from_backend_error(error: &crate::error::BackendError) -> (&'static str, String) {
    match error {
        crate::error::BackendError::PeerOffline { .. } => {
            ("peer_offline", "agent host is offline".to_string())
        }
        crate::error::BackendError::PeerBackpressure { .. } => {
            ("peer_backpressure", "agent host is busy".to_string())
        }
        crate::error::BackendError::ForwardRpcTimeout { .. } => (
            "dispatch_timeout",
            "agent host did not reply in time".to_string(),
        ),
        crate::error::BackendError::ForwardRpc { message, .. } => {
            ("dispatch_failed", message.clone())
        }
        other => ("dispatch_failed", other.to_string()),
    }
}

fn fan_out_agent_error(
    state: &BackendState,
    account_id: &str,
    session_id: Option<String>,
    code: &str,
    message: String,
) {
    let frame = Envelope::Event {
        version: 1,
        event: EventKind::AgentError {
            session_id,
            code: code.to_string(),
            message,
        },
    };
    let _ = state.registry.broadcast_mobile_account(account_id, frame);
}

#[allow(clippy::too_many_arguments)]
fn spawn_group_completion_watcher(
    state: BackendState,
    conversation_id: String,
    reply_to_message_id: String,
    session_id: String,
    agent: crate::store::social::AgentRow,
    trigger_seq: u64,
    mention_account_id: Option<String>,
    mention_minos_id: Option<String>,
) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + GROUP_COMPLETION_TIMEOUT;
        loop {
            if tokio::time::Instant::now() >= deadline {
                let timeout_text = mention_minos_id.as_deref().map_or_else(
                    || "Sorry, I timed out waiting for a response.".to_string(),
                    |minos_id| format!("@{minos_id} Sorry, I timed out waiting for a response."),
                );
                let mentions = mention_account_id.iter().cloned().collect::<Vec<_>>();
                let _ = post_agent_social_message(
                    &state,
                    &conversation_id,
                    &agent,
                    &session_id,
                    &reply_to_message_id,
                    &timeout_text,
                    &mentions,
                )
                .await;
                return;
            }

            match find_completed_agent_reply(
                &state.store,
                &session_id,
                agent_name_for_row(&agent),
                trigger_seq,
            )
            .await
            {
                Ok(Some(text)) => {
                    let final_text = mention_minos_id
                        .as_deref()
                        .map_or(text.clone(), |minos_id| format!("@{minos_id} {text}"));
                    let mentions = mention_account_id.iter().cloned().collect::<Vec<_>>();
                    let _ = post_agent_social_message(
                        &state,
                        &conversation_id,
                        &agent,
                        &session_id,
                        &reply_to_message_id,
                        &final_text,
                        &mentions,
                    )
                    .await;
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "minos_backend::social",
                        error = %error,
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        "group completion watcher failed to translate thread state"
                    );
                }
            }

            tokio::time::sleep(GROUP_COMPLETION_POLL_INTERVAL).await;
        }
    });
}

async fn find_completed_agent_reply(
    pool: &sqlx::SqlitePool,
    thread_id: &str,
    agent_name: AgentName,
    trigger_seq: u64,
) -> Result<Option<String>, crate::error::BackendError> {
    let rows = crate::store::raw_events::read_range(pool, thread_id, 1, 10_000).await?;

    match agent_name {
        AgentName::Codex => {
            let mut translator =
                minos_ui_protocol::CodexTranslatorState::new(thread_id.to_string());
            let mut message_texts = HashMap::<String, String>::new();
            for row in rows {
                let events = minos_ui_protocol::translate_codex(&mut translator, &row.payload)
                    .map_err(|error| crate::error::BackendError::ForwardRpc {
                        method: "group_completion_watcher".into(),
                        message: error.to_string(),
                    })?;
                for event in events {
                    match event {
                        minos_ui_protocol::UiEventMessage::MessageStarted {
                            role: minos_ui_protocol::MessageRole::Assistant,
                            message_id,
                            ..
                        } => {
                            message_texts.entry(message_id).or_default();
                        }
                        minos_ui_protocol::UiEventMessage::TextDelta { message_id, text } => {
                            message_texts.entry(message_id).or_default().push_str(&text);
                        }
                        minos_ui_protocol::UiEventMessage::MessageCompleted {
                            message_id, ..
                        } if u64::try_from(row.seq).unwrap_or_default() > trigger_seq => {
                            let text = message_texts.remove(&message_id).unwrap_or_default();
                            return Ok(Some(text.trim().to_string()));
                        }
                        _ => {}
                    }
                }
            }
            Ok(None)
        }
        AgentName::Claude | AgentName::Gemini => Ok(None),
    }
}

fn agent_name_for_row(agent: &crate::store::social::AgentRow) -> AgentName {
    match agent.runtime_agent.as_str() {
        "claude" => AgentName::Claude,
        "gemini" => AgentName::Gemini,
        _ => AgentName::Codex,
    }
}

async fn post_agent_social_message(
    state: &BackendState,
    conversation_id: &str,
    agent: &crate::store::social::AgentRow,
    session_id: &str,
    reply_to_message_id: &str,
    text: &str,
    mentioned_account_ids: &[String],
) -> Result<(), (StatusCode, Json<ErrorEnvelope>)> {
    let row = crate::store::social::insert_agent_message(
        &state.store,
        conversation_id,
        &agent.agent_id,
        text,
        chrono::Utc::now().timestamp_millis(),
        Some(reply_to_message_id),
        mentioned_account_ids,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    crate::store::social::bind_session_to_message(&state.store, &row.message_id, session_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let mut hydrated = hydrate_messages(state, vec![row]).await?;
    let message = hydrated.remove(0);
    fan_out_social_message(state, &message).await;
    Ok(())
}

async fn hydrate_friend_requests(
    state: &BackendState,
    rows: Vec<crate::store::social::FriendRequestRow>,
) -> Result<Vec<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    // Batch-load all referenced profiles in one query to avoid N+1.
    let mut account_ids: Vec<String> = rows
        .iter()
        .flat_map(|r| [r.from_account_id.clone(), r.to_account_id.clone()])
        .collect();
    account_ids.sort();
    account_ids.dedup();
    let profiles = crate::store::social::profiles_by_accounts(&state.store, &account_ids)
        .await
        .map_err(|e| err("internal", e.to_string()))?;

    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let from = profiles.get(&row.from_account_id).ok_or_else(|| {
            err(
                "internal",
                format!("profile not found: {}", row.from_account_id),
            )
        })?;
        let to = profiles.get(&row.to_account_id).ok_or_else(|| {
            err(
                "internal",
                format!("profile not found: {}", row.to_account_id),
            )
        })?;
        output.push(FriendRequestSummary {
            request_id: row.request_id,
            from: to_user_summary(from),
            to: to_user_summary(to),
            status: parse_request_status(&row.status)?,
            created_at_ms: row.created_at_ms,
            resolved_at_ms: row.resolved_at_ms,
        });
    }
    Ok(output)
}

async fn hydrate_conversations(
    state: &BackendState,
    account_id: &str,
    rows: Vec<crate::store::social::ConversationDigestRow>,
) -> Result<Vec<ConversationSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = parse_conversation_kind(&row.kind)?;
        let counterpart = match kind {
            ConversationKind::Direct => {
                let counterpart_id = if row.direct_account_low.as_deref() == Some(account_id) {
                    row.direct_account_high.as_deref()
                } else {
                    row.direct_account_low.as_deref()
                }
                .ok_or_else(|| err("internal", "direct conversation missing counterpart"))?;
                Some(to_user_summary(&load_profile(state, counterpart_id).await?))
            }
            ConversationKind::Group => None,
        };
        let title = match (&kind, &row.title, &counterpart) {
            (ConversationKind::Direct, _, Some(counterpart)) => counterpart.display_name.clone(),
            (ConversationKind::Group, Some(title), _) if !title.trim().is_empty() => title.clone(),
            (ConversationKind::Group, _, _) => DEFAULT_GROUP_TITLE.into(),
            _ => "Conversation".into(),
        };
        output.push(ConversationSummary {
            conversation_id: row.conversation_id,
            kind,
            title,
            counterpart,
            member_count: u32::try_from(row.member_count).unwrap_or(0),
            last_message_preview: row.last_message_preview,
            last_message_at_ms: row.last_message_at_ms,
            unread_count: u32::try_from(row.unread_count).unwrap_or(0),
            unread_mention_count: u32::try_from(row.unread_mention_count).unwrap_or(0),
        });
    }
    Ok(output)
}

async fn hydrate_messages(
    state: &BackendState,
    rows: Vec<crate::store::social::ChatMessageRow>,
) -> Result<Vec<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let message_ids = rows
        .iter()
        .map(|row| row.message_id.clone())
        .collect::<Vec<_>>();
    let mut mentions_by_message =
        crate::store::social::list_message_mentions(&state.store, &message_ids)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let reply_ids = rows
        .iter()
        .filter_map(|row| row.reply_to_message_id.clone())
        .collect::<Vec<_>>();
    let reply_rows = crate::store::social::list_messages_by_ids(&state.store, &reply_ids)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .into_iter()
        .map(|row| (row.message_id.clone(), row))
        .collect::<HashMap<_, _>>();

    let agent_sender_summary = |agent: crate::store::social::AgentRow| UserSummary {
        account_id: agent.agent_id.clone(),
        minos_id: agent.agent_id,
        display_name: format!("🤖 {}", agent.name),
    };

    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let mentioned_account_ids = mentions_by_message
            .remove(&row.message_id)
            .unwrap_or_default();
        let reply_to = if let Some(reply_id) = row.reply_to_message_id.as_ref() {
            if let Some(reply_row) = reply_rows.get(reply_id).cloned() {
                let reply_sender = if reply_row.sender_type == "agent" {
                    let agent_id = reply_row
                        .sender_agent_id
                        .as_deref()
                        .unwrap_or(&reply_row.sender_account_id);
                    let agent = crate::store::social::get_agent(&state.store, agent_id)
                        .await
                        .map_err(|e| err("internal", e.to_string()))?;
                    agent.map_or(
                        UserSummary {
                            account_id: agent_id.to_string(),
                            minos_id: agent_id.to_string(),
                            display_name: "🤖 Unknown Agent".to_string(),
                        },
                        agent_sender_summary,
                    )
                } else {
                    to_user_summary(&load_profile(state, &reply_row.sender_account_id).await?)
                };
                Some(ChatMessageReplySummary {
                    message_id: reply_row.message_id,
                    sender: reply_sender,
                    text: reply_row.text,
                    recalled_at_ms: reply_row.recalled_at_ms,
                })
            } else {
                None
            }
        } else {
            None
        };
        let (sender, sender_type) = if row.sender_type == "agent" {
            // Agent message: load agent info for sender display
            let agent_id = row
                .sender_agent_id
                .as_deref()
                .unwrap_or(&row.sender_account_id);
            let agent = crate::store::social::get_agent(&state.store, agent_id)
                .await
                .map_err(|e| err("internal", e.to_string()))?;
            match agent {
                Some(agent) => (agent_sender_summary(agent), SenderType::Agent),
                None => (
                    UserSummary {
                        account_id: agent_id.to_string(),
                        minos_id: agent_id.to_string(),
                        display_name: "🤖 Unknown Agent".to_string(),
                    },
                    SenderType::Agent,
                ),
            }
        } else {
            let sender = load_profile(state, &row.sender_account_id).await?;
            (to_user_summary(&sender), SenderType::User)
        };
        output.push(ChatMessageSummary {
            message_id: row.message_id,
            conversation_id: row.conversation_id,
            sender,
            text: row.text,
            created_at_ms: row.created_at_ms,
            reply_to,
            recalled_at_ms: row.recalled_at_ms,
            mentioned_account_ids,
            sender_type,
        });
    }
    Ok(output)
}

fn extract_mentioned_account_ids(
    text: &str,
    sender_account_id: &str,
    members: &[crate::store::social::ProfileRow],
) -> Vec<String> {
    let by_minos_id = members
        .iter()
        .map(|member| (member.minos_id.as_str(), member.account_id.as_str()))
        .collect::<HashMap<_, _>>();
    let mut mentions = BTreeSet::<String>::new();

    for token in collect_mention_tokens(text) {
        let Some(account_id) = by_minos_id.get(token) else {
            continue;
        };
        if *account_id == sender_account_id {
            continue;
        }
        mentions.insert((*account_id).to_string());
    }

    mentions.into_iter().collect()
}

fn collect_mention_tokens(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        if index > 0 && bytes[index - 1].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }

        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-') {
            end += 1;
        }
        if end > start {
            tokens.push(&text[start..end]);
            index = end;
            continue;
        }
        index += 1;
    }

    tokens
}

fn parse_request_status(
    status: &str,
) -> Result<FriendRequestStatus, (StatusCode, Json<ErrorEnvelope>)> {
    match status {
        "pending" => Ok(FriendRequestStatus::Pending),
        "accepted" => Ok(FriendRequestStatus::Accepted),
        "rejected" => Ok(FriendRequestStatus::Rejected),
        "canceled" => Ok(FriendRequestStatus::Canceled),
        _ => Err(err(
            "internal",
            format!("unknown friend request status: {status}"),
        )),
    }
}

fn parse_conversation_kind(
    kind: &str,
) -> Result<ConversationKind, (StatusCode, Json<ErrorEnvelope>)> {
    match kind {
        "direct" => Ok(ConversationKind::Direct),
        "group" => Ok(ConversationKind::Group),
        _ => Err(err(
            "internal",
            format!("unknown conversation kind: {kind}"),
        )),
    }
}

// ─── Agent Handlers ────────────────────────────────────────────────────

async fn register_agent(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RegisterAgentRequest>,
) -> Result<Json<AgentSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(err("bad_request", "agent name is required"));
    }
    let valid_runtimes = ["codex", "claude", "gemini"];
    if !valid_runtimes.contains(&req.runtime_agent.as_str()) {
        return Err(err("bad_request", "invalid runtime_agent"));
    }
    let row = crate::store::social::register_agent(
        &state.store,
        &account_id,
        name,
        req.description.trim(),
        &req.runtime_agent,
        req.model.trim(),
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(Json(agent_row_to_summary(&row)))
}

async fn list_agents(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ListAgentsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let rows = crate::store::social::list_agents_for_owner(&state.store, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let agents = rows.iter().map(agent_row_to_summary).collect();
    Ok(Json(ListAgentsResponse { agents }))
}

async fn delete_agent_handler(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let deleted = crate::store::social::delete_agent(&state.store, &agent_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    if !deleted {
        return Err(err("not_found", "agent not found or not owned by you"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn add_group_member(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<AddGroupMemberRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    // Verify caller is a member
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    // Verify conversation is a group
    let conversation = crate::store::social::get_conversation(&state.store, &conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    if conversation.kind != "group" {
        return Err(err(
            "bad_request",
            "can only add members to group conversations",
        ));
    }
    // Verify the new member is a friend of the caller
    if !crate::store::social::are_friends(&state.store, &account_id, &req.member_account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("conflict", "new member must be your friend"));
    }
    crate::store::social::add_member_to_group(
        &state.store,
        &conversation_id,
        &req.member_account_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_conversation_agents_handler(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationAgentMembersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    let rows = crate::store::social::list_conversation_agents(&state.store, &conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?;
    let agents = rows.iter().map(agent_row_to_summary).collect();
    Ok(Json(ConversationAgentMembersResponse { agents }))
}

async fn add_agent_to_group(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<AddAgentToGroupRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    // Verify caller is a member
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    // Verify conversation is a group
    let conversation = crate::store::social::get_conversation(&state.store, &conversation_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    if conversation.kind != "group" {
        return Err(err(
            "bad_request",
            "can only add agents to group conversations",
        ));
    }
    // Verify the agent exists
    let _agent = crate::store::social::get_agent(&state.store, &req.agent_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "agent not found"))?;
    crate::store::social::add_agent_to_conversation(
        &state.store,
        &conversation_id,
        &req.agent_id,
        &account_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_agent_from_group(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<RemoveAgentFromGroupRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    // Verify caller is a member
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    let removed = crate::store::social::remove_agent_from_conversation(
        &state.store,
        &conversation_id,
        &req.agent_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    if !removed {
        return Err(err("not_found", "agent not in this conversation"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn send_agent_message(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<SendAgentMessageRequest>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = require_account_id(&state, &headers).await?;
    let trimmed = req.text.trim().to_string();
    if trimmed.is_empty() {
        return Err(err("bad_request", "message text is required"));
    }
    // Verify caller is a member
    if !crate::store::social::is_conversation_member(&state.store, &conversation_id, &account_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err("not_found", "conversation not found"));
    }
    // Verify the agent exists and is owned by the caller
    let agent = crate::store::social::get_agent(&state.store, &req.agent_id)
        .await
        .map_err(|e| err("internal", e.to_string()))?
        .ok_or_else(|| err("not_found", "agent not found"))?;
    if agent.owner_account_id != account_id {
        return Err(err("forbidden", "you do not own this agent"));
    }
    // Verify the agent is in this conversation
    if !crate::store::social::is_agent_in_conversation(
        &state.store,
        &conversation_id,
        &req.agent_id,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?
    {
        return Err(err(
            "bad_request",
            "agent is not a member of this conversation",
        ));
    }
    // Extract mentions from the message
    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let mentioned_account_ids = extract_mentioned_account_ids(&trimmed, &req.agent_id, &members);
    let row = crate::store::social::insert_agent_message(
        &state.store,
        &conversation_id,
        &req.agent_id,
        &trimmed,
        chrono::Utc::now().timestamp_millis(),
        req.reply_to_message_id.as_deref(),
        &mentioned_account_ids,
    )
    .await
    .map_err(|e| err("internal", e.to_string()))?;
    // Hydrate the agent message with agent info as sender
    let message = ChatMessageSummary {
        message_id: row.message_id,
        conversation_id: row.conversation_id,
        sender: UserSummary {
            account_id: agent.agent_id.clone(),
            minos_id: agent.agent_id.clone(),
            display_name: format!("🤖 {}", agent.name),
        },
        text: row.text,
        created_at_ms: row.created_at_ms,
        reply_to: None,
        recalled_at_ms: row.recalled_at_ms,
        mentioned_account_ids,
        sender_type: SenderType::Agent,
    };
    fan_out_social_message(&state, &message).await;
    Ok(Json(message))
}

fn agent_row_to_summary(row: &crate::store::social::AgentRow) -> AgentSummary {
    AgentSummary {
        agent_id: row.agent_id.clone(),
        owner_account_id: row.owner_account_id.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        runtime_agent: row.runtime_agent.clone(),
        model: row.model.clone(),
        created_at_ms: row.created_at_ms,
        updated_at_ms: row.updated_at_ms,
    }
}
