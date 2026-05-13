use std::collections::HashMap;

use minos_protocol::FriendRequestStatus;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::error::BackendError;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ProfileRow {
    pub account_id: String,
    pub email: String,
    pub minos_id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct FriendRequestRow {
    pub request_id: String,
    pub from_account_id: String,
    pub to_account_id: String,
    pub status: String,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct FriendshipRow {
    pub friendship_id: String,
    pub account_low_id: String,
    pub account_high_id: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ConversationRow {
    pub conversation_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub created_by_account_id: String,
    pub direct_account_low: Option<String>,
    pub direct_account_high: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ConversationDigestRow {
    pub conversation_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub created_by_account_id: String,
    pub direct_account_low: Option<String>,
    pub direct_account_high: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub member_count: i64,
    pub last_message_preview: Option<String>,
    pub last_message_at_ms: i64,
    pub unread_count: i64,
    pub unread_mention_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ChatMessageRow {
    pub message_id: String,
    pub conversation_id: String,
    pub sender_account_id: String,
    pub sender_agent_id: Option<String>,
    pub text: String,
    pub created_at_ms: i64,
    pub reply_to_message_id: Option<String>,
    pub recalled_at_ms: Option<i64>,
    pub sender_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct MessageMentionRow {
    message_id: String,
    mentioned_account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AgentRow {
    pub agent_id: String,
    pub owner_account_id: String,
    pub name: String,
    pub description: String,
    pub runtime_agent: String,
    pub model: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ConversationAgentMemberRow {
    pub conversation_id: String,
    pub agent_id: String,
    pub added_by_account_id: String,
    pub joined_at_ms: i64,
}

pub async fn profile_by_account(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Option<ProfileRow>, BackendError> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT account_id, email, minos_id, display_name
           FROM accounts
          WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::profile_by_account"))
}

/// Batch-load profiles for multiple account IDs in a single query.
/// Returns a map from account_id to ProfileRow. Missing accounts are
/// silently omitted from the result.
pub async fn profiles_by_accounts(
    pool: &SqlitePool,
    account_ids: &[String],
) -> Result<HashMap<String, ProfileRow>, BackendError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT account_id, email, minos_id, display_name FROM accounts WHERE account_id IN (",
    );
    {
        let mut separated = builder.separated(", ");
        for id in account_ids {
            separated.push_bind(id);
        }
    }
    builder.push(')');
    let rows = builder
        .build_query_as::<ProfileRow>()
        .fetch_all(pool)
        .await
        .map_err(store_err("social::profiles_by_accounts"))?;
    Ok(rows
        .into_iter()
        .map(|r| (r.account_id.clone(), r))
        .collect())
}

pub async fn find_by_minos_id(
    pool: &SqlitePool,
    minos_id: &str,
) -> Result<Option<ProfileRow>, BackendError> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT account_id, email, minos_id, display_name
           FROM accounts
          WHERE minos_id = ? COLLATE BINARY",
    )
    .bind(minos_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::find_by_minos_id"))
}

pub async fn search_by_minos_id_prefix(
    pool: &SqlitePool,
    query: &str,
) -> Result<Vec<ProfileRow>, BackendError> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT account_id, email, minos_id, display_name
           FROM accounts
          WHERE substr(minos_id, 1, length(?)) = ?
          ORDER BY CASE WHEN minos_id = ? THEN 0 ELSE 1 END, minos_id
          LIMIT 20",
    )
    .bind(query)
    .bind(query)
    .bind(query)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::search_by_minos_id_prefix"))
}

pub async fn set_minos_id(
    pool: &SqlitePool,
    account_id: &str,
    minos_id: &str,
) -> Result<(), BackendError> {
    sqlx::query("UPDATE accounts SET minos_id = ? WHERE account_id = ?")
        .bind(minos_id)
        .bind(account_id)
        .execute(pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => BackendError::StoreQuery {
                operation: "social::set_minos_id".into(),
                message: "minos_id_taken".into(),
            },
            _ => BackendError::StoreQuery {
                operation: "social::set_minos_id".into(),
                message: e.to_string(),
            },
        })?;
    Ok(())
}

pub async fn set_display_name(
    pool: &SqlitePool,
    account_id: &str,
    display_name: Option<&str>,
) -> Result<(), BackendError> {
    sqlx::query("UPDATE accounts SET display_name = ? WHERE account_id = ?")
        .bind(display_name)
        .bind(account_id)
        .execute(pool)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: "social::set_display_name".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

pub async fn create_friend_request(
    pool: &SqlitePool,
    from_account_id: &str,
    to_account_id: &str,
    created_at_ms: i64,
) -> Result<String, BackendError> {
    let request_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO friend_requests
            (request_id, from_account_id, to_account_id, status, created_at_ms)
         VALUES (?, ?, ?, 'pending', ?)",
    )
    .bind(&request_id)
    .bind(from_account_id)
    .bind(to_account_id)
    .bind(created_at_ms)
    .execute(pool)
    .await
    .map_err(store_err("social::create_friend_request"))?;
    Ok(request_id)
}

pub async fn get_friend_request(
    pool: &SqlitePool,
    request_id: &str,
) -> Result<Option<FriendRequestRow>, BackendError> {
    sqlx::query_as::<_, FriendRequestRow>(
        "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
           FROM friend_requests
          WHERE request_id = ?",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::get_friend_request"))
}

pub async fn list_incoming_friend_requests(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<FriendRequestRow>, BackendError> {
    sqlx::query_as::<_, FriendRequestRow>(
        "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
           FROM friend_requests
          WHERE to_account_id = ?
          ORDER BY created_at_ms DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_incoming_friend_requests"))
}

pub async fn list_outgoing_friend_requests(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<FriendRequestRow>, BackendError> {
    sqlx::query_as::<_, FriendRequestRow>(
        "SELECT request_id, from_account_id, to_account_id, status, created_at_ms, resolved_at_ms
           FROM friend_requests
          WHERE from_account_id = ?
          ORDER BY created_at_ms DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_outgoing_friend_requests"))
}

pub async fn has_pending_friend_request_between(
    pool: &SqlitePool,
    left: &str,
    right: &str,
) -> Result<bool, BackendError> {
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
           FROM friend_requests
          WHERE status = 'pending'
            AND ((from_account_id = ? AND to_account_id = ?) OR
                 (from_account_id = ? AND to_account_id = ?))",
    )
    .bind(left)
    .bind(right)
    .bind(right)
    .bind(left)
    .fetch_one(pool)
    .await
    .map_err(store_err("social::has_pending_friend_request_between"))?;
    Ok(row > 0)
}

pub async fn resolve_friend_request(
    pool: &SqlitePool,
    request_id: &str,
    status: FriendRequestStatus,
    resolved_at_ms: i64,
) -> Result<bool, BackendError> {
    let status = match status {
        FriendRequestStatus::Pending => "pending",
        FriendRequestStatus::Accepted => "accepted",
        FriendRequestStatus::Rejected => "rejected",
        FriendRequestStatus::Canceled => "canceled",
    };
    let result = sqlx::query(
        "UPDATE friend_requests
            SET status = ?, resolved_at_ms = ?
          WHERE request_id = ? AND status = 'pending'",
    )
    .bind(status)
    .bind(resolved_at_ms)
    .bind(request_id)
    .execute(pool)
    .await
    .map_err(store_err("social::resolve_friend_request"))?;
    Ok(result.rows_affected() == 1)
}

pub async fn create_friendship(
    pool: &SqlitePool,
    left: &str,
    right: &str,
    created_at_ms: i64,
) -> Result<(), BackendError> {
    let (low, high) = normalized_pair(left, right);
    sqlx::query(
        "INSERT OR IGNORE INTO friendships
            (friendship_id, account_low_id, account_high_id, created_at_ms)
         VALUES (?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(low)
    .bind(high)
    .bind(created_at_ms)
    .execute(pool)
    .await
    .map_err(store_err("social::create_friendship"))?;
    Ok(())
}

pub async fn are_friends(pool: &SqlitePool, left: &str, right: &str) -> Result<bool, BackendError> {
    let (low, high) = normalized_pair(left, right);
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
           FROM friendships
          WHERE account_low_id = ? AND account_high_id = ?",
    )
    .bind(low)
    .bind(high)
    .fetch_one(pool)
    .await
    .map_err(store_err("social::are_friends"))?;
    Ok(row > 0)
}

pub async fn list_friendships_for(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<FriendshipRow>, BackendError> {
    sqlx::query_as::<_, FriendshipRow>(
        "SELECT friendship_id, account_low_id, account_high_id, created_at_ms
           FROM friendships
          WHERE account_low_id = ? OR account_high_id = ?
          ORDER BY created_at_ms DESC",
    )
    .bind(account_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_friendships_for"))
}

pub async fn ensure_direct_conversation(
    pool: &SqlitePool,
    creator_account_id: &str,
    left: &str,
    right: &str,
    now_ms: i64,
) -> Result<ConversationRow, BackendError> {
    let (low, high) = normalized_pair(left, right);
    if let Some(existing) = find_direct_conversation(pool, low, high).await? {
        return Ok(existing);
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("social::ensure_direct_conversation.begin"))?;
    let conversation_id = Uuid::new_v4().to_string();
    let insert_result = sqlx::query(
        "INSERT INTO conversations
            (conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms)
         VALUES (?, 'direct', NULL, ?, ?, ?, ?, ?)",
    )
    .bind(&conversation_id)
    .bind(creator_account_id)
    .bind(low)
    .bind(high)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await;

    match insert_result {
        Ok(_) => {
            for member in [low, high] {
                sqlx::query(
                    "INSERT INTO conversation_members (conversation_id, account_id, joined_at_ms)
                     VALUES (?, ?, ?)",
                )
                .bind(&conversation_id)
                .bind(member)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .map_err(store_err(
                    "social::ensure_direct_conversation.insert_member",
                ))?;
            }
            tx.commit()
                .await
                .map_err(store_err("social::ensure_direct_conversation.commit"))?;
            get_conversation(pool, &conversation_id)
                .await?
                .ok_or_else(|| BackendError::StoreQuery {
                    operation: "social::ensure_direct_conversation.load".into(),
                    message: "conversation missing after insert".into(),
                })
        }
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            // Concurrent insert won the race — rollback and fetch the winner.
            tx.rollback().await.ok();
            find_direct_conversation(pool, low, high)
                .await?
                .ok_or_else(|| BackendError::StoreQuery {
                    operation: "social::ensure_direct_conversation.race_fallback".into(),
                    message: "conversation missing after unique violation".into(),
                })
        }
        Err(e) => Err(store_err(
            "social::ensure_direct_conversation.insert_conversation",
        )(e)),
    }
}

pub async fn create_group_conversation(
    pool: &SqlitePool,
    creator_account_id: &str,
    title: &str,
    member_account_ids: &[String],
    now_ms: i64,
) -> Result<ConversationRow, BackendError> {
    let mut members = member_account_ids.to_vec();
    if !members.iter().any(|member| member == creator_account_id) {
        members.push(creator_account_id.to_string());
    }
    members.sort();
    members.dedup();

    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("social::create_group_conversation.begin"))?;
    let conversation_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO conversations
            (conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms)
         VALUES (?, 'group', ?, ?, NULL, NULL, ?, ?)",
    )
    .bind(&conversation_id)
    .bind(title)
    .bind(creator_account_id)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::create_group_conversation.insert_conversation"))?;
    for member in members {
        sqlx::query(
            "INSERT INTO conversation_members (conversation_id, account_id, joined_at_ms)
             VALUES (?, ?, ?)",
        )
        .bind(&conversation_id)
        .bind(member)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(store_err("social::create_group_conversation.insert_member"))?;
    }
    tx.commit()
        .await
        .map_err(store_err("social::create_group_conversation.commit"))?;
    get_conversation(pool, &conversation_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::create_group_conversation.load".into(),
            message: "conversation missing after insert".into(),
        })
}

pub async fn get_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
) -> Result<Option<ConversationRow>, BackendError> {
    sqlx::query_as::<_, ConversationRow>(
        "SELECT conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms
           FROM conversations
          WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::get_conversation"))
}

pub async fn list_conversations_for(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<ConversationDigestRow>, BackendError> {
    sqlx::query_as::<_, ConversationDigestRow>(
        "SELECT
            c.conversation_id,
            c.kind,
            c.title,
            c.created_by_account_id,
            c.direct_account_low,
            c.direct_account_high,
            c.created_at_ms,
            c.updated_at_ms,
            (SELECT COUNT(*) FROM conversation_members cm2 WHERE cm2.conversation_id = c.conversation_id) AS member_count,
            (SELECT m.text FROM chat_messages m WHERE m.conversation_id = c.conversation_id ORDER BY m.created_at_ms DESC LIMIT 1) AS last_message_preview,
                        COALESCE((SELECT MAX(m.created_at_ms) FROM chat_messages m WHERE m.conversation_id = c.conversation_id), c.updated_at_ms) AS last_message_at_ms,
                        COALESCE((
                                SELECT COUNT(*)
                                    FROM chat_messages m
                                 WHERE m.conversation_id = c.conversation_id
                                     AND m.sender_account_id <> ?
                                     AND m.recalled_at_ms IS NULL
                                     AND m.created_at_ms > COALESCE(cr.last_read_at_ms, 0)
                        ), 0) AS unread_count,
                        COALESCE((
                                SELECT COUNT(*)
                                    FROM chat_messages m
                                    JOIN chat_message_mentions mm ON mm.message_id = m.message_id
                                 WHERE m.conversation_id = c.conversation_id
                                     AND m.sender_account_id <> ?
                                     AND m.recalled_at_ms IS NULL
                                     AND m.created_at_ms > COALESCE(cr.last_read_at_ms, 0)
                                     AND mm.mentioned_account_id = ?
                        ), 0) AS unread_mention_count
          FROM conversations c
          JOIN conversation_members cm ON cm.conversation_id = c.conversation_id
                    LEFT JOIN conversation_reads cr
                        ON cr.conversation_id = c.conversation_id
                     AND cr.account_id = ?
         WHERE cm.account_id = ?
         ORDER BY last_message_at_ms DESC",
    )
        .bind(account_id)
        .bind(account_id)
        .bind(account_id)
        .bind(account_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_conversations_for"))
}

pub async fn list_conversation_member_profiles(
    pool: &SqlitePool,
    conversation_id: &str,
) -> Result<Vec<ProfileRow>, BackendError> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT a.account_id, a.email, a.minos_id, a.display_name
                     FROM accounts a
                     JOIN conversation_members cm ON cm.account_id = a.account_id
                    WHERE cm.conversation_id = ?
                    ORDER BY cm.joined_at_ms ASC, a.account_id ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_conversation_member_profiles"))
}

pub async fn list_conversation_members(
    pool: &SqlitePool,
    conversation_id: &str,
) -> Result<Vec<String>, BackendError> {
    sqlx::query_scalar::<_, String>(
        "SELECT account_id
           FROM conversation_members
          WHERE conversation_id = ?
          ORDER BY joined_at_ms ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_conversation_members"))
}

pub async fn is_conversation_member(
    pool: &SqlitePool,
    conversation_id: &str,
    account_id: &str,
) -> Result<bool, BackendError> {
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
           FROM conversation_members
          WHERE conversation_id = ? AND account_id = ?",
    )
    .bind(conversation_id)
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(store_err("social::is_conversation_member"))?;
    Ok(row > 0)
}

pub async fn get_message(
    pool: &SqlitePool,
    message_id: &str,
) -> Result<Option<ChatMessageRow>, BackendError> {
    sqlx::query_as::<_, ChatMessageRow>(
        "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, reply_to_message_id, recalled_at_ms, sender_type
           FROM chat_messages
          WHERE message_id = ?",
    )
    .bind(message_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::get_message"))
}

pub async fn list_messages(
    pool: &SqlitePool,
    conversation_id: &str,
    before_ts_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<ChatMessageRow>, BackendError> {
    let effective_limit = i64::from(limit.min(200));
    let before = before_ts_ms.unwrap_or(i64::MAX);
    sqlx::query_as::<_, ChatMessageRow>(
        "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, reply_to_message_id, recalled_at_ms, sender_type
           FROM chat_messages
          WHERE conversation_id = ? AND created_at_ms < ?
          ORDER BY created_at_ms DESC
          LIMIT ?",
    )
    .bind(conversation_id)
    .bind(before)
    .bind(effective_limit)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_messages"))
}

pub async fn list_messages_by_ids(
    pool: &SqlitePool,
    message_ids: &[String],
) -> Result<Vec<ChatMessageRow>, BackendError> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, reply_to_message_id, recalled_at_ms, sender_type\n           FROM chat_messages\n          WHERE message_id IN (",
    );
    {
        let mut separated = builder.separated(", ");
        for message_id in message_ids {
            separated.push_bind(message_id);
        }
    }
    builder.push(')');

    builder
        .build_query_as::<ChatMessageRow>()
        .fetch_all(pool)
        .await
        .map_err(store_err("social::list_messages_by_ids"))
}

pub async fn list_message_mentions(
    pool: &SqlitePool,
    message_ids: &[String],
) -> Result<HashMap<String, Vec<String>>, BackendError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT message_id, mentioned_account_id
           FROM chat_message_mentions
          WHERE message_id IN (",
    );
    {
        let mut separated = builder.separated(", ");
        for message_id in message_ids {
            separated.push_bind(message_id);
        }
    }
    builder.push(") ORDER BY message_id ASC, mentioned_account_id ASC");

    let rows = builder
        .build_query_as::<MessageMentionRow>()
        .fetch_all(pool)
        .await
        .map_err(store_err("social::list_message_mentions"))?;

    let mut mentions_by_message = HashMap::<String, Vec<String>>::new();
    for row in rows {
        mentions_by_message
            .entry(row.message_id)
            .or_default()
            .push(row.mentioned_account_id);
    }
    Ok(mentions_by_message)
}

pub async fn mark_conversation_read_to_latest(
    pool: &SqlitePool,
    conversation_id: &str,
    account_id: &str,
    updated_at_ms: i64,
) -> Result<Option<i64>, BackendError> {
    let Some(last_read_at_ms) = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(created_at_ms)
           FROM chat_messages
          WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_one(pool)
    .await
    .map_err(store_err(
        "social::mark_conversation_read_to_latest.fetch_latest",
    ))?
    else {
        return Ok(None);
    };

    sqlx::query(
        "INSERT INTO conversation_reads
            (conversation_id, account_id, last_read_at_ms, updated_at_ms)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(conversation_id, account_id) DO UPDATE SET
            last_read_at_ms = CASE
                WHEN excluded.last_read_at_ms > conversation_reads.last_read_at_ms
                    THEN excluded.last_read_at_ms
                ELSE conversation_reads.last_read_at_ms
            END,
            updated_at_ms = excluded.updated_at_ms",
    )
    .bind(conversation_id)
    .bind(account_id)
    .bind(last_read_at_ms)
    .bind(updated_at_ms)
    .execute(pool)
    .await
    .map_err(store_err("social::mark_conversation_read_to_latest.upsert"))?;

    Ok(Some(last_read_at_ms))
}

pub async fn insert_message(
    pool: &SqlitePool,
    conversation_id: &str,
    sender_account_id: &str,
    text: &str,
    created_at_ms: i64,
    reply_to_message_id: Option<&str>,
    mentioned_account_ids: &[String],
) -> Result<ChatMessageRow, BackendError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("social::insert_message.begin"))?;
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO chat_messages
            (message_id, conversation_id, sender_account_id, text, created_at_ms, reply_to_message_id)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(conversation_id)
    .bind(sender_account_id)
    .bind(text)
    .bind(created_at_ms)
    .bind(reply_to_message_id)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::insert_message.insert"))?;
    let mut unique_mentions = mentioned_account_ids.to_vec();
    unique_mentions.sort();
    unique_mentions.dedup();
    for mentioned_account_id in unique_mentions {
        sqlx::query(
            "INSERT INTO chat_message_mentions
                (message_id, mentioned_account_id)
             VALUES (?, ?)",
        )
        .bind(&message_id)
        .bind(mentioned_account_id)
        .execute(&mut *tx)
        .await
        .map_err(store_err("social::insert_message.insert_mention"))?;
    }
    sqlx::query(
        "UPDATE conversations
            SET updated_at_ms = ?
          WHERE conversation_id = ?",
    )
    .bind(created_at_ms)
    .bind(conversation_id)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::insert_message.touch_conversation"))?;
    tx.commit()
        .await
        .map_err(store_err("social::insert_message.commit"))?;
    Ok(ChatMessageRow {
        message_id,
        conversation_id: conversation_id.to_string(),
        sender_account_id: sender_account_id.to_string(),
        sender_agent_id: None,
        text: text.to_string(),
        created_at_ms,
        reply_to_message_id: reply_to_message_id.map(ToOwned::to_owned),
        recalled_at_ms: None,
        sender_type: "user".to_string(),
    })
}

pub async fn bind_session_to_message(
    pool: &SqlitePool,
    message_id: &str,
    session_id: &str,
) -> Result<(), BackendError> {
    sqlx::query(
        "UPDATE chat_messages
            SET agent_session_id = ?
          WHERE message_id = ?",
    )
    .bind(session_id)
    .bind(message_id)
    .execute(pool)
    .await
    .map_err(store_err("social::bind_session_to_message"))?;
    Ok(())
}

pub async fn lookup_session_id_for_message(
    pool: &SqlitePool,
    message_id: &str,
) -> Result<Option<String>, BackendError> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT agent_session_id
           FROM chat_messages
          WHERE message_id = ?",
    )
    .bind(message_id)
    .fetch_optional(pool)
    .await
    .map(|value| value.flatten())
    .map_err(store_err("social::lookup_session_id_for_message"))
}

pub async fn lookup_latest_session_id_for_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
) -> Result<Option<String>, BackendError> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT agent_session_id
           FROM chat_messages
          WHERE conversation_id = ?
            AND agent_session_id IS NOT NULL
          ORDER BY created_at_ms DESC
          LIMIT 1",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await
    .map(|value| value.flatten())
    .map_err(store_err(
        "social::lookup_latest_session_id_for_conversation",
    ))
}

pub async fn has_bound_message_for_session(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<bool, BackendError> {
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
           FROM chat_messages
          WHERE agent_session_id = ?",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .map_err(store_err("social::has_bound_message_for_session"))?;
    Ok(row > 0)
}

pub async fn suppress_live_ui_fanout_for_session(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<bool, BackendError> {
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
             SELECT 1
               FROM chat_messages m
               JOIN conversation_members cm ON cm.conversation_id = m.conversation_id
              WHERE m.agent_session_id = ?
              GROUP BY m.conversation_id
             HAVING COUNT(DISTINCT cm.account_id) > 1
         )",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .map_err(store_err("social::suppress_live_ui_fanout_for_session"))?;
    Ok(row > 0)
}

pub async fn recall_message(
    pool: &SqlitePool,
    conversation_id: &str,
    message_id: &str,
    sender_account_id: &str,
    recalled_at_ms: i64,
) -> Result<Option<ChatMessageRow>, BackendError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("social::recall_message.begin"))?;
    sqlx::query(
        "UPDATE chat_messages
            SET text = '[message recalled]',
                recalled_at_ms = COALESCE(recalled_at_ms, ?)
          WHERE message_id = ?
            AND conversation_id = ?
            AND sender_account_id = ?",
    )
    .bind(recalled_at_ms)
    .bind(message_id)
    .bind(conversation_id)
    .bind(sender_account_id)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::recall_message.update_message"))?;

    sqlx::query(
        "DELETE FROM chat_message_mentions
          WHERE message_id = ?",
    )
    .bind(message_id)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::recall_message.delete_mentions"))?;

    sqlx::query(
        "UPDATE conversations
            SET updated_at_ms = MAX(updated_at_ms, ?)
          WHERE conversation_id = ?",
    )
    .bind(recalled_at_ms)
    .bind(conversation_id)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::recall_message.touch_conversation"))?;

    tx.commit()
        .await
        .map_err(store_err("social::recall_message.commit"))?;

    get_message(pool, message_id).await
}

fn normalized_pair<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

async fn find_direct_conversation(
    pool: &SqlitePool,
    low: &str,
    high: &str,
) -> Result<Option<ConversationRow>, BackendError> {
    sqlx::query_as::<_, ConversationRow>(
        "SELECT conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms
           FROM conversations
          WHERE kind = 'direct'
            AND direct_account_low = ?
            AND direct_account_high = ?",
    )
    .bind(low)
    .bind(high)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::find_direct_conversation"))
}

fn store_err(operation: &'static str) -> impl FnOnce(sqlx::Error) -> BackendError {
    move |e| BackendError::StoreQuery {
        operation: operation.into(),
        message: e.to_string(),
    }
}

// ─── Agent Store Functions ─────────────────────────────────────────────

pub async fn register_agent(
    pool: &SqlitePool,
    owner_account_id: &str,
    name: &str,
    description: &str,
    runtime_agent: &str,
    model: &str,
    now_ms: i64,
) -> Result<AgentRow, BackendError> {
    let agent_id = format!("bot-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO agents (agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&agent_id)
    .bind(owner_account_id)
    .bind(name)
    .bind(description)
    .bind(runtime_agent)
    .bind(model)
    .bind(now_ms)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(store_err("social::register_agent"))?;

    get_agent(pool, &agent_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::register_agent.load".into(),
            message: "agent missing after insert".into(),
        })
}

pub async fn get_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<Option<AgentRow>, BackendError> {
    sqlx::query_as::<_, AgentRow>(
        "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms
           FROM agents WHERE agent_id = ?",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::get_agent"))
}

pub async fn list_agents_for_owner(
    pool: &SqlitePool,
    owner_account_id: &str,
) -> Result<Vec<AgentRow>, BackendError> {
    sqlx::query_as::<_, AgentRow>(
        "SELECT agent_id, owner_account_id, name, description, runtime_agent, model, created_at_ms, updated_at_ms
           FROM agents WHERE owner_account_id = ? ORDER BY created_at_ms DESC",
    )
    .bind(owner_account_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_agents_for_owner"))
}

pub async fn delete_agent(
    pool: &SqlitePool,
    agent_id: &str,
    owner_account_id: &str,
) -> Result<bool, BackendError> {
    let result = sqlx::query("DELETE FROM agents WHERE agent_id = ? AND owner_account_id = ?")
        .bind(agent_id)
        .bind(owner_account_id)
        .execute(pool)
        .await
        .map_err(store_err("social::delete_agent"))?;
    Ok(result.rows_affected() > 0)
}

pub async fn add_agent_to_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
    agent_id: &str,
    added_by_account_id: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT OR IGNORE INTO conversation_agent_members (conversation_id, agent_id, added_by_account_id, joined_at_ms)
         VALUES (?, ?, ?, ?)",
    )
    .bind(conversation_id)
    .bind(agent_id)
    .bind(added_by_account_id)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(store_err("social::add_agent_to_conversation"))?;
    Ok(())
}

pub async fn remove_agent_from_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
    agent_id: &str,
) -> Result<bool, BackendError> {
    let result = sqlx::query(
        "DELETE FROM conversation_agent_members WHERE conversation_id = ? AND agent_id = ?",
    )
    .bind(conversation_id)
    .bind(agent_id)
    .execute(pool)
    .await
    .map_err(store_err("social::remove_agent_from_conversation"))?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_conversation_agents(
    pool: &SqlitePool,
    conversation_id: &str,
) -> Result<Vec<AgentRow>, BackendError> {
    sqlx::query_as::<_, AgentRow>(
        "SELECT a.agent_id, a.owner_account_id, a.name, a.description, a.runtime_agent, a.model, a.created_at_ms, a.updated_at_ms
           FROM agents a
           JOIN conversation_agent_members cam ON cam.agent_id = a.agent_id
          WHERE cam.conversation_id = ?
          ORDER BY cam.joined_at_ms ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_conversation_agents"))
}

pub async fn is_agent_in_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
    agent_id: &str,
) -> Result<bool, BackendError> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM conversation_agent_members WHERE conversation_id = ? AND agent_id = ?",
    )
    .bind(conversation_id)
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(store_err("social::is_agent_in_conversation"))?;
    Ok(row.is_some())
}

pub async fn add_member_to_group(
    pool: &SqlitePool,
    conversation_id: &str,
    account_id: &str,
    now_ms: i64,
) -> Result<(), BackendError> {
    sqlx::query(
        "INSERT OR IGNORE INTO conversation_members (conversation_id, account_id, joined_at_ms)
         VALUES (?, ?, ?)",
    )
    .bind(conversation_id)
    .bind(account_id)
    .bind(now_ms)
    .execute(pool)
    .await
    .map_err(store_err("social::add_member_to_group"))?;
    Ok(())
}

pub async fn insert_agent_message(
    pool: &SqlitePool,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
    now_ms: i64,
    reply_to_message_id: Option<&str>,
    mentioned_account_ids: &[String],
) -> Result<ChatMessageRow, BackendError> {
    let agent = get_agent(pool, agent_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::insert_agent_message.load_agent".into(),
            message: format!("agent not found: {agent_id}"),
        })?;
    let message_id = Uuid::new_v4().to_string();
    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("social::insert_agent_message.begin"))?;

    sqlx::query(
        "INSERT INTO chat_messages (
            message_id,
            conversation_id,
            sender_account_id,
            sender_agent_id,
            text,
            created_at_ms,
            reply_to_message_id,
            sender_type
         ) VALUES (?, ?, ?, ?, ?, ?, ?, 'agent')",
    )
    .bind(&message_id)
    .bind(conversation_id)
    .bind(&agent.owner_account_id)
    .bind(&agent.agent_id)
    .bind(text)
    .bind(now_ms)
    .bind(reply_to_message_id)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::insert_agent_message.insert"))?;

    for mentioned_id in mentioned_account_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO chat_message_mentions (message_id, mentioned_account_id)
             VALUES (?, ?)",
        )
        .bind(&message_id)
        .bind(mentioned_id)
        .execute(&mut *tx)
        .await
        .map_err(store_err("social::insert_agent_message.mention"))?;
    }

    sqlx::query("UPDATE conversations SET updated_at_ms = ? WHERE conversation_id = ?")
        .bind(now_ms)
        .bind(conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(store_err(
            "social::insert_agent_message.update_conversation",
        ))?;

    tx.commit()
        .await
        .map_err(store_err("social::insert_agent_message.commit"))?;

    get_message(pool, &message_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::insert_agent_message.load".into(),
            message: "message missing after insert".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, memory_pool, T0};

    async fn seed_group(pool: &SqlitePool) -> (String, String, String, String) {
        let alice = insert_account(pool, "alice@example.com").await;
        let bob = insert_account(pool, "bob@example.com").await;
        let carol = insert_account(pool, "carol@example.com").await;
        let conversation = create_group_conversation(
            pool,
            &alice,
            "Study Group",
            &[bob.clone(), carol.clone()],
            T0,
        )
        .await
        .unwrap();
        (conversation.conversation_id, alice, bob, carol)
    }

    #[tokio::test]
    async fn list_conversations_reports_unread_counts_and_mentions() {
        let pool = memory_pool().await;
        let (conversation_id, alice, bob, carol) = seed_group(&pool).await;

        insert_message(
            &pool,
            &conversation_id,
            &alice,
            "hello team",
            T0 + 1,
            None,
            &[],
        )
        .await
        .unwrap();
        let last_read = mark_conversation_read_to_latest(&pool, &conversation_id, &carol, T0 + 1)
            .await
            .unwrap();
        assert_eq!(last_read, Some(T0 + 1));

        insert_message(
            &pool,
            &conversation_id,
            &bob,
            "@carol please review",
            T0 + 2,
            None,
            std::slice::from_ref(&carol),
        )
        .await
        .unwrap();

        let carol_rows = list_conversations_for(&pool, &carol).await.unwrap();
        assert_eq!(carol_rows.len(), 1);
        assert_eq!(carol_rows[0].unread_count, 1);
        assert_eq!(carol_rows[0].unread_mention_count, 1);

        let alice_rows = list_conversations_for(&pool, &alice).await.unwrap();
        assert_eq!(alice_rows.len(), 1);
        assert_eq!(alice_rows[0].unread_count, 1);
        assert_eq!(alice_rows[0].unread_mention_count, 0);
    }

    #[tokio::test]
    async fn insert_message_persists_unique_mentions() {
        let pool = memory_pool().await;
        let (conversation_id, _alice, bob, carol) = seed_group(&pool).await;

        let message = insert_message(
            &pool,
            &conversation_id,
            &bob,
            "@carol @carol hello",
            T0 + 5,
            None,
            &[carol.clone(), carol.clone()],
        )
        .await
        .unwrap();

        let mentions = list_message_mentions(&pool, &[message.message_id])
            .await
            .unwrap();
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions.values().next().unwrap(), &vec![carol]);
    }

    #[tokio::test]
    async fn replied_message_tracks_parent_message() {
        let pool = memory_pool().await;
        let (conversation_id, alice, bob, _carol) = seed_group(&pool).await;

        let original = insert_message(
            &pool,
            &conversation_id,
            &alice,
            "hello team",
            T0 + 1,
            None,
            &[],
        )
        .await
        .unwrap();
        let reply = insert_message(
            &pool,
            &conversation_id,
            &bob,
            "收到",
            T0 + 2,
            Some(&original.message_id),
            &[],
        )
        .await
        .unwrap();

        let rows = list_messages(&pool, &conversation_id, None, 20)
            .await
            .unwrap();
        let reply_row = rows
            .iter()
            .find(|row| row.message_id == reply.message_id)
            .unwrap();
        assert_eq!(
            reply_row.reply_to_message_id.as_deref(),
            Some(original.message_id.as_str())
        );
    }

    #[tokio::test]
    async fn recalled_messages_stop_counting_as_unread() {
        let pool = memory_pool().await;
        let (conversation_id, alice, bob, carol) = seed_group(&pool).await;

        let last_read = mark_conversation_read_to_latest(&pool, &conversation_id, &carol, T0)
            .await
            .unwrap();
        assert_eq!(last_read, None);

        let message = insert_message(
            &pool,
            &conversation_id,
            &bob,
            "@carol please review",
            T0 + 2,
            None,
            std::slice::from_ref(&carol),
        )
        .await
        .unwrap();
        recall_message(&pool, &conversation_id, &message.message_id, &bob, T0 + 3)
            .await
            .unwrap();

        let carol_rows = list_conversations_for(&pool, &carol).await.unwrap();
        assert_eq!(carol_rows.len(), 1);
        assert_eq!(carol_rows[0].unread_count, 0);
        assert_eq!(carol_rows[0].unread_mention_count, 0);
        assert_eq!(
            carol_rows[0].last_message_preview.as_deref(),
            Some("[message recalled]")
        );

        let alice_rows = list_conversations_for(&pool, &alice).await.unwrap();
        assert_eq!(alice_rows.len(), 1);
        assert_eq!(
            alice_rows[0].last_message_preview.as_deref(),
            Some("[message recalled]")
        );
    }

    #[tokio::test]
    async fn bind_and_lookup_session_for_message_round_trip() {
        let pool = memory_pool().await;
        let (conversation_id, alice, _bob, _carol) = seed_group(&pool).await;

        let message = insert_message(&pool, &conversation_id, &alice, "ping", T0 + 10, None, &[])
            .await
            .unwrap();

        assert_eq!(
            lookup_session_id_for_message(&pool, &message.message_id)
                .await
                .unwrap(),
            None
        );

        bind_session_to_message(&pool, &message.message_id, "thr-social-1")
            .await
            .unwrap();

        assert_eq!(
            lookup_session_id_for_message(&pool, &message.message_id)
                .await
                .unwrap(),
            Some("thr-social-1".to_string())
        );

        assert_eq!(
            lookup_latest_session_id_for_conversation(&pool, &conversation_id)
                .await
                .unwrap(),
            Some("thr-social-1".to_string())
        );
        assert!(has_bound_message_for_session(&pool, "thr-social-1")
            .await
            .unwrap());
        assert!(suppress_live_ui_fanout_for_session(&pool, "thr-social-1")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn one_human_plus_agent_session_keeps_live_ui_fanout_enabled() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "Agent DM", &[], T0)
            .await
            .unwrap();
        let message = insert_message(
            &pool,
            &conversation.conversation_id,
            &alice,
            "hello",
            T0,
            None,
            &[],
        )
        .await
        .unwrap();

        bind_session_to_message(&pool, &message.message_id, "thr-direct-1")
            .await
            .unwrap();

        assert!(!suppress_live_ui_fanout_for_session(&pool, "thr-direct-1")
            .await
            .unwrap());
    }
}
