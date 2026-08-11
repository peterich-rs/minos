use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use minos_protocol::{
    ChatMessageReplySummary, ChatMessageSummary, ConversationKind, ConversationResponse,
    ConversationSummary, MessageSender, UserSummary,
};

use crate::app::tx::Storage;
use crate::error::BackendError;
use crate::profiles::use_case::to_user_summary;
use crate::store::{social, StoreHandle};

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("conversation_not_found")]
    NotFound,
    #[error("conversation_forbidden")]
    Forbidden,
    #[error("users_not_friends")]
    NotFriends,
    #[error("group_title_required")]
    TitleRequired,
    #[error("invalid_conversation_kind: {0}")]
    InvalidKind(String),
    #[error("missing_profile: {0}")]
    MissingProfile(String),
    #[error("validation_format: {0}")]
    ValidationFormat(String),
    /// Same `client_message_id` with a different request fingerprint.
    #[error("idempotency_conflict: {0}")]
    IdempotencyConflict(String),
    #[error(transparent)]
    Internal(#[from] BackendError),
}

fn map_store_write(error: BackendError) -> ConversationError {
    match &error {
        BackendError::StoreQuery { operation, message }
            if operation.ends_with("idempotency_conflict") =>
        {
            ConversationError::IdempotencyConflict(message.clone())
        }
        _ => ConversationError::Internal(error),
    }
}

#[derive(Debug, Clone)]
pub struct ListMessagesResult {
    pub messages: Vec<ChatMessageSummary>,
    pub next_before_seq: Option<i64>,
}

#[async_trait]
pub trait ConversationService: Send + Sync {
    async fn list_conversations(
        &self,
        account_id: &str,
    ) -> Result<Vec<ConversationSummary>, ConversationError>;

    async fn ensure_direct(
        &self,
        account_id: &str,
        friend_account_id: &str,
    ) -> Result<ConversationResponse, ConversationError>;

    async fn create_group(
        &self,
        account_id: &str,
        title: &str,
        member_account_ids: &[String],
    ) -> Result<ConversationResponse, ConversationError>;

    /// Upsert a work conversation with a client-owned id (Desktop → Hub IM).
    async fn upsert_conversation(
        &self,
        account_id: &str,
        conversation_id: &str,
        title: &str,
        member_account_ids: &[String],
        agent_ids: &[String],
    ) -> Result<ConversationResponse, ConversationError>;

    async fn list_members(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<UserSummary>, ConversationError>;

    /// Returns `(last_read_seq, last_read_at_ms)` when messages exist.
    async fn mark_read(
        &self,
        account_id: &str,
        conversation_id: &str,
        read_up_to_message_seq: i64,
    ) -> Result<Option<(i64, i64)>, ConversationError>;

    async fn list_messages(
        &self,
        account_id: &str,
        conversation_id: &str,
        before_seq: Option<i64>,
        after_seq: Option<i64>,
        limit: u32,
    ) -> Result<ListMessagesResult, ConversationError>;

    /// Returns `(message, members, bot_deliveries_enqueued)`.
    /// Bot deliveries are co-committed with the message when planned.
    async fn send_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        text: &str,
        reply_to_message_id: Option<&str>,
        client_message_id: Option<&str>,
        message_source: minos_protocol::MessageSource,
        client_sent_at_ms: Option<i64>,
        attachment_blob_ids: &[String],
        // Optional structured mentions from Account WS AppendMessage.
        // Validated against conversation participants; body never invents targets.
        structured_mentions: &[minos_protocol::MentionTarget],
    ) -> Result<(ChatMessageSummary, Vec<social::ProfileRow>, bool), ConversationError>;

    async fn recall_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<ChatMessageSummary, ConversationError>;

    /// Toggle cloud reaction; returns action, aggregates, and outbox publish handle.
    ///
    /// `client_op_id` is required: becomes the durable event_id suffix and
    /// must match the client Intent Outbox entry id.
    async fn toggle_reaction(
        &self,
        account_id: &str,
        conversation_id: &str,
        message_id: &str,
        emoji: &str,
        client_op_id: &str,
    ) -> Result<
        (
            String,
            Vec<minos_protocol::ReactionGroup>,
            social::PendingDurablePublish,
        ),
        ConversationError,
    >;

    async fn get_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<social::ConversationRow>, ConversationError>;

    async fn is_member(
        &self,
        conversation_id: &str,
        account_id: &str,
    ) -> Result<bool, ConversationError>;

    async fn get_message(
        &self,
        message_id: &str,
    ) -> Result<Option<social::ChatMessageRow>, ConversationError>;

    async fn list_member_profiles(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<social::ProfileRow>, ConversationError>;

    async fn list_member_ids(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<String>, ConversationError>;

    /// Add member. Caller must be owner/admin. Returns durable publishes + change metadata.
    async fn add_member(
        &self,
        actor_account_id: &str,
        conversation_id: &str,
        member_account_id: &str,
    ) -> Result<
        (
            social::MembershipChangeResult,
            Vec<social::PendingDurablePublish>,
        ),
        ConversationError,
    >;

    /// Remove member or self-leave. Moderator remove requires owner/admin.
    /// Returns whether membership row changed plus durable publishes.
    async fn remove_member(
        &self,
        actor_account_id: &str,
        conversation_id: &str,
        member_account_id: &str,
    ) -> Result<
        (
            social::MembershipChangeResult,
            Vec<social::PendingDurablePublish>,
        ),
        ConversationError,
    >;

    async fn delete_conversation(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> Result<bool, ConversationError>;
}

pub struct DefaultConversationService {
    store: StoreHandle,
}

impl DefaultConversationService {
    #[must_use]
    pub fn new(store: StoreHandle) -> Arc<Self> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl ConversationService for DefaultConversationService {
    async fn list_conversations(
        &self,
        account_id: &str,
    ) -> Result<Vec<ConversationSummary>, ConversationError> {
        let rows = social::list_conversations_for(&self.store, account_id).await?;
        hydrate_conversations(&self.store, account_id, rows).await
    }

    async fn ensure_direct(
        &self,
        account_id: &str,
        friend_account_id: &str,
    ) -> Result<ConversationResponse, ConversationError> {
        if !social::are_friends(&self.store, account_id, friend_account_id).await? {
            return Err(ConversationError::NotFriends);
        }
        let conversation = social::ensure_direct_conversation(
            &self.store,
            account_id,
            account_id,
            friend_account_id,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(ConversationResponse {
            conversation_id: conversation.conversation_id,
        })
    }

    async fn create_group(
        &self,
        account_id: &str,
        title: &str,
        member_account_ids: &[String],
    ) -> Result<ConversationResponse, ConversationError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(ConversationError::TitleRequired);
        }
        for member in member_account_ids {
            if member == account_id {
                continue;
            }
            if !social::are_friends(&self.store, account_id, member).await? {
                return Err(ConversationError::NotFriends);
            }
        }
        let conversation = social::create_group_conversation(
            &self.store,
            account_id,
            title,
            member_account_ids,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(ConversationResponse {
            conversation_id: conversation.conversation_id,
        })
    }

    async fn upsert_conversation(
        &self,
        account_id: &str,
        conversation_id: &str,
        title: &str,
        member_account_ids: &[String],
        agent_ids: &[String],
    ) -> Result<ConversationResponse, ConversationError> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Err(ConversationError::ValidationFormat(
                "conversation_id is required".into(),
            ));
        }
        let title = title.trim();
        if title.is_empty() {
            return Err(ConversationError::TitleRequired);
        }

        // Work conversations: members must be friends (or self). Peers may be empty.
        for member in member_account_ids {
            if member == account_id {
                continue;
            }
            if !social::are_friends(&self.store, account_id, member).await? {
                return Err(ConversationError::NotFriends);
            }
        }

        // If conversation already exists, caller must already be a member (or become one
        // via this upsert as creator path on first create only).
        if let Some(existing) = social::get_conversation(&self.store, conversation_id).await? {
            if existing.kind != "group" {
                return Err(ConversationError::InvalidKind(existing.kind));
            }
            if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
                return Err(ConversationError::Forbidden);
            }
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let conversation = social::upsert_group_conversation(
            &self.store,
            conversation_id,
            account_id,
            title,
            member_account_ids,
            now_ms,
        )
        .await
        .map_err(|error| match error {
            BackendError::StoreQuery { message, .. }
                if message.contains("group title is required") =>
            {
                ConversationError::TitleRequired
            }
            other => ConversationError::Internal(other),
        })?;

        for agent_id in agent_ids {
            let agent_id = agent_id.trim();
            if agent_id.is_empty() {
                continue;
            }
            // Ignore unknown agents; attach only registered cloud agents.
            if social::get_agent(&self.store, agent_id).await?.is_none() {
                continue;
            }
            social::add_agent_to_conversation(
                &self.store,
                conversation_id,
                agent_id,
                account_id,
                now_ms,
            )
            .await?;
        }

        Ok(ConversationResponse {
            conversation_id: conversation.conversation_id,
        })
    }

    async fn list_members(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<UserSummary>, ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let profiles =
            social::list_conversation_member_profiles(&self.store, conversation_id).await?;
        Ok(profiles.iter().map(to_user_summary).collect())
    }

    async fn mark_read(
        &self,
        account_id: &str,
        conversation_id: &str,
        read_up_to_message_seq: i64,
    ) -> Result<Option<(i64, i64)>, ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        if read_up_to_message_seq <= 0 {
            return Err(ConversationError::ValidationFormat(
                "read_up_to_message_seq must be positive".into(),
            ));
        }
        let latest = social::mark_conversation_read_to_seq(
            &self.store,
            conversation_id,
            account_id,
            read_up_to_message_seq,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(latest)
    }

    async fn list_messages(
        &self,
        account_id: &str,
        conversation_id: &str,
        before_seq: Option<i64>,
        after_seq: Option<i64>,
        limit: u32,
    ) -> Result<ListMessagesResult, ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let limit = limit.min(200);
        let deleted_at_ms =
            social::conversation_deleted_at_for_account(&self.store, conversation_id, account_id)
                .await?;
        let mut messages =
            social::list_messages(&self.store, conversation_id, before_seq, after_seq, limit)
                .await?;
        if let Some(deleted_at_ms) = deleted_at_ms {
            messages.retain(|message| message.created_at_ms > deleted_at_ms);
        }
        let next_before_seq = if messages.len() as u32 == limit {
            messages.last().map(|m| m.message_seq)
        } else {
            None
        };
        // list_messages returns DESC; reverse to chronological ASC for clients.
        messages.reverse();
        let hydrated = hydrate_messages_for_viewer(&self.store, messages, Some(account_id)).await?;
        Ok(ListMessagesResult {
            messages: hydrated,
            next_before_seq,
        })
    }

    async fn send_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        text: &str,
        reply_to_message_id: Option<&str>,
        client_message_id: Option<&str>,
        message_source: minos_protocol::MessageSource,
        client_sent_at_ms: Option<i64>,
        attachment_blob_ids: &[String],
        structured_mentions: &[minos_protocol::MentionTarget],
    ) -> Result<(ChatMessageSummary, Vec<social::ProfileRow>, bool), ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let attachment_rows = crate::store::message_attachments::load_ready_owned_blobs(
            &self.store,
            account_id,
            attachment_blob_ids,
        )
        .await
        .map_err(|e| ConversationError::ValidationFormat(e.to_string()))?;
        let attachments_wire: Vec<minos_protocol::ChatMessageAttachment> = attachment_rows
            .iter()
            .map(|b| minos_protocol::ChatMessageAttachment {
                blob_id: b.blob_id.clone(),
                content_type: b.content_type.clone(),
                byte_size: b.byte_size,
                kind: b.kind.clone(),
                original_filename: b.original_filename.clone(),
            })
            .collect();
        let attachment_ids: Vec<String> =
            attachment_rows.iter().map(|b| b.blob_id.clone()).collect();
        // reply_to: client_live hard-fails; host_projection / system soft-drop.
        let reply_to_id = match reply_to_message_id {
            Some(message_id) => match social::get_message(&self.store, message_id).await? {
                Some(reply_target) if reply_target.conversation_id == conversation_id => {
                    Some(reply_target.message_id)
                }
                _ => match message_source {
                    minos_protocol::MessageSource::ClientLive => {
                        return Err(ConversationError::ValidationFormat(
                            "reply_to_message_id not found in this conversation".into(),
                        ));
                    }
                    minos_protocol::MessageSource::HostProjection
                    | minos_protocol::MessageSource::System => {
                        tracing::warn!(
                            target: "minos_backend::conversations",
                            conversation_id = %conversation_id,
                            reply_to_message_id = %message_id,
                            ?message_source,
                            "dropping invalid reply_to for non-client message source"
                        );
                        None
                    }
                },
            },
            None => None,
        };
        let members =
            social::list_conversation_member_profiles(&self.store, conversation_id).await?;
        let member_ids = members
            .iter()
            .map(|m| m.account_id.clone())
            .collect::<Vec<_>>();
        // Active bots are delivery targets; full roster detects disabled structured mentions.
        // Body text never invents delivery targets — clients send structured mentions.
        let agents = social::list_conversation_agents_active(&self.store, conversation_id).await?;
        let all_agents = social::list_conversation_agents(&self.store, conversation_id).await?;
        let mentions = validate_structured_mentions(
            structured_mentions,
            account_id,
            &members,
            &agents,
            &all_agents,
        )
        .map_err(ConversationError::ValidationFormat)?;
        // Hub server clock is the sole ordering authority for created_at_ms.
        // client_sent_at_ms is accepted for future display/debug only.
        let _ = client_sent_at_ms;
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Build reply summary before the write tx so durable payload is complete
        // without reading uncommitted rows from a second connection.
        let reply_to = match reply_to_id.as_deref() {
            Some(id) => match social::get_message(&self.store, id).await? {
                Some(reply_row) => {
                    let profiles = members
                        .iter()
                        .map(|p| (p.account_id.clone(), p.clone()))
                        .collect::<HashMap<_, _>>();
                    let mut agent_map = HashMap::new();
                    if reply_row.sender_type == "agent" {
                        let agent_id = agent_id_for_row(&reply_row);
                        if agent_id != "unknown-bot" {
                            if let Some(agent) = social::get_agent(&self.store, &agent_id).await? {
                                agent_map.insert(agent_id, agent);
                            }
                        }
                    }
                    Some(ChatMessageReplySummary {
                        message_id: reply_row.message_id.clone(),
                        sender: sender_summary(&reply_row, &profiles, &agent_map)?,
                        text: reply_row.text.clone(),
                        recalled_at_ms: reply_row.recalled_at_ms,
                    })
                }
                None => None,
            },
            None => None,
        };

        let sender_profile = members
            .iter()
            .find(|m| m.account_id == account_id)
            .ok_or_else(|| ConversationError::MissingProfile(account_id.to_string()))?;
        let sender = MessageSender::from_user_summary(to_user_summary(sender_profile));
        let sender_minos_id = Some(sender_profile.minos_id.clone());

        // Plan bot deliveries before the write TX (no store reads while holding SQLite tx).
        // message_id is filled after insert via build_dispatch_rows.
        let dispatch_plans = if message_source.allows_agent_dispatch() {
            let reply_target = match reply_to_id.as_deref() {
                Some(id) => social::get_message(&self.store, id).await?,
                None => None,
            };
            let plan_message = ChatMessageSummary {
                message_id: String::new(),
                conversation_id: conversation_id.to_string(),
                sender: sender.clone(),
                text: text.to_string(),
                created_at_ms: now_ms,
                message_seq: 0,
                reply_to: reply_to.clone(),
                recalled_at_ms: None,
                mentioned_account_ids: mentions.account_ids.clone(),
                mentioned_agent_ids: mentions.agent_ids.clone(),
                sender_type: ChatMessageSummary::sender_type_from(&sender),
                reactions: vec![],
                attachments: attachments_wire.clone(),
            };
            crate::agent_inbox::plan_agent_deliveries(
                &self.store,
                conversation_id,
                &plan_message,
                text,
                reply_target.as_ref(),
            )
            .await?
        } else {
            Vec::new()
        };

        // Transactional Outbox: chat_messages + durable + social outbox + bot
        // deliveries in one commit when agent dispatch is allowed.
        let mut tx = self.store.begin().await?;
        let outcome = social::insert_message_with_id_in_tx(
            &mut tx,
            conversation_id,
            account_id,
            text,
            now_ms,
            reply_to_id.as_deref(),
            &mentions,
            client_message_id,
            &attachment_ids,
            message_source.as_str(),
        )
        .await
        .map_err(map_store_write)?;
        let message = if outcome.inserted {
            // Attachments join rows in the same transaction as message + durable/outbox.
            if !attachment_ids.is_empty() {
                crate::store::message_attachments::link_blobs_to_message_in_tx(
                    &mut tx,
                    &outcome.row.message_id,
                    &attachment_ids,
                )
                .await?;
            }
            ChatMessageSummary {
                message_id: outcome.row.message_id.clone(),
                conversation_id: outcome.row.conversation_id.clone(),
                sender: sender.clone(),
                text: outcome.row.text.clone(),
                created_at_ms: outcome.row.created_at_ms,
                message_seq: outcome.row.message_seq,
                reply_to,
                recalled_at_ms: None,
                mentioned_account_ids: mentions.account_ids.clone(),
                mentioned_agent_ids: mentions.agent_ids.clone(),
                sender_type: ChatMessageSummary::sender_type_from(&sender),
                reactions: vec![],
                attachments: attachments_wire.clone(),
            }
        } else {
            // Idempotent hit: re-hydrate so durable repair uses current SSOT text.
            drop(tx);
            let mut hydrated = hydrate_messages(&self.store, vec![outcome.row]).await?;
            let message = hydrated.remove(0);
            let mut tx = self.store.begin().await?;
            social::ensure_social_message_delivery_in_tx(&mut tx, &message, &member_ids).await?;
            // Re-drive bot deliveries if a prior crash left message without inbox rows.
            let rows = crate::agent_inbox::build_dispatch_rows(
                dispatch_plans,
                &message,
                conversation_id,
                account_id,
                sender_minos_id,
                0,
                now_ms,
            );
            let bot_enqueued = crate::agent_inbox::enqueue_plans_in_tx(&mut tx, &rows).await?;
            tx.commit().await?;
            return Ok((message, members, bot_enqueued));
        };
        social::ensure_social_message_delivery_in_tx(&mut tx, &message, &member_ids).await?;
        let rows = crate::agent_inbox::build_dispatch_rows(
            dispatch_plans,
            &message,
            conversation_id,
            account_id,
            sender_minos_id,
            0,
            now_ms,
        );
        let bot_enqueued = crate::agent_inbox::enqueue_plans_in_tx(&mut tx, &rows).await?;
        tx.commit().await?;
        Ok((message, members, bot_enqueued))
    }

    async fn recall_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<ChatMessageSummary, ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let existing = social::get_message(&self.store, message_id)
            .await?
            .ok_or(ConversationError::NotFound)?;
        if existing.conversation_id != conversation_id {
            return Err(ConversationError::NotFound);
        }
        if existing.sender_account_id.as_deref() != Some(account_id) {
            return Err(ConversationError::ValidationFormat(
                "only the sender can recall this message".into(),
            ));
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        if now_ms - existing.created_at_ms > RECALL_WINDOW_MS {
            return Err(ConversationError::ValidationFormat(
                "message recall window has expired (5 minutes)".into(),
            ));
        }
        let member_ids = social::list_conversation_members(&self.store, conversation_id).await?;

        // Hydrate before mutating so sender/reply stay available for durable payload.
        let mut hydrated = hydrate_messages(&self.store, vec![existing]).await?;
        let mut message = hydrated.remove(0);

        let mut tx = self.store.begin().await?;
        social::recall_message_in_tx(&mut tx, conversation_id, message_id, account_id, now_ms)
            .await?;
        message.text = "[message recalled]".to_string();
        message.recalled_at_ms = Some(message.recalled_at_ms.unwrap_or(now_ms));
        message.mentioned_account_ids.clear();
        message.mentioned_agent_ids.clear();
        social::ensure_social_message_delivery_in_tx(&mut tx, &message, &member_ids).await?;
        tx.commit().await?;
        Ok(message)
    }

    async fn toggle_reaction(
        &self,
        account_id: &str,
        conversation_id: &str,
        message_id: &str,
        emoji: &str,
        client_op_id: &str,
    ) -> Result<
        (
            String,
            Vec<minos_protocol::ReactionGroup>,
            social::PendingDurablePublish,
        ),
        ConversationError,
    > {
        let client_op_id = client_op_id.trim();
        if client_op_id.is_empty() {
            return Err(ConversationError::ValidationFormat(
                "client_op_id is required".into(),
            ));
        }
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let existing = social::get_message(&self.store, message_id)
            .await?
            .ok_or(ConversationError::NotFound)?;
        if existing.conversation_id != conversation_id {
            return Err(ConversationError::NotFound);
        }
        if existing.recalled_at_ms.is_some() {
            return Err(ConversationError::ValidationFormat(
                "cannot react to a recalled message".into(),
            ));
        }

        // Same client_op_id retry: return current aggregate without re-toggle.
        if let Some(prior) = social::get_reaction_client_op(&self.store, client_op_id).await? {
            if prior.conversation_id == conversation_id
                && prior.message_id == message_id
                && prior.account_id == account_id
            {
                let rows =
                    social::list_for_messages(&self.store, &[message_id.to_string()]).await?;
                let reactions = social::aggregate_groups(&rows, Some(account_id));
                let event_id = social::reaction_event_id(
                    conversation_id,
                    message_id,
                    prior.emoji.as_str(),
                    &format!("user-{account_id}"),
                    prior.action.as_str(),
                    client_op_id,
                );
                return Ok((
                    prior.action,
                    reactions,
                    social::PendingDurablePublish {
                        topic_kind: "conversation".into(),
                        event_id,
                        outbox_id: None,
                    },
                ));
            }
            return Err(ConversationError::ValidationFormat(
                "client_op_id already used for a different reaction".into(),
            ));
        }

        let profile = social::profiles_by_accounts(&self.store, &[account_id.to_string()])
            .await?
            .remove(account_id)
            .ok_or_else(|| ConversationError::MissingProfile(account_id.to_string()))?;
        let display_name = profile
            .display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| profile.minos_id.clone());
        let now_ms = chrono::Utc::now().timestamp_millis();

        let mut tx = self.store.begin().await?;
        // Claim client_op_id **before** toggle so concurrent retries cannot double-flip.
        let claimed = social::try_claim_reaction_client_op_in_tx(
            &mut tx,
            client_op_id,
            conversation_id,
            message_id,
            emoji.trim(),
            "pending",
            account_id,
            now_ms,
        )
        .await?;
        if !claimed {
            // Lost race or sequential retry raced into the tx: return current aggregate.
            let rows = social::list_for_message_in_tx(&mut tx, message_id).await?;
            let reactions = social::aggregate_groups(&rows, Some(account_id));
            tx.rollback().await.ok();
            if let Some(prior) = social::get_reaction_client_op(&self.store, client_op_id).await? {
                let event_id = social::reaction_event_id(
                    conversation_id,
                    message_id,
                    prior.emoji.as_str(),
                    &format!("user-{account_id}"),
                    prior.action.as_str(),
                    client_op_id,
                );
                return Ok((
                    prior.action,
                    reactions,
                    social::PendingDurablePublish {
                        topic_kind: "conversation".into(),
                        event_id,
                        outbox_id: None,
                    },
                ));
            }
            return Err(ConversationError::ValidationFormat(
                "client_op_id claim lost without prior row".into(),
            ));
        }

        let action = social::toggle_user_in_tx(
            &mut tx,
            conversation_id,
            message_id,
            account_id,
            &display_name,
            emoji,
            now_ms,
        )
        .await?;
        social::set_reaction_client_op_action_in_tx(&mut tx, client_op_id, &action).await?;
        let rows = social::list_for_message_in_tx(&mut tx, message_id).await?;
        // HTTP response: viewer-resolved for the toggling account.
        let reactions = social::aggregate_groups(&rows, Some(account_id));
        // Durable fanout: viewer-neutral (reacted_by_me=false); clients recompute.
        let durable_reactions = social::aggregate_groups(&rows, None);
        let pending = social::ensure_reaction_delivery_in_tx(
            &mut tx,
            conversation_id,
            message_id,
            emoji.trim(),
            &action,
            minos_protocol::SenderRef::User {
                account_id: account_id.to_string(),
            },
            now_ms,
            durable_reactions,
            client_op_id,
        )
        .await?;
        tx.commit().await?;
        Ok((action, reactions, pending))
    }

    async fn get_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<social::ConversationRow>, ConversationError> {
        Ok(social::get_conversation(&self.store, conversation_id).await?)
    }

    async fn is_member(
        &self,
        conversation_id: &str,
        account_id: &str,
    ) -> Result<bool, ConversationError> {
        Ok(social::is_conversation_member(&self.store, conversation_id, account_id).await?)
    }

    async fn get_message(
        &self,
        message_id: &str,
    ) -> Result<Option<social::ChatMessageRow>, ConversationError> {
        Ok(social::get_message(&self.store, message_id).await?)
    }

    async fn list_member_profiles(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<social::ProfileRow>, ConversationError> {
        Ok(social::list_conversation_member_profiles(&self.store, conversation_id).await?)
    }

    async fn list_member_ids(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<String>, ConversationError> {
        Ok(social::list_conversation_members(&self.store, conversation_id).await?)
    }

    async fn add_member(
        &self,
        actor_account_id: &str,
        conversation_id: &str,
        member_account_id: &str,
    ) -> Result<
        (
            social::MembershipChangeResult,
            Vec<social::PendingDurablePublish>,
        ),
        ConversationError,
    > {
        let actor_role =
            social::get_member_role(&self.store, conversation_id, actor_account_id).await?;
        match actor_role.as_deref() {
            Some("owner") | Some("admin") => {}
            Some(_) => return Err(ConversationError::Forbidden),
            None => return Err(ConversationError::NotFound),
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let change =
            social::add_member_to_group(&self.store, conversation_id, member_account_id, now_ms)
                .await?;
        if !change.changed {
            return Ok((change, Vec::new()));
        }
        let remaining = social::list_conversation_members(&self.store, conversation_id).await?;
        let mut tx = self.store.begin().await?;
        let pending = social::ensure_membership_change_delivery_in_tx(
            &mut tx,
            conversation_id,
            member_account_id,
            actor_account_id,
            &change.action,
            change.role.as_deref(),
            change.membership_version,
            now_ms,
            &remaining,
        )
        .await?;
        tx.commit().await?;
        Ok((change, pending))
    }

    async fn remove_member(
        &self,
        actor_account_id: &str,
        conversation_id: &str,
        member_account_id: &str,
    ) -> Result<
        (
            social::MembershipChangeResult,
            Vec<social::PendingDurablePublish>,
        ),
        ConversationError,
    > {
        let self_leave = actor_account_id == member_account_id;
        if !self_leave {
            let actor_role =
                social::get_member_role(&self.store, conversation_id, actor_account_id).await?;
            match actor_role.as_deref() {
                Some("owner") | Some("admin") => {}
                Some(_) => return Err(ConversationError::Forbidden),
                None => return Err(ConversationError::NotFound),
            }
            // Non-owners cannot remove the owner.
            let target_role =
                social::get_member_role(&self.store, conversation_id, member_account_id).await?;
            if target_role.as_deref() == Some("owner") && actor_role.as_deref() != Some("owner") {
                return Err(ConversationError::Forbidden);
            }
            if target_role.is_none() {
                return Ok((
                    social::MembershipChangeResult {
                        membership_version: 0,
                        action: "removed".into(),
                        member_account_id: member_account_id.to_string(),
                        role: None,
                        changed: false,
                    },
                    Vec::new(),
                ));
            }
        } else if !social::is_conversation_member(&self.store, conversation_id, actor_account_id)
            .await?
        {
            return Err(ConversationError::NotFound);
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let change = social::remove_member_from_group(
            &self.store,
            conversation_id,
            member_account_id,
            now_ms,
        )
        .await?;
        if !change.changed {
            return Ok((change, Vec::new()));
        }
        let remaining = social::list_conversation_members(&self.store, conversation_id).await?;
        let mut tx = self.store.begin().await?;
        let pending = social::ensure_membership_change_delivery_in_tx(
            &mut tx,
            conversation_id,
            member_account_id,
            actor_account_id,
            &change.action,
            None,
            change.membership_version,
            now_ms,
            &remaining,
        )
        .await?;
        tx.commit().await?;
        Ok((change, pending))
    }

    async fn delete_conversation(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> Result<bool, ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        Ok(social::mark_conversation_deleted_for_account(
            &self.store,
            conversation_id,
            account_id,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?)
    }
}

/// Maximum time window (in milliseconds) within which a message can be recalled.
const RECALL_WINDOW_MS: i64 = 5 * 60 * 1000;

// ─── Hydration Helpers ────────────────────────────────────────────────

async fn hydrate_conversations(
    store: &StoreHandle,
    account_id: &str,
    rows: Vec<social::ConversationDigestRow>,
) -> Result<Vec<ConversationSummary>, ConversationError> {
    let mut counterpart_ids = rows
        .iter()
        .filter_map(|row| conversation_counterpart_id(row, account_id))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    counterpart_ids.sort();
    counterpart_ids.dedup();
    let profiles = social::profiles_by_accounts(store, &counterpart_ids)
        .await
        .map_err(ConversationError::Internal)?;

    rows.into_iter()
        .map(|row| {
            let kind = parse_conversation_kind(&row.kind)?;
            let counterpart = match kind {
                ConversationKind::Direct => {
                    let counterpart_id =
                        conversation_counterpart_id(&row, account_id).ok_or_else(|| {
                            ConversationError::InvalidKind(
                                "direct conversation missing counterpart".to_string(),
                            )
                        })?;
                    let profile = profiles.get(counterpart_id).ok_or_else(|| {
                        ConversationError::MissingProfile(counterpart_id.to_string())
                    })?;
                    Some(to_user_summary(profile))
                }
                ConversationKind::Group => None,
            };
            let title = match (&kind, &row.title, &counterpart) {
                (ConversationKind::Direct, _, Some(counterpart)) => {
                    counterpart.display_name.clone()
                }
                (ConversationKind::Group, Some(title), _) if !title.trim().is_empty() => {
                    title.clone()
                }
                (ConversationKind::Group, _, _) => "Group Chat".to_string(),
                _ => "Conversation".to_string(),
            };
            Ok(ConversationSummary {
                conversation_id: row.conversation_id,
                kind,
                title,
                counterpart,
                member_count: u32::try_from(row.member_count).unwrap_or(0),
                last_message_preview: row.last_message_preview,
                last_message_at_ms: row.last_message_at_ms,
                unread_count: u32::try_from(row.unread_count).unwrap_or(0),
                unread_mention_count: u32::try_from(row.unread_mention_count).unwrap_or(0),
            })
        })
        .collect()
}

pub async fn hydrate_messages(
    store: &StoreHandle,
    rows: Vec<social::ChatMessageRow>,
) -> Result<Vec<ChatMessageSummary>, ConversationError> {
    hydrate_messages_for_viewer(store, rows, None).await
}

/// Hydrate messages with optional viewer for `reacted_by_me` resolution.
pub async fn hydrate_messages_for_viewer(
    store: &StoreHandle,
    rows: Vec<social::ChatMessageRow>,
    viewer_account_id: Option<&str>,
) -> Result<Vec<ChatMessageSummary>, ConversationError> {
    let message_ids = rows
        .iter()
        .map(|row| row.message_id.clone())
        .collect::<Vec<_>>();
    let mut mentions_by_message = social::list_message_mentions_full(store, &message_ids).await?;
    let reaction_rows = social::list_for_messages(store, &message_ids).await?;
    let mut reactions_by_message: HashMap<String, Vec<social::MessageReactionRow>> = HashMap::new();
    for row in reaction_rows {
        reactions_by_message
            .entry(row.message_id.clone())
            .or_default()
            .push(row);
    }
    let attachment_join =
        crate::store::message_attachments::list_for_messages(store, &message_ids).await?;
    let mut attachments_by_message: HashMap<String, Vec<minos_protocol::ChatMessageAttachment>> =
        HashMap::new();
    for row in attachment_join {
        attachments_by_message
            .entry(row.message_id.clone())
            .or_default()
            .push(minos_protocol::ChatMessageAttachment {
                blob_id: row.blob_id,
                content_type: row.content_type,
                byte_size: row.byte_size,
                kind: row.kind,
                original_filename: row.original_filename,
            });
    }

    let mut reply_ids = rows
        .iter()
        .filter_map(|row| row.reply_to_message_id.clone())
        .collect::<Vec<_>>();
    reply_ids.sort();
    reply_ids.dedup();
    let reply_rows = social::list_messages_by_ids(store, &reply_ids)
        .await?
        .into_iter()
        .map(|row| (row.message_id.clone(), row))
        .collect::<HashMap<_, _>>();

    let mut profile_ids = rows
        .iter()
        .chain(reply_rows.values())
        .filter(|row| row.sender_type != "agent")
        .filter_map(|row| row.sender_account_id.clone())
        .collect::<Vec<_>>();
    profile_ids.sort();
    profile_ids.dedup();
    let profiles = social::profiles_by_accounts(store, &profile_ids).await?;

    let mut agent_ids = rows
        .iter()
        .chain(reply_rows.values())
        .filter(|row| row.sender_type == "agent")
        .map(agent_id_for_row)
        .collect::<Vec<_>>();
    agent_ids.sort();
    agent_ids.dedup();
    let agents = social::agents_by_ids(store, &agent_ids).await?;

    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let mentions = mentions_by_message
            .remove(&row.message_id)
            .unwrap_or_default();
        let reply_to = row
            .reply_to_message_id
            .as_ref()
            .and_then(|reply_id| reply_rows.get(reply_id))
            .map(
                |reply_row| -> Result<ChatMessageReplySummary, ConversationError> {
                    Ok(ChatMessageReplySummary {
                        message_id: reply_row.message_id.clone(),
                        sender: sender_summary(reply_row, &profiles, &agents)?,
                        text: reply_row.text.clone(),
                        recalled_at_ms: reply_row.recalled_at_ms,
                    })
                },
            )
            .transpose()?;
        let sender = sender_summary(&row, &profiles, &agents)?;
        let sender_type = ChatMessageSummary::sender_type_from(&sender);
        let reactions = reactions_by_message
            .remove(&row.message_id)
            .map(|rows| social::aggregate_groups(&rows, viewer_account_id))
            .unwrap_or_default();
        let attachments = attachments_by_message
            .remove(&row.message_id)
            .unwrap_or_default();
        output.push(ChatMessageSummary {
            message_id: row.message_id,
            conversation_id: row.conversation_id,
            sender,
            text: row.text,
            created_at_ms: row.created_at_ms,
            message_seq: row.message_seq,
            reply_to,
            recalled_at_ms: row.recalled_at_ms,
            mentioned_account_ids: mentions.account_ids,
            mentioned_agent_ids: mentions.agent_ids,
            sender_type,
            reactions,
            attachments,
        });
    }
    Ok(output)
}

fn conversation_counterpart_id<'a>(
    row: &'a social::ConversationDigestRow,
    account_id: &str,
) -> Option<&'a str> {
    if row.kind != "direct" {
        return None;
    }
    if row.direct_account_low.as_deref() == Some(account_id) {
        row.direct_account_high.as_deref()
    } else {
        row.direct_account_low.as_deref()
    }
}

fn parse_conversation_kind(kind: &str) -> Result<ConversationKind, ConversationError> {
    match kind {
        "direct" => Ok(ConversationKind::Direct),
        "group" => Ok(ConversationKind::Group),
        _ => Err(ConversationError::InvalidKind(kind.to_string())),
    }
}

fn sender_summary(
    row: &social::ChatMessageRow,
    profiles: &HashMap<String, social::ProfileRow>,
    agents: &HashMap<String, social::AgentRow>,
) -> Result<MessageSender, ConversationError> {
    if row.sender_type == "agent" {
        let agent_id = agent_id_for_row(row);
        return Ok(match agents.get(&agent_id) {
            Some(agent) => bot_sender_summary(Some(agent), &agent_id),
            None => bot_sender_summary(None, &agent_id),
        });
    }

    let account_id = row.sender_account_id.as_deref().ok_or_else(|| {
        ConversationError::MissingProfile("missing sender_account_id on user message".into())
    })?;
    let profile = profiles
        .get(account_id)
        .ok_or_else(|| ConversationError::MissingProfile(account_id.to_string()))?;
    Ok(MessageSender::from_user_summary(to_user_summary(profile)))
}

fn agent_id_for_row(row: &social::ChatMessageRow) -> String {
    // Authoritative bot identity is sender_agent_id. Never fall back to
    // sender_account_id (audit owner FK) — that would reintroduce the old
    // UserSummary.account_id = agent_id type lie.
    row.sender_agent_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| "unknown-bot".to_string())
}

/// Build a first-class bot author card from Hub agent digital body.
fn bot_sender_summary(agent: Option<&social::AgentRow>, agent_id: &str) -> MessageSender {
    match agent {
        Some(agent) => {
            // Prefer digital-body display_name; fall back to registry name.
            let label = {
                let d = agent.display_name.trim();
                if d.is_empty() {
                    agent.name.trim()
                } else {
                    d
                }
            };
            let display_name = if label.is_empty() {
                format!("🤖 {}", agent.agent_id)
            } else if label.starts_with('🤖') {
                label.to_string()
            } else {
                format!("🤖 {label}")
            };
            let name = {
                let n = agent.name.trim();
                if n.is_empty() {
                    None
                } else {
                    Some(n.to_string())
                }
            };
            MessageSender::Bot {
                bot_id: agent.agent_id.clone(),
                display_name,
                runtime_agent: agent.runtime_agent.clone(),
                name,
                avatar_url: agent.avatar_url.clone(),
            }
        }
        None => MessageSender::Bot {
            bot_id: agent_id.to_string(),
            display_name: "🤖 Unknown Agent".to_string(),
            runtime_agent: String::new(),
            name: None,
            avatar_url: None,
        },
    }
}

/// Validate client-provided structured mentions for persistence and delivery.
///
/// Body text never invents targets. Self-account mentions are dropped. Unknown
/// human account mentions are dropped. Structured **bot** mentions that are not
/// active conversation members fail the send (so sole-agent cannot soft-route
/// after a dropped @bot intent). Client order is ordinal authority.
pub(crate) fn validate_structured_mentions(
    structured: &[minos_protocol::MentionTarget],
    sender_account_id: &str,
    members: &[social::ProfileRow],
    active_agents: &[social::AgentRow],
    all_agents: &[social::AgentRow],
) -> Result<social::MessageMentions, String> {
    if structured.is_empty() {
        return Ok(social::MessageMentions {
            account_ids: Vec::new(),
            agent_ids: Vec::new(),
        });
    }
    let member_ids: std::collections::HashSet<&str> =
        members.iter().map(|m| m.account_id.as_str()).collect();
    let active_agent_ids: std::collections::HashSet<&str> =
        active_agents.iter().map(|a| a.agent_id.as_str()).collect();
    let mut account_ids = Vec::<String>::new();
    let mut agent_id_list = Vec::<String>::new();
    let mut seen_accounts = std::collections::HashSet::<String>::new();
    let mut seen_agents = std::collections::HashSet::<String>::new();

    for target in structured {
        match target {
            minos_protocol::MentionTarget::Account { account_id, .. } => {
                if account_id == sender_account_id {
                    continue;
                }
                if !member_ids.contains(account_id.as_str()) {
                    continue;
                }
                if seen_accounts.insert(account_id.clone()) {
                    account_ids.push(account_id.clone());
                }
            }
            minos_protocol::MentionTarget::Bot { bot_id, .. } => {
                if active_agent_ids.contains(bot_id.as_str()) {
                    if seen_agents.insert(bot_id.clone()) {
                        agent_id_list.push(bot_id.clone());
                    }
                    continue;
                }
                if let Some(disabled) = all_agents
                    .iter()
                    .find(|a| !a.is_active() && a.agent_id == *bot_id)
                {
                    let label = if disabled.display_name.trim().is_empty() {
                        disabled.name.as_str()
                    } else {
                        disabled.display_name.as_str()
                    };
                    return Err(format!(
                        "Agent「{label}」已停用，无法投递。请在 Agents 中重新启用后再试。"
                    ));
                }
                return Err(format!(
                    "未匹配到会话成员里的 Agent（{bot_id}）。请确认 Agent 已加入本会话。"
                ));
            }
        }
    }

    Ok(social::MessageMentions {
        account_ids,
        agent_ids: agent_id_list,
    })
}

/// Deprecated body-token helper retained only for non-delivery tooling/tests.
/// Delivery paths must use [`validate_structured_mentions`].
#[cfg(test)]
pub(crate) fn extract_participant_mentions(
    text: &str,
    sender_account_id: &str,
    members: &[social::ProfileRow],
    agents: &[social::AgentRow],
) -> social::MessageMentions {
    let by_minos_id = members
        .iter()
        .map(|member| (member.minos_id.as_str(), member.account_id.as_str()))
        .collect::<HashMap<_, _>>();
    let mut account_ids = Vec::<String>::new();
    let mut agent_ids = Vec::<String>::new();
    let mut seen_accounts = std::collections::HashSet::<String>::new();
    let mut seen_agents = std::collections::HashSet::<String>::new();

    for token in collect_mention_tokens(text) {
        let (name_part, _session_short) = split_agent_session_token(token);
        if let Some(account_id) = by_minos_id.get(name_part) {
            if *account_id != sender_account_id && seen_accounts.insert((*account_id).to_string()) {
                account_ids.push((*account_id).to_string());
            }
            continue;
        }
        if let Some(agent) = match_agent_token(name_part, agents) {
            if seen_agents.insert(agent.agent_id.clone()) {
                agent_ids.push(agent.agent_id.clone());
            }
        }
    }

    social::MessageMentions {
        account_ids,
        agent_ids,
    }
}

pub(crate) fn collect_mention_tokens(text: &str) -> Vec<&str> {
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
        // Allow `#short` so mid-body `@codex#abcd` keeps session targeting parity
        // with agent routing / Desktop parseAllAgentRoutings.
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric()
                || bytes[end] == b'-'
                || bytes[end] == b'_'
                || bytes[end] == b'#')
        {
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

pub(crate) fn split_agent_session_token(token: &str) -> (&str, Option<&str>) {
    match token.split_once('#') {
        Some((name, short)) if !name.is_empty() && !short.is_empty() => (name, Some(short)),
        _ => (token, None),
    }
}

pub(crate) fn match_agent_token<'a>(
    token: &str,
    agents: &'a [social::AgentRow],
) -> Option<&'a social::AgentRow> {
    let t = token.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    agents.iter().find(|agent| {
        agent.agent_id.eq_ignore_ascii_case(t)
            || agent.runtime_agent.eq_ignore_ascii_case(&lower)
            || agent.name.eq_ignore_ascii_case(t)
    })
}

/// First `@agent#short` session hint for a structured agent mention (text is not
/// a delivery target — only a session-resolution hint for agents already in
/// `mentioned_agent_ids`).
pub(crate) fn session_short_hint_for_agent<'a>(
    text: &'a str,
    agent: &social::AgentRow,
) -> Option<&'a str> {
    for token in collect_mention_tokens(text) {
        let (name_part, short) = split_agent_session_token(token);
        if short.is_none() {
            continue;
        }
        if match_agent_token(name_part, std::slice::from_ref(agent)).is_some() {
            return short;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::social::{AgentRow, ProfileRow};

    fn profile(account_id: &str, minos_id: &str) -> ProfileRow {
        ProfileRow {
            account_id: account_id.into(),
            email: format!("{minos_id}@example.com"),
            minos_id: minos_id.into(),
            display_name: Some(minos_id.into()),
        }
    }

    fn agent(agent_id: &str, name: &str, runtime: &str) -> AgentRow {
        AgentRow::test_stub(agent_id, "owner", name, "host_runtime", runtime)
    }

    #[test]
    fn validate_structured_mentions_accepts_membership_order() {
        let members = vec![profile("acct-alice", "alice"), profile("acct-bob", "bob")];
        let agents = vec![
            agent("bot-1", "Codex", "codex"),
            agent("bot-2", "Claude", "claude"),
        ];
        let structured = vec![
            minos_protocol::MentionTarget::bot("bot-2"),
            minos_protocol::MentionTarget::account("acct-bob"),
            minos_protocol::MentionTarget::bot("bot-1"),
            minos_protocol::MentionTarget::account("acct-alice"), // self dropped
            minos_protocol::MentionTarget::bot("bot-2"),          // dedupe
        ];
        let mentions =
            validate_structured_mentions(&structured, "acct-alice", &members, &agents, &agents)
                .expect("valid structured mentions");
        assert_eq!(mentions.account_ids, vec!["acct-bob".to_string()]);
        assert_eq!(
            mentions.agent_ids,
            vec!["bot-2".to_string(), "bot-1".to_string()]
        );
    }

    #[test]
    fn validate_structured_mentions_rejects_unknown_bot() {
        let members = vec![profile("acct-alice", "alice")];
        let agents = vec![agent("bot-1", "Codex", "codex")];
        let structured = vec![minos_protocol::MentionTarget::bot("bot-missing")];
        let err =
            validate_structured_mentions(&structured, "acct-alice", &members, &agents, &agents)
                .expect_err("unknown bot must fail");
        assert!(err.contains("bot-missing") || err.contains("未匹配"));
    }

    #[test]
    fn extract_participant_mentions_splits_humans_and_agents() {
        let members = vec![profile("acct-alice", "alice"), profile("acct-bob", "bob")];
        let agents = vec![
            agent("bot-1", "Codex", "codex"),
            agent("bot-2", "Claude", "claude"),
        ];
        let mentions = extract_participant_mentions(
            "@bob please and @codex#abcd fix it @claude",
            "acct-alice",
            &members,
            &agents,
        );
        assert_eq!(mentions.account_ids, vec!["acct-bob".to_string()]);
        assert_eq!(
            mentions.agent_ids,
            vec!["bot-1".to_string(), "bot-2".to_string()]
        );
    }

    #[test]
    fn extract_participant_mentions_preserves_body_appearance_order() {
        let members = vec![profile("acct-alice", "alice")];
        // Lex order of agent_id would be bot-a < bot-z; body order is z then a.
        let agents = vec![
            agent("bot-z", "Claude", "claude"),
            agent("bot-a", "Codex", "codex"),
        ];
        let mentions = extract_participant_mentions(
            "@claude first @codex second @claude again",
            "acct-alice",
            &members,
            &agents,
        );
        assert_eq!(
            mentions.agent_ids,
            vec!["bot-z".to_string(), "bot-a".to_string()]
        );
    }

    #[test]
    fn extract_participant_mentions_skips_self_and_unknown_tokens() {
        let members = vec![profile("acct-alice", "alice")];
        let agents = vec![agent("bot-1", "Codex", "codex")];
        let mentions =
            extract_participant_mentions("@alice @nobody @codex", "acct-alice", &members, &agents);
        assert!(mentions.account_ids.is_empty());
        assert_eq!(mentions.agent_ids, vec!["bot-1".to_string()]);
    }

    #[test]
    fn bot_sender_summary_prefers_display_name() {
        let mut row = agent("bot-1", "codex-bin", "codex");
        row.display_name = "Code Reviewer".into();
        let summary = bot_sender_summary(Some(&row), "bot-1");
        match summary {
            MessageSender::Bot {
                bot_id,
                display_name,
                runtime_agent,
                name,
                ..
            } => {
                assert_eq!(bot_id, "bot-1");
                assert_eq!(display_name, "🤖 Code Reviewer");
                assert_eq!(runtime_agent, "codex");
                assert_eq!(name.as_deref(), Some("codex-bin"));
            }
            MessageSender::Account { .. } => panic!("expected bot sender"),
        }

        row.display_name.clear();
        let summary = bot_sender_summary(Some(&row), "bot-1");
        assert_eq!(summary.display_name(), "🤖 codex-bin");

        let unknown = bot_sender_summary(None, "missing");
        assert_eq!(unknown.display_name(), "🤖 Unknown Agent");
        assert_eq!(unknown.bot_id(), Some("missing"));
    }

    #[test]
    fn session_short_hint_only_from_matching_agent_token() {
        let agent = agent("bot-1", "Codex", "codex");
        assert_eq!(
            session_short_hint_for_agent("@codex#abcd please", &agent),
            Some("abcd")
        );
        assert_eq!(session_short_hint_for_agent("@codex please", &agent), None);
        assert_eq!(
            session_short_hint_for_agent("@claude#zzzz please", &agent),
            None
        );
    }
}
