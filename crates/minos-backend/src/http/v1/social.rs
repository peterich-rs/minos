use std::collections::{BTreeSet, HashMap};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use minos_protocol::{
    ChatMessageReplySummary, ChatMessageSummary, ConversationKind,
    ConversationMembersResponse, ConversationReadResponse, ConversationResponse,
    ConversationSummary, ConversationsResponse, CreateFriendRequestRequest,
    CreateGroupConversationRequest, EnsureDirectConversationRequest, Envelope, EventKind,
    FriendRequestStatus, FriendRequestSummary, FriendRequestsResponse, FriendSummary,
    FriendsResponse, ListChatMessagesResponse, MyProfileResponse, SearchUsersResponse,
    SendChatMessageRequest, SetMinosIdRequest, UserSummary,
};
use serde::{Deserialize, Serialize};

use crate::auth::bearer;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/me/profile", get(get_my_profile))
        .route("/me/profile/minos-id", post(set_minos_id))
        .route("/users/search", get(search_users))
        .route("/friends", get(list_friends))
        .route(
            "/friend-requests",
            get(list_friend_requests).post(create_friend_request),
        )
        .route(
            "/friend-requests/:request_id/accept",
            post(accept_friend_request),
        )
        .route(
            "/friend-requests/:request_id/reject",
            post(reject_friend_request),
        )
        .route("/conversations", get(list_conversations))
        .route("/conversations/direct", post(ensure_direct_conversation))
        .route("/conversations/group", post(create_group_conversation))
        .route(
            "/conversations/:conversation_id/members",
            get(list_conversation_members),
        )
        .route(
            "/conversations/:conversation_id/read",
            post(mark_conversation_read),
        )
        .route(
            "/conversations/:conversation_id/messages",
            get(list_messages).post(send_message),
        )
        .route(
            "/conversations/:conversation_id/messages/:message_id/recall",
            post(recall_message),
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

#[derive(Debug, Deserialize)]
struct SearchUsersQuery {
    minos_id: String,
}

async fn search_users(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Query(query): Query<SearchUsersQuery>,
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
    let mut friends = Vec::with_capacity(friendships.len());
    for friendship in friendships {
        let other_id = if friendship.account_low_id == account_id {
            friendship.account_high_id
        } else {
            friendship.account_low_id
        };
        let profile = load_profile(&state, &other_id).await?;
        let friend_display_name = display_name(&profile);
        friends.push(FriendSummary {
            account_id: profile.account_id,
            minos_id: profile.minos_id,
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

#[derive(Debug, Deserialize)]
struct ListMessagesQuery {
    before_ts_ms: Option<i64>,
    limit: Option<u32>,
}

async fn list_messages(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Query(query): Query<ListMessagesQuery>,
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
    let reply_to_message_id = match req.reply_to_message_id.as_deref() {
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
            Some(reply_target.message_id)
        }
        None => None,
    };
    let members =
        crate::store::social::list_conversation_member_profiles(&state.store, &conversation_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    let mentioned_account_ids =
        extract_mentioned_account_ids(&trimmed, &account_id, &members);
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
    fan_out_social_message(&state, &message).await;
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
        return Err(err("bad_request", "only the sender can recall this message"));
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

async fn hydrate_friend_requests(
    state: &BackendState,
    rows: Vec<crate::store::social::FriendRequestRow>,
) -> Result<Vec<FriendRequestSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let from = load_profile(state, &row.from_account_id).await?;
        let to = load_profile(state, &row.to_account_id).await?;
        output.push(FriendRequestSummary {
            request_id: row.request_id,
            from: to_user_summary(&from),
            to: to_user_summary(&to),
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
            (ConversationKind::Group, _, _) => "未命名群聊".into(),
            _ => "对话".into(),
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
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let mentioned_account_ids = mentions_by_message
            .remove(&row.message_id)
            .unwrap_or_default();
        let reply_to = if let Some(reply_id) = row.reply_to_message_id.as_ref() {
            if let Some(reply_row) = reply_rows.get(reply_id).cloned() {
                let reply_sender = load_profile(state, &reply_row.sender_account_id).await?;
                Some(ChatMessageReplySummary {
                    message_id: reply_row.message_id,
                    sender: to_user_summary(&reply_sender),
                    text: reply_row.text,
                    recalled_at_ms: reply_row.recalled_at_ms,
                })
            } else {
                None
            }
        } else {
            None
        };
        let sender = load_profile(state, &row.sender_account_id).await?;
        output.push(ChatMessageSummary {
            message_id: row.message_id,
            conversation_id: row.conversation_id,
            sender: to_user_summary(&sender),
            text: row.text,
            created_at_ms: row.created_at_ms,
            reply_to,
            recalled_at_ms: row.recalled_at_ms,
            mentioned_account_ids,
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
        while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
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
