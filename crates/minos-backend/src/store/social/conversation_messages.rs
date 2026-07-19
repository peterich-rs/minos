use std::collections::HashMap;

use sqlx::{Postgres, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::{store_err, ChatMessageRow, MessageMentionRow};

pub async fn get_message(
    store: &impl AsStorePool,
    message_id: &str,
) -> Result<Option<ChatMessageRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, reply_to_message_id, recalled_at_ms, sender_type
                   FROM chat_messages
                  WHERE message_id = ?",
            )
            .bind(message_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, reply_to_message_id, recalled_at_ms, sender_type
                   FROM chat_messages
                  WHERE message_id = $1",
            )
            .bind(message_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(store_err("social::get_message"))
}

pub async fn list_messages(
    store: &impl AsStorePool,
    conversation_id: &str,
    before_ts_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<ChatMessageRow>, BackendError> {
    let effective_limit = i64::from(limit.min(200));
    let before = before_ts_ms.unwrap_or(i64::MAX);
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, reply_to_message_id, recalled_at_ms, sender_type
                   FROM chat_messages
                  WHERE conversation_id = $1 AND created_at_ms < $2
                  ORDER BY created_at_ms DESC
                  LIMIT $3",
            )
            .bind(conversation_id)
            .bind(before)
            .bind(effective_limit)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_messages"))
}

pub async fn list_messages_by_ids(
    store: &impl AsStorePool,
    message_ids: &[String],
) -> Result<Vec<ChatMessageRow>, BackendError> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
            builder.build_query_as::<ChatMessageRow>().fetch_all(pool).await
        }
        StorePoolRef::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, reply_to_message_id, recalled_at_ms, sender_type\n           FROM chat_messages\n          WHERE message_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for message_id in message_ids {
                    separated.push_bind(message_id);
                }
            }
            builder.push(')');
            builder.build_query_as::<ChatMessageRow>().fetch_all(pool).await
        }
    }
    .map_err(store_err("social::list_messages_by_ids"))
}

pub async fn list_message_mentions(
    store: &impl AsStorePool,
    message_ids: &[String],
) -> Result<HashMap<String, Vec<String>>, BackendError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
            builder
                .build_query_as::<MessageMentionRow>()
                .fetch_all(pool)
                .await
        }
        StorePoolRef::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(
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
            builder
                .build_query_as::<MessageMentionRow>()
                .fetch_all(pool)
                .await
        }
    }
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

pub async fn insert_message(
    store: &impl AsStorePool,
    conversation_id: &str,
    sender_account_id: &str,
    text: &str,
    created_at_ms: i64,
    reply_to_message_id: Option<&str>,
    mentioned_account_ids: &[String],
) -> Result<ChatMessageRow, BackendError> {
    let message_id = Uuid::new_v4().to_string();
    let mut unique_mentions = mentioned_account_ids.to_vec();
    unique_mentions.sort();
    unique_mentions.dedup();

    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::insert_message.begin"))?;
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
            for mentioned_account_id in &unique_mentions {
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
        }
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::insert_message.begin"))?;
            sqlx::query(
                "INSERT INTO chat_messages
                    (message_id, conversation_id, sender_account_id, text, created_at_ms, reply_to_message_id)
                 VALUES ($1, $2, $3, $4, $5, $6)",
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
            for mentioned_account_id in &unique_mentions {
                sqlx::query(
                    "INSERT INTO chat_message_mentions
                        (message_id, mentioned_account_id)
                     VALUES ($1, $2)",
                )
                .bind(&message_id)
                .bind(mentioned_account_id)
                .execute(&mut *tx)
                .await
                .map_err(store_err("social::insert_message.insert_mention"))?;
            }
            sqlx::query(
                "UPDATE conversations
                    SET updated_at_ms = $1
                  WHERE conversation_id = $2",
            )
            .bind(created_at_ms)
            .bind(conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::insert_message.touch_conversation"))?;
            tx.commit()
                .await
                .map_err(store_err("social::insert_message.commit"))?;
        }
    }

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
    store: &impl AsStorePool,
    message_id: &str,
    session_id: &str,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE chat_messages
                    SET agent_session_id = ?
                  WHERE message_id = ?",
        )
        .bind(session_id)
        .bind(message_id)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE chat_messages
                    SET agent_session_id = $1
                  WHERE message_id = $2",
        )
        .bind(session_id)
        .bind(message_id)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("social::bind_session_to_message"))?;

    Ok(())
}

pub async fn bind_session_to_message_for_agent(
    store: &impl AsStorePool,
    message_id: &str,
    agent_id: &str,
    session_id: &str,
) -> Result<(), BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => sqlx::query(
            "UPDATE chat_messages
                    SET agent_session_id = ?,
                        sender_agent_id = COALESCE(sender_agent_id, ?)
                  WHERE message_id = ?",
        )
        .bind(session_id)
        .bind(agent_id)
        .bind(message_id)
        .execute(pool)
        .await
        .map(|_| ()),
        StorePoolRef::Postgres(pool) => sqlx::query(
            "UPDATE chat_messages
                    SET agent_session_id = $1,
                        sender_agent_id = COALESCE(sender_agent_id, $2)
                  WHERE message_id = $3",
        )
        .bind(session_id)
        .bind(agent_id)
        .bind(message_id)
        .execute(pool)
        .await
        .map(|_| ()),
    }
    .map_err(store_err("social::bind_session_to_message_for_agent"))?;

    Ok(())
}

pub async fn lookup_session_id_for_message(
    store: &impl AsStorePool,
    message_id: &str,
) -> Result<Option<String>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT agent_session_id
                   FROM chat_messages
                  WHERE message_id = ?",
            )
            .bind(message_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT agent_session_id
                   FROM chat_messages
                  WHERE message_id = $1",
            )
            .bind(message_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map(Option::flatten)
    .map_err(store_err("social::lookup_session_id_for_message"))
}

pub async fn lookup_latest_session_id_for_conversation(
    store: &impl AsStorePool,
    conversation_id: &str,
) -> Result<Option<String>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT agent_session_id
                   FROM chat_messages
                  WHERE conversation_id = $1
                    AND agent_session_id IS NOT NULL
                  ORDER BY created_at_ms DESC
                  LIMIT 1",
            )
            .bind(conversation_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map(Option::flatten)
    .map_err(store_err(
        "social::lookup_latest_session_id_for_conversation",
    ))
}

pub async fn lookup_latest_session_id_for_conversation_agent(
    store: &impl AsStorePool,
    conversation_id: &str,
    agent_id: &str,
) -> Result<Option<String>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT agent_session_id
                   FROM chat_messages
                  WHERE conversation_id = ?
                    AND sender_agent_id = ?
                    AND agent_session_id IS NOT NULL
                  ORDER BY created_at_ms DESC
                  LIMIT 1",
            )
            .bind(conversation_id)
            .bind(agent_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT agent_session_id
                   FROM chat_messages
                  WHERE conversation_id = $1
                    AND sender_agent_id = $2
                    AND agent_session_id IS NOT NULL
                  ORDER BY created_at_ms DESC
                  LIMIT 1",
            )
            .bind(conversation_id)
            .bind(agent_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map(Option::flatten)
    .map_err(store_err(
        "social::lookup_latest_session_id_for_conversation_agent",
    ))
}

pub async fn has_bound_message_for_session(
    store: &impl AsStorePool,
    session_id: &str,
) -> Result<bool, BackendError> {
    let row = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM chat_messages
                  WHERE agent_session_id = ?",
            )
            .bind(session_id)
            .fetch_one(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                   FROM chat_messages
                  WHERE agent_session_id = $1",
            )
            .bind(session_id)
            .fetch_one(pool)
            .await
        }
    }
    .map_err(store_err("social::has_bound_message_for_session"))?;

    Ok(row > 0)
}

pub async fn suppress_live_ui_fanout_for_session(
    store: &impl AsStorePool,
    session_id: &str,
) -> Result<bool, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
        StorePoolRef::Postgres(pool) => {
            let row = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(
                     SELECT 1
                       FROM chat_messages m
                       JOIN conversation_members cm ON cm.conversation_id = m.conversation_id
                      WHERE m.agent_session_id = $1
                      GROUP BY m.conversation_id
                     HAVING COUNT(DISTINCT cm.account_id) > 1
                 )",
            )
            .bind(session_id)
            .fetch_one(pool)
            .await
            .map_err(store_err("social::suppress_live_ui_fanout_for_session"))?;
            Ok(row)
        }
    }
}

pub async fn recall_message(
    store: &impl AsStorePool,
    conversation_id: &str,
    message_id: &str,
    sender_account_id: &str,
    recalled_at_ms: i64,
) -> Result<Option<ChatMessageRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
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
        StorePoolRef::Postgres(pool) => {
            let mut tx = pool
                .begin()
                .await
                .map_err(store_err("social::recall_message.begin"))?;
            sqlx::query(
                "UPDATE chat_messages
                    SET text = '[message recalled]',
                        recalled_at_ms = COALESCE(recalled_at_ms, $1)
                  WHERE message_id = $2
                    AND conversation_id = $3
                    AND sender_account_id = $4",
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
                  WHERE message_id = $1",
            )
            .bind(message_id)
            .execute(&mut *tx)
            .await
            .map_err(store_err("social::recall_message.delete_mentions"))?;

            sqlx::query(
                "UPDATE conversations
                    SET updated_at_ms = GREATEST(updated_at_ms, $1)
                  WHERE conversation_id = $2",
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
    }
}
