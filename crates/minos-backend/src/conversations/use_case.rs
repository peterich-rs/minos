use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use minos_protocol::{
    ChatMessageReplySummary, ChatMessageSummary, ConversationKind, ConversationResponse,
    ConversationSummary, SenderType, UserSummary,
};

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
    #[error(transparent)]
    Internal(#[from] BackendError),
}

#[derive(Debug, Clone)]
pub struct ListMessagesResult {
    pub messages: Vec<ChatMessageSummary>,
    pub next_before_ts_ms: Option<i64>,
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

    async fn list_members(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<UserSummary>, ConversationError>;

    async fn mark_read(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> Result<Option<i64>, ConversationError>;

    async fn list_messages(
        &self,
        account_id: &str,
        conversation_id: &str,
        before_ts_ms: Option<i64>,
        limit: u32,
    ) -> Result<ListMessagesResult, ConversationError>;

    async fn send_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        text: &str,
        reply_to_message_id: Option<&str>,
    ) -> Result<(ChatMessageSummary, Vec<social::ProfileRow>), ConversationError>;

    async fn recall_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<ChatMessageSummary, ConversationError>;

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

    async fn add_member(
        &self,
        conversation_id: &str,
        account_id: &str,
    ) -> Result<(), ConversationError>;
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
    ) -> Result<Option<i64>, ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let last_read_at_ms = social::mark_conversation_read_to_latest(
            &self.store,
            conversation_id,
            account_id,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(last_read_at_ms)
    }

    async fn list_messages(
        &self,
        account_id: &str,
        conversation_id: &str,
        before_ts_ms: Option<i64>,
        limit: u32,
    ) -> Result<ListMessagesResult, ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let limit = limit.min(200);
        let mut messages =
            social::list_messages(&self.store, conversation_id, before_ts_ms, limit).await?;
        let next_before_ts_ms = if messages.len() as u32 == limit {
            messages.last().map(|m| m.created_at_ms)
        } else {
            None
        };
        messages.reverse();
        let hydrated = hydrate_messages(&self.store, messages).await?;
        Ok(ListMessagesResult {
            messages: hydrated,
            next_before_ts_ms,
        })
    }

    async fn send_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        text: &str,
        reply_to_message_id: Option<&str>,
    ) -> Result<(ChatMessageSummary, Vec<social::ProfileRow>), ConversationError> {
        if !social::is_conversation_member(&self.store, conversation_id, account_id).await? {
            return Err(ConversationError::NotFound);
        }
        let reply_target = match reply_to_message_id {
            Some(message_id) => {
                let reply_target = social::get_message(&self.store, message_id)
                    .await?
                    .ok_or_else(|| {
                        ConversationError::ValidationFormat("reply target not found".into())
                    })?;
                if reply_target.conversation_id != conversation_id {
                    return Err(ConversationError::ValidationFormat(
                        "reply target not in conversation".into(),
                    ));
                }
                Some(reply_target)
            }
            None => None,
        };
        let reply_to_id = reply_target.as_ref().map(|row| row.message_id.clone());
        let members =
            social::list_conversation_member_profiles(&self.store, conversation_id).await?;
        let mentioned_account_ids = extract_mentioned_account_ids(text, account_id, &members);
        let row = social::insert_message(
            &self.store,
            conversation_id,
            account_id,
            text,
            chrono::Utc::now().timestamp_millis(),
            reply_to_id.as_deref(),
            &mentioned_account_ids,
        )
        .await?;
        let mut hydrated = hydrate_messages(&self.store, vec![row]).await?;
        let message = hydrated.remove(0);
        Ok((message, members))
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
        if existing.sender_account_id != account_id {
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
        let recalled =
            social::recall_message(&self.store, conversation_id, message_id, account_id, now_ms)
                .await?
                .ok_or(ConversationError::NotFound)?;
        let mut hydrated = hydrate_messages(&self.store, vec![recalled]).await?;
        Ok(hydrated.remove(0))
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
        conversation_id: &str,
        account_id: &str,
    ) -> Result<(), ConversationError> {
        social::add_member_to_group(
            &self.store,
            conversation_id,
            account_id,
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(())
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
    let message_ids = rows
        .iter()
        .map(|row| row.message_id.clone())
        .collect::<Vec<_>>();
    let mut mentions_by_message = social::list_message_mentions(store, &message_ids).await?;

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
        .map(|row| row.sender_account_id.clone())
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
        let mentioned_account_ids = mentions_by_message
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
            reply_to,
            recalled_at_ms: row.recalled_at_ms,
            mentioned_account_ids,
            sender_type,
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
) -> Result<UserSummary, ConversationError> {
    if row.sender_type == "agent" {
        let agent_id = agent_id_for_row(row);
        return Ok(match agents.get(&agent_id) {
            Some(agent) => agent_sender_summary(Some(agent), &agent_id),
            None => agent_sender_summary(None, &agent_id),
        });
    }

    let profile = profiles
        .get(&row.sender_account_id)
        .ok_or_else(|| ConversationError::MissingProfile(row.sender_account_id.clone()))?;
    Ok(to_user_summary(profile))
}

fn agent_id_for_row(row: &social::ChatMessageRow) -> String {
    row.sender_agent_id
        .clone()
        .unwrap_or_else(|| row.sender_account_id.clone())
}

fn agent_sender_summary(agent: Option<&social::AgentRow>, agent_id: &str) -> UserSummary {
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

pub(crate) fn extract_mentioned_account_ids(
    text: &str,
    sender_account_id: &str,
    members: &[social::ProfileRow],
) -> Vec<String> {
    let by_minos_id = members
        .iter()
        .map(|member| (member.minos_id.as_str(), member.account_id.as_str()))
        .collect::<HashMap<_, _>>();
    let mut mentions = std::collections::BTreeSet::<String>::new();

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
