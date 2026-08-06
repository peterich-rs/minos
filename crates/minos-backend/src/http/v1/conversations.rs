use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, post};
use axum::{Json, Router};
use minos_protocol::{
    ChatMessageSummary, ConversationMembersResponse, ConversationReadResponse,
    ConversationResponse, ConversationsResponse, CreateGroupConversationRequest,
    EnsureDirectConversationRequest, ListChatMessagesRequest, ListChatMessagesResponse,
    SendChatMessageRequest, ToggleReactionRequest, ToggleReactionResponse,
    UpsertConversationRequest,
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
        .route("/conversations/upsert", post(upsert_conversation))
        .route(
            "/conversations/:conversation_id",
            delete(delete_conversation),
        )
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
            "/conversations/:conversation_id/messages/:message_id/reactions/toggle",
            post(toggle_reaction),
        )
        .route(
            "/conversations/:conversation_id/members/add",
            post(add_group_member),
        )
        .route(
            "/conversations/:conversation_id/members/remove",
            post(remove_group_member),
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

async fn upsert_conversation(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<UpsertConversationRequest>,
) -> Result<Json<ConversationResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let response = conversations_svc
        .upsert_conversation(
            &account_id,
            &req.conversation_id,
            &req.title,
            &req.member_account_ids,
            &req.agent_ids,
        )
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(response))
}

async fn delete_conversation(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    if !conversations_svc
        .is_member(&conversation_id, &account_id)
        .await
        .map_err(map_conversation_error)?
    {
        return Err(err("not_found", "conversation not found"));
    }

    stop_active_sessions_for_deleted_conversation(&state, &conversation_id, &account_id).await;

    let deleted = conversations_svc
        .delete_conversation(&account_id, &conversation_id)
        .await
        .map_err(map_conversation_error)?;
    if !deleted {
        return Err(err("not_found", "conversation not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_active_sessions_for_deleted_conversation(
    state: &BackendState,
    conversation_id: &str,
    account_id: &str,
) {
    let sessions = match crate::store::agent_sessions::list_for_account_conversation(
        &state.store,
        conversation_id,
        account_id,
        None,
        200,
    )
    .await
    {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!(
                target: "minos_backend::conversations",
                conversation_id = %conversation_id,
                error = %error,
                "failed to list agent sessions before deleting conversation"
            );
            return;
        }
    };

    for session in sessions {
        if !matches!(session.status.as_str(), "pending" | "running") {
            continue;
        }
        if let Err(error) = state
            .agent_sessions
            .stop(crate::agent_sessions::StopAgentSessionInput {
                session_id: session.session_id.clone(),
                caller_account_id: account_id.to_string(),
            })
            .await
        {
            tracing::warn!(
                target: "minos_backend::conversations",
                conversation_id = %conversation_id,
                session_id = %session.session_id,
                error = %error,
                "failed to stop active agent session before deleting conversation"
            );
        }
    }
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
    let latest = conversations_svc
        .mark_read(&account_id, &conversation_id)
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(ConversationReadResponse {
        last_read_seq: latest.map(|(seq, _)| seq),
        last_read_at_ms: latest.map(|(_, at)| at),
    }))
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
            query.before_seq,
            query.after_seq,
            query.limit.unwrap_or(50),
        )
        .await
        .map_err(map_conversation_error)?;
    Ok(Json(ListChatMessagesResponse {
        messages: result.messages,
        next_before_seq: result.next_before_seq,
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
        req.client_message_id,
        req.message_source.unwrap_or_default(),
        req.client_sent_at_ms.or(req.created_at_ms),
        req.attachment_blob_ids,
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
        req.client_message_id,
        req.message_source.unwrap_or_default(),
        req.client_sent_at_ms.or(req.created_at_ms),
        req.attachment_blob_ids,
    )
    .await
}

async fn send_message_inner(
    state: BackendState,
    headers: HeaderMap,
    conversation_id: String,
    text: String,
    reply_to_message_id: Option<String>,
    client_message_id: Option<String>,
    message_source: minos_protocol::MessageSource,
    client_sent_at_ms: Option<i64>,
    attachment_blob_ids: Vec<String>,
) -> Result<Json<ChatMessageSummary>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() && attachment_blob_ids.is_empty() {
        return Err(err(
            "bad_request",
            "message text or attachment_blob_ids is required",
        ));
    }

    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let (message, _members) = conversations_svc
        .send_message(
            &account_id,
            &conversation_id,
            &trimmed,
            reply_to_message_id.as_deref(),
            client_message_id.as_deref(),
            message_source,
            client_sent_at_ms,
            &attachment_blob_ids,
        )
        .await
        .map_err(map_conversation_error)?;

    super::social::fan_out_social_message(&state, &message).await;

    // Dispatch only for live client sends (Mobile / multi-end). Desktop uses
    // host_projection after native local execution so Hub never double-starts.
    // Failures are user-visible (timeline bubble + StreamEvent agent_error).
    if message_source.allows_agent_dispatch() {
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
                message_id = %message.message_id,
                "agent dispatch pipeline error after message send"
            );
        }
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
    super::social::fan_out_social_message(&state, &message).await;
    Ok(Json(message))
}

async fn toggle_reaction(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path((conversation_id, message_id)): Path<(String, String)>,
    Json(req): Json<ToggleReactionRequest>,
) -> Result<Json<ToggleReactionResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    let conversations_svc = DefaultConversationService::new(state.store.clone());
    let (action, reactions, pending) = conversations_svc
        .toggle_reaction(
            &account_id,
            &conversation_id,
            &message_id,
            &req.emoji,
            &req.client_op_id,
        )
        .await
        .map_err(map_conversation_error)?;

    state.wake_outbox();
    let now_ms = chrono::Utc::now().timestamp_millis();
    if let Err(error) = state
        .realtime
        .publish_durable_event_by_id(&pending.topic_kind, &pending.event_id)
        .await
    {
        tracing::warn!(
            target: "minos_backend::conversations",
            error = %error,
            event_id = %pending.event_id,
            "failed to publish reaction durable; outbox will retry"
        );
    } else if let Some(outbox_id) = pending.outbox_id.as_deref() {
        let _ = crate::store::outbox_events::ack(&state.store, outbox_id, now_ms).await;
    }

    Ok(Json(ToggleReactionResponse {
        message_id,
        conversation_id,
        reactions,
        action,
    }))
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

async fn remove_group_member(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(req): Json<minos_protocol::RemoveGroupMemberRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    let account_id = super::social::require_account_id_from_state(&state, &headers)?;
    if req.member_account_id == account_id {
        return Err(err("bad_request", "leaving a group is not supported yet"));
    }

    let conversations_svc = DefaultConversationService::new(state.store.clone());
    if !conversations_svc
        .is_member(&conversation_id, &account_id)
        .await
        .map_err(map_conversation_error)?
    {
        return Err(err("not_found", "conversation not found"));
    }

    let conversation = conversations_svc
        .get_conversation(&conversation_id)
        .await
        .map_err(map_conversation_error)?
        .ok_or_else(|| err("not_found", "conversation not found"))?;
    if conversation.kind != "group" {
        return Err(err(
            "bad_request",
            "can only remove members from group conversations",
        ));
    }

    let removed = conversations_svc
        .remove_member(&conversation_id, &req.member_account_id)
        .await
        .map_err(map_conversation_error)?;
    if !removed {
        return Err(err("not_found", "member not in this conversation"));
    }
    Ok(StatusCode::NO_CONTENT)
}
