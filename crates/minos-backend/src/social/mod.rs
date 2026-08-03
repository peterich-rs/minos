use std::{collections::HashMap, sync::Arc};

use minos_protocol::{
    ChatMessageReplySummary, ChatMessageSummary, ConversationKind, ConversationSummary,
    FriendRequestStatus, SenderType, UserSummary,
};

use crate::{
    error::BackendError,
    store::social::{
        self, AgentRow, ChatMessageRow, ConversationDigestRow, FriendRequestRow, ProfileRow,
        ResolveFriendRequestTxResult,
    },
    store::StoreHandle,
};

#[derive(Debug)]
pub enum ResolveFriendRequestError {
    NotFound,
    Unauthorized,
    AlreadyResolved,
    Internal(BackendError),
}

#[derive(Debug)]
pub enum SocialViewError {
    MissingProfile(String),
    InvalidConversationKind(String),
    Internal(BackendError),
}

pub struct SocialService {
    store: StoreHandle,
}

impl SocialService {
    #[must_use]
    pub fn new(store: impl Into<StoreHandle>) -> Arc<Self> {
        Arc::new(Self {
            store: store.into(),
        })
    }

    pub async fn resolve_friend_request(
        &self,
        acting_account_id: &str,
        request_id: &str,
        status: FriendRequestStatus,
    ) -> Result<FriendRequestRow, ResolveFriendRequestError> {
        let resolved_at_ms = chrono::Utc::now().timestamp_millis();
        match social::resolve_friend_request_transactional(
            &self.store,
            acting_account_id,
            request_id,
            status,
            resolved_at_ms,
        )
        .await
        .map_err(ResolveFriendRequestError::Internal)?
        {
            ResolveFriendRequestTxResult::Resolved(row) => Ok(row),
            ResolveFriendRequestTxResult::NotFound => Err(ResolveFriendRequestError::NotFound),
            ResolveFriendRequestTxResult::Unauthorized => {
                Err(ResolveFriendRequestError::Unauthorized)
            }
            ResolveFriendRequestTxResult::AlreadyResolved => {
                Err(ResolveFriendRequestError::AlreadyResolved)
            }
        }
    }

    pub async fn hydrate_conversations(
        &self,
        account_id: &str,
        rows: Vec<ConversationDigestRow>,
    ) -> Result<Vec<ConversationSummary>, SocialViewError> {
        let mut counterpart_ids = rows
            .iter()
            .filter_map(|row| conversation_counterpart_id(row, account_id))
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        counterpart_ids.sort();
        counterpart_ids.dedup();
        let profiles = social::profiles_by_accounts(&self.store, &counterpart_ids)
            .await
            .map_err(SocialViewError::Internal)?;

        rows.into_iter()
            .map(|row| {
                let kind = parse_conversation_kind(&row.kind)?;
                let counterpart = match kind {
                    ConversationKind::Direct => {
                        let counterpart_id = conversation_counterpart_id(&row, account_id)
                            .ok_or_else(|| {
                                SocialViewError::InvalidConversationKind(
                                    "direct conversation missing counterpart".to_string(),
                                )
                            })?;
                        let profile = profiles.get(counterpart_id).ok_or_else(|| {
                            SocialViewError::MissingProfile(counterpart_id.to_string())
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
        &self,
        rows: Vec<ChatMessageRow>,
    ) -> Result<Vec<ChatMessageSummary>, SocialViewError> {
        let message_ids = rows
            .iter()
            .map(|row| row.message_id.clone())
            .collect::<Vec<_>>();
        let mut mentions_by_message = social::list_message_mentions(&self.store, &message_ids)
            .await
            .map_err(SocialViewError::Internal)?;

        let mut reply_ids = rows
            .iter()
            .filter_map(|row| row.reply_to_message_id.clone())
            .collect::<Vec<_>>();
        reply_ids.sort();
        reply_ids.dedup();
        let reply_rows = social::list_messages_by_ids(&self.store, &reply_ids)
            .await
            .map_err(SocialViewError::Internal)?
            .into_iter()
            .map(|row| (row.message_id.clone(), row))
            .collect::<HashMap<_, _>>();

        let mut profile_ids = rows
            .iter()
            .chain(reply_rows.values())
            .filter(|row| row.sender_type != "agent")
            .map(|row| row.sender_account_id.clone())
            .collect::<Vec<_>>();
        profile_ids.sort();
        profile_ids.dedup();
        let profiles = social::profiles_by_accounts(&self.store, &profile_ids)
            .await
            .map_err(SocialViewError::Internal)?;

        let mut agent_ids = rows
            .iter()
            .chain(reply_rows.values())
            .filter(|row| row.sender_type == "agent")
            .map(agent_id_for_row)
            .collect::<Vec<_>>();
        agent_ids.sort();
        agent_ids.dedup();
        let agents = social::agents_by_ids(&self.store, &agent_ids)
            .await
            .map_err(SocialViewError::Internal)?;

        let mut output = Vec::with_capacity(rows.len());
        for row in rows {
            let mentioned_account_ids = mentions_by_message
                .remove(&row.message_id)
                .unwrap_or_default();
            let reply_to = row
                .reply_to_message_id
                .as_ref()
                .and_then(|reply_id| reply_rows.get(reply_id))
                .map(|reply_row| {
                    Ok(ChatMessageReplySummary {
                        message_id: reply_row.message_id.clone(),
                        sender: sender_summary(reply_row, &profiles, &agents)?,
                        text: reply_row.text.clone(),
                        recalled_at_ms: reply_row.recalled_at_ms,
                    })
                })
                .transpose()?;
            let sender = sender_summary(&row, &profiles, &agents)?;
            let sender_type = if row.sender_type == "agent" {
                SenderType::Agent
            } else {
                SenderType::User
            };
            output.push(ChatMessageSummary {
                message_id: row.message_id,
                conversation_id: row.conversation_id,
                sender,
                text: row.text,
                created_at_ms: row.created_at_ms,
                message_seq: row.message_seq,
                reply_to,
                recalled_at_ms: row.recalled_at_ms,
                mentioned_account_ids,
                sender_type,
                reactions: vec![],
            });
        }
        Ok(output)
    }
}

fn conversation_counterpart_id<'a>(
    row: &'a ConversationDigestRow,
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

fn parse_conversation_kind(kind: &str) -> Result<ConversationKind, SocialViewError> {
    match kind {
        "direct" => Ok(ConversationKind::Direct),
        "group" => Ok(ConversationKind::Group),
        _ => Err(SocialViewError::InvalidConversationKind(kind.to_string())),
    }
}

fn sender_summary(
    row: &ChatMessageRow,
    profiles: &HashMap<String, ProfileRow>,
    agents: &HashMap<String, AgentRow>,
) -> Result<UserSummary, SocialViewError> {
    if row.sender_type == "agent" {
        let agent_id = agent_id_for_row(row);
        return Ok(match agents.get(&agent_id) {
            Some(agent) => agent_sender_summary(Some(agent), &agent_id),
            None => agent_sender_summary(None, &agent_id),
        });
    }

    let profile = profiles
        .get(&row.sender_account_id)
        .ok_or_else(|| SocialViewError::MissingProfile(row.sender_account_id.clone()))?;
    Ok(to_user_summary(profile))
}

fn agent_id_for_row(row: &ChatMessageRow) -> String {
    row.sender_agent_id
        .clone()
        .unwrap_or_else(|| row.sender_account_id.clone())
}

fn display_name(profile: &ProfileRow) -> String {
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

fn to_user_summary(profile: &ProfileRow) -> UserSummary {
    UserSummary {
        account_id: profile.account_id.clone(),
        minos_id: profile.minos_id.clone(),
        display_name: display_name(profile),
    }
}

fn agent_sender_summary(agent: Option<&AgentRow>, agent_id: &str) -> UserSummary {
    match agent {
        Some(agent) => UserSummary {
            account_id: agent.agent_id.clone(),
            minos_id: agent.agent_id.clone(),
            display_name: format!("🤖 {}", agent.name),
        },
        None => UserSummary {
            account_id: agent_id.to_string(),
            minos_id: agent_id.to_string(),
            display_name: "🤖 Unknown Agent".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, memory_pool};

    #[tokio::test]
    async fn accept_request_creates_friendship_atomically() {
        let pool = memory_pool().await;
        let from_account = insert_account(&pool, "social-service-a@example.com").await;
        let to_account = insert_account(&pool, "social-service-b@example.com").await;
        let request_id = social::create_friend_request(&pool, &from_account, &to_account, 10)
            .await
            .unwrap();
        let service = SocialService::new(pool.clone());

        let row = service
            .resolve_friend_request(&to_account, &request_id, FriendRequestStatus::Accepted)
            .await
            .unwrap();

        assert_eq!(row.status, "accepted");
        assert!(social::are_friends(&pool, &from_account, &to_account)
            .await
            .unwrap());
    }
}
