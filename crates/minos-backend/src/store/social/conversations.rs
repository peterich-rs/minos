use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::{normalized_pair, store_err, ConversationDigestRow, ConversationRow, ProfileRow};

pub async fn ensure_direct_conversation(
    store: &impl AsStorePool,
    creator_account_id: &str,
    left: &str,
    right: &str,
    now_ms: i64,
) -> Result<ConversationRow, BackendError> {
    let (low, high) = normalized_pair(left, right);
    if let Some(existing) = find_direct_conversation(store, low, high).await? {
        return Ok(existing);
    }

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::ensure_direct_conversation.begin"))?;
            let conversation_id = Uuid::new_v4().to_string();
            let insert_result = sqlx::query(
                "INSERT INTO conversations
                    (conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms)
                 VALUES ($1, 'direct', NULL, $2, $3, $4, $5, $6)",
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
                             VALUES ($1, $2, $3)",
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
    }
}

pub async fn create_group_conversation(
    store: &impl AsStorePool,
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

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
            for member in &members {
                let role = if member == creator_account_id {
                    "owner"
                } else {
                    "member"
                };
                sqlx::query(
                    "INSERT INTO conversation_members (conversation_id, account_id, joined_at_ms, role)
                     VALUES (?, ?, ?, ?)",
                )
                .bind(&conversation_id)
                .bind(member)
                .bind(now_ms)
                .bind(role)
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
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::create_group_conversation.begin"))?;
            let conversation_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO conversations
                    (conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms)
                 VALUES ($1, 'group', $2, $3, NULL, NULL, $4, $5)",
            )
            .bind(&conversation_id)
            .bind(title)
            .bind(creator_account_id)
            .bind(now_ms)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::create_group_conversation.insert_conversation"))?;
            for member in &members {
                let role = if member == creator_account_id {
                    "owner"
                } else {
                    "member"
                };
                sqlx::query(
                    "INSERT INTO conversation_members (conversation_id, account_id, joined_at_ms, role)
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(&conversation_id)
                .bind(member)
                .bind(now_ms)
                .bind(role)
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
    }
}

/// Ensure a group conversation exists with the given id (host-local projection).
///
/// When missing, creates it with `creator_account_id` and all `member_account_ids`.
/// Idempotent: concurrent inserts are treated as success if the row appears.
pub async fn ensure_group_conversation_with_id(
    store: &impl AsStorePool,
    conversation_id: &str,
    creator_account_id: &str,
    title: &str,
    member_account_ids: &[String],
    now_ms: i64,
) -> Result<ConversationRow, BackendError> {
    if let Some(existing) = get_conversation(store, conversation_id).await? {
        return Ok(existing);
    }

    let mut members = member_account_ids.to_vec();
    if !members.iter().any(|m| m == creator_account_id) {
        members.push(creator_account_id.to_string());
    }
    members.sort();
    members.dedup();

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::ensure_group_conversation_with_id.begin"))?;
            let insert = sqlx::query(
                "INSERT OR IGNORE INTO conversations
                    (conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms)
                 VALUES (?, 'group', ?, ?, NULL, NULL, ?, ?)",
            )
            .bind(conversation_id)
            .bind(title)
            .bind(creator_account_id)
            .bind(now_ms)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::ensure_group_conversation_with_id.insert_conversation"))?;
            if insert.rows_affected() > 0 {
                for member in &members {
                    sqlx::query(
                        "INSERT OR IGNORE INTO conversation_members (conversation_id, account_id, joined_at_ms)
                         VALUES (?, ?, ?)",
                    )
                    .bind(conversation_id)
                    .bind(member)
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::ensure_group_conversation_with_id.insert_member"))?;
                }
            }
            tx.commit().await.map_err(store_err(
                "social::ensure_group_conversation_with_id.commit",
            ))?;
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::ensure_group_conversation_with_id.begin"))?;
            let insert = sqlx::query(
                "INSERT INTO conversations
                    (conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms)
                 VALUES ($1, 'group', $2, $3, NULL, NULL, $4, $5)
                 ON CONFLICT (conversation_id) DO NOTHING",
            )
            .bind(conversation_id)
            .bind(title)
            .bind(creator_account_id)
            .bind(now_ms)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::ensure_group_conversation_with_id.insert_conversation"))?;
            if insert.rows_affected() > 0 {
                for member in &members {
                    sqlx::query(
                        "INSERT INTO conversation_members (conversation_id, account_id, joined_at_ms)
                         VALUES ($1, $2, $3)
                         ON CONFLICT DO NOTHING",
                    )
                    .bind(conversation_id)
                    .bind(member)
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::ensure_group_conversation_with_id.insert_member"))?;
                }
            }
            tx.commit().await.map_err(store_err(
                "social::ensure_group_conversation_with_id.commit",
            ))?;
        }
    }

    get_conversation(store, conversation_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::ensure_group_conversation_with_id.load".into(),
            message: "conversation missing after ensure".into(),
        })
}

/// Create or update a group conversation with a client-owned id (Desktop → Hub).
///
/// - Missing: create with title + members (includes creator).
/// - Existing: update title, ensure membership for creator + `member_account_ids`.
///
/// Does not delete messages or remove existing members.
pub async fn upsert_group_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
    creator_account_id: &str,
    title: &str,
    member_account_ids: &[String],
    now_ms: i64,
) -> Result<ConversationRow, BackendError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(BackendError::StoreQuery {
            operation: "social::upsert_group_conversation.title".into(),
            message: "group title is required".into(),
        });
    }
    // Client placeholders must not clobber a real title already on the Hub row.
    let is_placeholder_title = title.eq_ignore_ascii_case("conversation")
        || title.eq_ignore_ascii_case("group chat")
        || title.eq_ignore_ascii_case("direct agent sessions");

    let mut members = member_account_ids.to_vec();
    if !members.iter().any(|m| m == creator_account_id) {
        members.push(creator_account_id.to_string());
    }
    members.sort();
    members.dedup();

    if get_conversation(store, conversation_id).await?.is_some() {
        match store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(store_err("social::upsert_group_conversation.begin"))?;
                // Skip placeholder titles so host_projection / empty client titles
                // cannot wipe a real name already stored on the Hub row.
                if is_placeholder_title {
                    sqlx::query(
                        "UPDATE conversations
                            SET updated_at_ms = ?
                          WHERE conversation_id = ?",
                    )
                    .bind(now_ms)
                    .bind(conversation_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::upsert_group_conversation.touch"))?;
                } else {
                    sqlx::query(
                        "UPDATE conversations
                            SET title = ?, updated_at_ms = ?
                          WHERE conversation_id = ?",
                    )
                    .bind(title)
                    .bind(now_ms)
                    .bind(conversation_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::upsert_group_conversation.update_title"))?;
                }
                for member in &members {
                    sqlx::query(
                        "INSERT OR IGNORE INTO conversation_members (conversation_id, account_id, joined_at_ms)
                         VALUES (?, ?, ?)",
                    )
                    .bind(conversation_id)
                    .bind(member)
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::upsert_group_conversation.insert_member"))?;
                }
                tx.commit()
                    .await
                    .map_err(store_err("social::upsert_group_conversation.commit"))?;
            }
            StorePoolRef::Postgres(pool) => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(store_err("social::upsert_group_conversation.begin"))?;
                if is_placeholder_title {
                    sqlx::query(
                        "UPDATE conversations
                            SET updated_at_ms = $1
                          WHERE conversation_id = $2",
                    )
                    .bind(now_ms)
                    .bind(conversation_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::upsert_group_conversation.touch"))?;
                } else {
                    sqlx::query(
                        "UPDATE conversations
                            SET title = $1, updated_at_ms = $2
                          WHERE conversation_id = $3",
                    )
                    .bind(title)
                    .bind(now_ms)
                    .bind(conversation_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::upsert_group_conversation.update_title"))?;
                }
                for member in &members {
                    sqlx::query(
                        "INSERT INTO conversation_members (conversation_id, account_id, joined_at_ms)
                         VALUES ($1, $2, $3)
                         ON CONFLICT DO NOTHING",
                    )
                    .bind(conversation_id)
                    .bind(member)
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_err("social::upsert_group_conversation.insert_member"))?;
                }
                tx.commit()
                    .await
                    .map_err(store_err("social::upsert_group_conversation.commit"))?;
            }
        }
    } else {
        ensure_group_conversation_with_id(
            store,
            conversation_id,
            creator_account_id,
            title,
            &members,
            now_ms,
        )
        .await?;
    }

    get_conversation(store, conversation_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "social::upsert_group_conversation.load".into(),
            message: "conversation missing after upsert".into(),
        })
}

pub async fn get_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
) -> Result<Option<ConversationRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ConversationRow>(
                "SELECT conversation_id, kind, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms
                   FROM conversations
                  WHERE conversation_id = ?",
            )
            .bind(conversation_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ConversationRow>(
                "SELECT conversation_id, kind::text, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms
                   FROM conversations
                  WHERE conversation_id = $1",
            )
            .bind(conversation_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::get_conversation"))
}

pub async fn list_conversations_for(
    store: &impl AsStorePool,
    account_id: &str,
) -> Result<Vec<ConversationDigestRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
                    (SELECT m.text FROM chat_messages m WHERE m.conversation_id = c.conversation_id ORDER BY m.message_seq DESC LIMIT 1) AS last_message_preview,
                    COALESCE((SELECT MAX(m.created_at_ms) FROM chat_messages m WHERE m.conversation_id = c.conversation_id), c.updated_at_ms) AS last_message_at_ms,
                    COALESCE((
                        SELECT COUNT(*)
                          FROM chat_messages m
                         WHERE m.conversation_id = c.conversation_id
                           AND (m.sender_account_id IS NULL OR m.sender_account_id <> ?)
                           AND m.recalled_at_ms IS NULL
                           AND m.message_seq > COALESCE(cr.last_read_seq, 0)
                           AND m.created_at_ms > COALESCE(cd.deleted_at_ms, 0)
                    ), 0) AS unread_count,
                    COALESCE((
                        SELECT COUNT(*)
                          FROM chat_messages m
                          JOIN chat_message_mentions mm ON mm.message_id = m.message_id
                         WHERE m.conversation_id = c.conversation_id
                           AND (m.sender_account_id IS NULL OR m.sender_account_id <> ?)
                           AND m.recalled_at_ms IS NULL
                           AND m.message_seq > COALESCE(cr.last_read_seq, 0)
                           AND m.created_at_ms > COALESCE(cd.deleted_at_ms, 0)
                           AND mm.target_kind = 'account'
                           AND mm.target_id = ?
                    ), 0) AS unread_mention_count
                  FROM conversations c
                  JOIN conversation_members cm ON cm.conversation_id = c.conversation_id
             LEFT JOIN conversation_reads cr
                    ON cr.conversation_id = c.conversation_id
                   AND cr.account_id = ?
             LEFT JOIN conversation_deletions cd
                    ON cd.conversation_id = c.conversation_id
                   AND cd.account_id = ?
                 WHERE cm.account_id = ?
                   AND COALESCE((SELECT MAX(m.created_at_ms) FROM chat_messages m WHERE m.conversation_id = c.conversation_id), c.updated_at_ms) > COALESCE(cd.deleted_at_ms, 0)
              ORDER BY last_message_at_ms DESC",
            )
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ConversationDigestRow>(
                "SELECT
                    c.conversation_id,
                    c.kind::text,
                    c.title,
                    c.created_by_account_id,
                    c.direct_account_low,
                    c.direct_account_high,
                    c.created_at_ms,
                    c.updated_at_ms,
                    (SELECT COUNT(*) FROM conversation_members cm2 WHERE cm2.conversation_id = c.conversation_id) AS member_count,
                    (SELECT m.text FROM chat_messages m WHERE m.conversation_id = c.conversation_id ORDER BY m.message_seq DESC LIMIT 1) AS last_message_preview,
                    COALESCE((SELECT MAX(m.created_at_ms) FROM chat_messages m WHERE m.conversation_id = c.conversation_id), c.updated_at_ms) AS last_message_at_ms,
                    COALESCE((
                        SELECT COUNT(*)
                          FROM chat_messages m
                         WHERE m.conversation_id = c.conversation_id
                           AND (m.sender_account_id IS NULL OR m.sender_account_id <> $1)
                           AND m.recalled_at_ms IS NULL
                           AND m.message_seq > COALESCE(cr.last_read_seq, 0)
                           AND m.created_at_ms > COALESCE(cd.deleted_at_ms, 0)
                    ), 0) AS unread_count,
                    COALESCE((
                        SELECT COUNT(*)
                          FROM chat_messages m
                          JOIN chat_message_mentions mm ON mm.message_id = m.message_id
                         WHERE m.conversation_id = c.conversation_id
                           AND (m.sender_account_id IS NULL OR m.sender_account_id <> $2)
                           AND m.recalled_at_ms IS NULL
                           AND m.message_seq > COALESCE(cr.last_read_seq, 0)
                           AND m.created_at_ms > COALESCE(cd.deleted_at_ms, 0)
                           AND mm.target_kind = 'account'
                           AND mm.target_id = $3
                    ), 0) AS unread_mention_count
                  FROM conversations c
                  JOIN conversation_members cm ON cm.conversation_id = c.conversation_id
             LEFT JOIN conversation_reads cr
                    ON cr.conversation_id = c.conversation_id
                   AND cr.account_id = $4
             LEFT JOIN conversation_deletions cd
                    ON cd.conversation_id = c.conversation_id
                   AND cd.account_id = $5
                 WHERE cm.account_id = $6
                   AND COALESCE((SELECT MAX(m.created_at_ms) FROM chat_messages m WHERE m.conversation_id = c.conversation_id), c.updated_at_ms) > COALESCE(cd.deleted_at_ms, 0)
              ORDER BY last_message_at_ms DESC",
            )
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .bind(account_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_conversations_for"))
}

pub async fn list_conversation_member_profiles(
    store: &impl AsStorePool,
    conversation_id: &str,
) -> Result<Vec<ProfileRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ProfileRow>(
                "SELECT a.account_id, a.email, a.minos_id, a.display_name
                     FROM accounts a
                     JOIN conversation_members cm ON cm.account_id = a.account_id
                    WHERE cm.conversation_id = $1
                    ORDER BY cm.joined_at_ms ASC, a.account_id ASC",
            )
            .bind(conversation_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_conversation_member_profiles"))
}

pub async fn list_conversation_members(
    store: &impl AsStorePool,
    conversation_id: &str,
) -> Result<Vec<String>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, String>(
                "SELECT account_id
                   FROM conversation_members
                  WHERE conversation_id = ?
                  ORDER BY joined_at_ms ASC",
            )
            .bind(conversation_id)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, String>(
                "SELECT account_id
                   FROM conversation_members
                  WHERE conversation_id = $1
                  ORDER BY joined_at_ms ASC",
            )
            .bind(conversation_id)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_conversation_members"))
}

pub async fn is_conversation_member(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
) -> Result<bool, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM conversation_members
                  WHERE conversation_id = ? AND account_id = ?",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM conversation_members
                  WHERE conversation_id = $1 AND account_id = $2",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("social::is_conversation_member"))?;

    Ok(row > 0)
}

/// Member role in a conversation (`owner` | `admin` | `member`).
pub async fn get_member_role(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
) -> Result<Option<String>, BackendError> {
    let role = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, String>(
                "SELECT role FROM conversation_members
                  WHERE conversation_id = ? AND account_id = ?",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, String>(
                "SELECT role FROM conversation_members
                  WHERE conversation_id = $1 AND account_id = $2",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::get_member_role"))?;
    Ok(role)
}

/// Result of a membership mutation that must be published + may revoke live subs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipChangeResult {
    pub membership_version: i64,
    pub action: String,
    pub member_account_id: String,
    pub role: Option<String>,
    /// True when a row was inserted/deleted (false = no-op idempotent).
    pub changed: bool,
}

pub async fn add_member_to_group(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
    now_ms: i64,
) -> Result<MembershipChangeResult, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::add_member_to_group.begin"))?;
            let result = sqlx::query(
                "INSERT OR IGNORE INTO conversation_members
                    (conversation_id, account_id, joined_at_ms, role)
                 VALUES (?, ?, ?, 'member')",
            )
            .bind(conversation_id)
            .bind(account_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::add_member_to_group.insert"))?;
            let changed = result.rows_affected() > 0;
            let version = if changed {
                bump_membership_version_sqlite(&mut tx, conversation_id, now_ms).await?
            } else {
                read_membership_version_sqlite(&mut tx, conversation_id).await?
            };
            tx.commit()
                .await
                .map_err(store_err("social::add_member_to_group.commit"))?;
            Ok(MembershipChangeResult {
                membership_version: version,
                action: "added".into(),
                member_account_id: account_id.to_string(),
                role: Some("member".into()),
                changed,
            })
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::add_member_to_group.begin"))?;
            let result = sqlx::query(
                "INSERT INTO conversation_members
                    (conversation_id, account_id, joined_at_ms, role)
                 VALUES ($1, $2, $3, 'member')
                 ON CONFLICT DO NOTHING",
            )
            .bind(conversation_id)
            .bind(account_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::add_member_to_group.insert"))?;
            let changed = result.rows_affected() > 0;
            let version = if changed {
                bump_membership_version_postgres(&mut tx, conversation_id, now_ms).await?
            } else {
                read_membership_version_postgres(&mut tx, conversation_id).await?
            };
            tx.commit()
                .await
                .map_err(store_err("social::add_member_to_group.commit"))?;
            Ok(MembershipChangeResult {
                membership_version: version,
                action: "added".into(),
                member_account_id: account_id.to_string(),
                role: Some("member".into()),
                changed,
            })
        }
    }
}

pub async fn remove_member_from_group(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
    now_ms: i64,
) -> Result<MembershipChangeResult, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::remove_member_from_group.begin"))?;
            let result = sqlx::query(
                "DELETE FROM conversation_members
                  WHERE conversation_id = ? AND account_id = ?",
            )
            .bind(conversation_id)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::remove_member_from_group.delete"))?;
            let changed = result.rows_affected() > 0;
            let version = if changed {
                bump_membership_version_sqlite(&mut tx, conversation_id, now_ms).await?
            } else {
                read_membership_version_sqlite(&mut tx, conversation_id).await?
            };
            tx.commit()
                .await
                .map_err(store_err("social::remove_member_from_group.commit"))?;
            Ok(MembershipChangeResult {
                membership_version: version,
                action: "removed".into(),
                member_account_id: account_id.to_string(),
                role: None,
                changed,
            })
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::remove_member_from_group.begin"))?;
            let result = sqlx::query(
                "DELETE FROM conversation_members
                  WHERE conversation_id = $1 AND account_id = $2",
            )
            .bind(conversation_id)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::remove_member_from_group.delete"))?;
            let changed = result.rows_affected() > 0;
            let version = if changed {
                bump_membership_version_postgres(&mut tx, conversation_id, now_ms).await?
            } else {
                read_membership_version_postgres(&mut tx, conversation_id).await?
            };
            tx.commit()
                .await
                .map_err(store_err("social::remove_member_from_group.commit"))?;
            Ok(MembershipChangeResult {
                membership_version: version,
                action: "removed".into(),
                member_account_id: account_id.to_string(),
                role: None,
                changed,
            })
        }
    }
}

async fn bump_membership_version_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    conversation_id: &str,
    now_ms: i64,
) -> Result<i64, BackendError> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE conversations
            SET membership_version = membership_version + 1,
                updated_at_ms = ?
          WHERE conversation_id = ?
      RETURNING membership_version",
    )
    .bind(now_ms)
    .bind(conversation_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(store_err("social::bump_membership_version"))
}

async fn bump_membership_version_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    conversation_id: &str,
    now_ms: i64,
) -> Result<i64, BackendError> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE conversations
            SET membership_version = membership_version + 1,
                updated_at_ms = $1
          WHERE conversation_id = $2
      RETURNING membership_version",
    )
    .bind(now_ms)
    .bind(conversation_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(store_err("social::bump_membership_version"))
}

async fn read_membership_version_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    conversation_id: &str,
) -> Result<i64, BackendError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT membership_version FROM conversations WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(store_err("social::read_membership_version"))
}

async fn read_membership_version_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    conversation_id: &str,
) -> Result<i64, BackendError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT membership_version FROM conversations WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(store_err("social::read_membership_version"))
}

pub async fn mark_conversation_deleted_for_account(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
    deleted_at_ms: i64,
) -> Result<bool, BackendError> {
    let result = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO conversation_deletions
                    (conversation_id, account_id, deleted_at_ms)
                 VALUES (?, ?, ?)
                 ON CONFLICT(conversation_id, account_id) DO UPDATE SET
                    deleted_at_ms = excluded.deleted_at_ms",
        )
        .bind(conversation_id)
        .bind(account_id)
        .bind(deleted_at_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO conversation_deletions
                    (conversation_id, account_id, deleted_at_ms)
                 VALUES ($1, $2, $3)
                 ON CONFLICT(conversation_id, account_id) DO UPDATE SET
                    deleted_at_ms = EXCLUDED.deleted_at_ms",
        )
        .bind(conversation_id)
        .bind(account_id)
        .bind(deleted_at_ms)
        .execute(pool)
        .await
        .map(|result| result.rows_affected()),
    }
    .map_err(store_err("social::mark_conversation_deleted_for_account"))?;

    Ok(result > 0)
}

pub async fn conversation_deleted_at_for_account(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
) -> Result<Option<i64>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT deleted_at_ms
               FROM conversation_deletions
              WHERE conversation_id = ? AND account_id = ?",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT deleted_at_ms
               FROM conversation_deletions
              WHERE conversation_id = $1 AND account_id = $2",
            )
            .bind(conversation_id)
            .bind(account_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::conversation_deleted_at_for_account"))
}

/// Mark conversation read through a client-observed `message_seq` watermark.
///
/// Server clamps to the latest existing seq and applies monotonic MAX on the
/// stored watermark so multi-device merge never rewinds.
/// Returns `(last_read_seq, last_read_at_ms)` after the update, or `None` when
/// there are no messages / observed seq is non-positive.
pub async fn mark_conversation_read_to_seq(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
    read_up_to_message_seq: i64,
    updated_at_ms: i64,
) -> Result<Option<(i64, i64)>, BackendError> {
    if read_up_to_message_seq <= 0 {
        return Ok(None);
    }

    #[derive(sqlx::FromRow)]
    struct LatestRead {
        last_read_seq: i64,
        last_read_at_ms: i64,
    }

    // Clamp observed watermark to messages that actually exist (never invent seqs).
    let latest = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query_as::<_, LatestRead>(
            "SELECT COALESCE(MAX(message_seq), 0) AS last_read_seq,
                    COALESCE(MAX(created_at_ms), 0) AS last_read_at_ms
               FROM chat_messages
              WHERE conversation_id = ?
                AND message_seq <= ?",
        )
        .bind(conversation_id)
        .bind(read_up_to_message_seq)
        .fetch_one(pool)
        .await
        .map_err(store_err(
            "social::mark_conversation_read_to_seq.fetch_observed",
        ))?,
        StorePoolRef::Postgres(pool) => sqlx::query_as::<_, LatestRead>(
            "SELECT COALESCE(MAX(message_seq), 0) AS last_read_seq,
                    COALESCE(MAX(created_at_ms), 0) AS last_read_at_ms
               FROM chat_messages
              WHERE conversation_id = $1
                AND message_seq <= $2",
        )
        .bind(conversation_id)
        .bind(read_up_to_message_seq)
        .fetch_one(pool)
        .await
        .map_err(store_err(
            "social::mark_conversation_read_to_seq.fetch_observed",
        ))?,
    };
    if latest.last_read_seq <= 0 {
        return Ok(None);
    }

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "INSERT INTO conversation_reads
                    (conversation_id, account_id, last_read_seq, last_read_at_ms, updated_at_ms)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(conversation_id, account_id) DO UPDATE SET
                    last_read_seq = CASE
                        WHEN excluded.last_read_seq > conversation_reads.last_read_seq
                            THEN excluded.last_read_seq
                        ELSE conversation_reads.last_read_seq
                    END,
                    last_read_at_ms = CASE
                        WHEN excluded.last_read_seq > conversation_reads.last_read_seq
                            THEN excluded.last_read_at_ms
                        ELSE conversation_reads.last_read_at_ms
                    END,
                    updated_at_ms = excluded.updated_at_ms",
        )
        .bind(conversation_id)
        .bind(account_id)
        .bind(latest.last_read_seq)
        .bind(latest.last_read_at_ms)
        .bind(updated_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "INSERT INTO conversation_reads
                    (conversation_id, account_id, last_read_seq, last_read_at_ms, updated_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(conversation_id, account_id) DO UPDATE SET
                    last_read_seq = CASE
                        WHEN EXCLUDED.last_read_seq > conversation_reads.last_read_seq
                            THEN EXCLUDED.last_read_seq
                        ELSE conversation_reads.last_read_seq
                    END,
                    last_read_at_ms = CASE
                        WHEN EXCLUDED.last_read_seq > conversation_reads.last_read_seq
                            THEN EXCLUDED.last_read_at_ms
                        ELSE conversation_reads.last_read_at_ms
                    END,
                    updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(conversation_id)
        .bind(account_id)
        .bind(latest.last_read_seq)
        .bind(latest.last_read_at_ms)
        .bind(updated_at_ms)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("social::mark_conversation_read_to_seq.upsert"))?;

    Ok(Some((latest.last_read_seq, latest.last_read_at_ms)))
}

/// Test/admin helper: mark read through the absolute latest message seq.
pub async fn mark_conversation_read_to_latest(
    store: &impl AsStorePool,
    conversation_id: &str,
    account_id: &str,
    updated_at_ms: i64,
) -> Result<Option<(i64, i64)>, BackendError> {
    let max_seq = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(message_seq), 0) FROM chat_messages WHERE conversation_id = ?",
            )
            .bind(conversation_id)
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(message_seq), 0) FROM chat_messages WHERE conversation_id = $1",
        )
        .bind(conversation_id)
        .fetch_one(pool)
        .await,
    }
    .map_err(store_err("social::mark_conversation_read_to_latest.max"))?;
    mark_conversation_read_to_seq(store, conversation_id, account_id, max_seq, updated_at_ms).await
}

pub(crate) async fn find_direct_conversation(
    store: &impl AsStorePool,
    low: &str,
    high: &str,
) -> Result<Option<ConversationRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ConversationRow>(
                "SELECT conversation_id, kind::text, title, created_by_account_id, direct_account_low, direct_account_high, created_at_ms, updated_at_ms
                   FROM conversations
                  WHERE kind = 'direct'
                    AND direct_account_low = $1
                    AND direct_account_high = $2",
            )
            .bind(low)
            .bind(high)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::find_direct_conversation"))
}
