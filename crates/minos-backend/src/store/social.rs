use minos_protocol::FriendRequestStatus;
use sqlx::{FromRow, SqlitePool};
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
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ChatMessageRow {
    pub message_id: String,
    pub conversation_id: String,
    pub sender_account_id: String,
    pub text: String,
    pub created_at_ms: i64,
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
    sqlx::query(
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
    .await
    .map_err(store_err("social::ensure_direct_conversation.insert_conversation"))?;
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
            COALESCE((SELECT MAX(m.created_at_ms) FROM chat_messages m WHERE m.conversation_id = c.conversation_id), c.updated_at_ms) AS last_message_at_ms
          FROM conversations c
          JOIN conversation_members cm ON cm.conversation_id = c.conversation_id
         WHERE cm.account_id = ?
         ORDER BY last_message_at_ms DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(store_err("social::list_conversations_for"))
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

pub async fn list_messages(
    pool: &SqlitePool,
    conversation_id: &str,
    before_ts_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<ChatMessageRow>, BackendError> {
    let effective_limit = i64::from(limit.min(200));
    let before = before_ts_ms.unwrap_or(i64::MAX);
    sqlx::query_as::<_, ChatMessageRow>(
        "SELECT message_id, conversation_id, sender_account_id, text, created_at_ms
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

pub async fn insert_message(
    pool: &SqlitePool,
    conversation_id: &str,
    sender_account_id: &str,
    text: &str,
    created_at_ms: i64,
) -> Result<ChatMessageRow, BackendError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(store_err("social::insert_message.begin"))?;
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO chat_messages
            (message_id, conversation_id, sender_account_id, text, created_at_ms)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(conversation_id)
    .bind(sender_account_id)
    .bind(text)
    .bind(created_at_ms)
    .execute(&mut *tx)
    .await
    .map_err(store_err("social::insert_message.insert"))?;
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
        text: text.to_string(),
        created_at_ms,
    })
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
