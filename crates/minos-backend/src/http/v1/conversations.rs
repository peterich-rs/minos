use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use minos_protocol::{
    ChatMessageSummary, ConversationMembersResponse, ConversationReadResponse,
    ConversationResponse, ConversationsResponse, CreateGroupConversationRequest,
    EnsureDirectConversationRequest, Envelope, EventKind, ListChatMessagesRequest,
    ListChatMessagesResponse, SendChatMessageRequest,
};

use crate::conversations::{ConversationError, ConversationService, DefaultConversationService};
use crate::http::error_response::{err_response, ErrorEnvelope};
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/conversations/query", post(list_conversations))
        .route("/conversations/list", post(list_conversations))
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
        .route("/conversations/send-message", post(send_message_command))
        .route(
            "/conversations/:conversation_id/messages/query",
            post(list_messages_query),
        )
        .route(
            "/conversations/:conversation_id/messages/:message_id/recall",
            post(recall_message),
        )
        .route(
            "/conversations/:conversation_id/members/add",
            post(add_group_member),
        )
}

pub fn external_sql_router() -> Router<BackendState> {
    router()
}

fn err(code: &'static str, message: impl Into<String>) -> (StatusCode, Json<ErrorEnvelope>) {
    err_response(code, message)
}

fn map_conversation_error(e: ConversationError) -> (StatusCode, Json<ErrorEnvelope>) {
    match e {
        ConversationError::NotFound => err("not_found", "conversation not found"),
        ConversationError::Forbidden => err("not_found", "conversation not found"),
        ConversationError::NotFriends => err("conflict", "users are not friends"),
        ConversationError::TitleRequired => err("bad_request", "group title is required"),
        ConversationError::InvalidKind(msg) => err("internal", msg),
        ConversationError::MissingProfile(id) => {
            err("internal", format!("profile not found: {id}"))
        }
        ConversationError::ValidationFormat(msg) => err("bad_request", msg),
        ConversationError::Internal(e) => err("internal", e.to_string()),
    }
}

async fn list_conversations(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<Json<ConversationsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let conversations = conversations_svc
        .list_conversations(&account_id)
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(ConversationsResponse { conversations }))
}

async fn ensure_direct_conversation(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<EnsureDirectConversationRequest>,
) -> Result<Json<ConversationResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let response = conversations_svc
        .ensure_direct(&account_id, &req.friend_account_id)
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(response))
}

async fn create_group_conversation(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<CreateGroupConversationRequest>,
) -> Result<Json<ConversationResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let response = conversations_svc
        .create_group(&account_id, &req.title, &req.member_account_ids)
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(response))
}

async fn list_conversation_members(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationMembersResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let members = conversations_svc
        .list_members(&account_id, &conversation_id)
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(ConversationMembersResponse { members }))
}

async fn mark_conversation_read(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationReadResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let last_read_at_ms = conversations_svc
        .mark_read(&account_id, &conversation_id)
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(ConversationReadResponse { last_read_at_ms }))
}

async fn list_messages_query(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(query): Json<ListChatMessagesRequest>,
) -> Result<Json<ListChatMessagesResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let result = conversations_svc
        .list_messages(
            &account_id,
            &conversation_id,
            query.before_ts_ms,
            query.limit.unwrap_or(50),
        )
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(ListChatMessagesResponse {
        messages: result.messages,
        next_before_ts_ms: result.next_before_ts_ms,
    }))
}

async fn send_message(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<SendChatMessageRequest>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    send_message_inner(
        state,
        headers,
        conversation_id,
        req.text,
        req.reply_to_message_id,
    )
    .await
}

async fn send_message_command(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<crate::http::v1::social::SendConversationMessageRequest>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    send_message_inner(
        state,
        headers,
        req.conversation_id,
        req.text,
        req.reply_to_message_id,
    )
    .await
}

async fn send_message_inner(
    state: BackendState,
    headers: HeaderMap,
    conversation_id: String,
    text: String,
    reply_to_message_id: Option<String>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err(err("bad_request", "message text is required"));
    }

    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let (message, _members) = conversations_svc
        .send_message(
            &account_id,
            &conversation_id,
            &trimmed,
            reply_to_message_id.as_deref(),
        )
        .await
        .map_err(map_conversation_error)?;

    fan_out_social_message(&state, &message).await;

    // Agent dispatch logic (delegated to the social handler)
    if let Err(e) = super::social::try_agent_dispatch(
        &state,
        &account_id,
        &conversation_id,
        &message,
        reply_to_message_id.as_deref(),
        &trimmed,
    )
    .await
    {
        tracing::warn!(
            target: "minos_backend::conversations",
            error = %e,
            conversation_id = %conversation_id,
            "agent dispatch failed after message send"
        );
    }

    Ok(Json(message))
}

async fn recall_message(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let message = conversations_svc
        .recall_message(&account_id, &conversation_id, &message_id)
        .await
        .map_err(map_conversation_error)?;
    fan_out_social_message(&state, &message).await;
    Ok(Json(message))
}

async fn add_group_member(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<minos_protocol::AddGroupMemberRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());

    // Verify caller is a member
    if !conversations_svc
        .is_member(&conversation_id, &account_id)
        .await
        .map_err(map_conversation_error)?
    {
        return Err(err("not_found", "conversation not found"));
    }
    // Verify conversation is a group
    let conversation = conversations_svc
        .get_conversation(&conversation_id)
        .await
        .map_err(map_conversation_error)?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    if conversation.kind != "group" {
        return Err(err(
            "bad_request",
            "can only add members to group conversations",
        ));
    }
    // Verify the new member is a friend of the caller
    let friendships =
        crate::store::social::are_friends(&state.store, &account_id, &req.member_account_id)
            .await
            .map_err(|e| err("internal", e.to_string()))?;
    if !friendships {
        return Err(err("conflict", "new member must be your friend"));
    }
    conversations_svc
        .add_member(&conversation_id, &req.member_account_id)
        .await
        .map_err(map_conversation_error)?;
    Ok(StatusCode::NO_CONTENT)
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
                target: "minos_backend::conversations",
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
    state.realtime.fanout_social_message(&members, &frame).await;
}
