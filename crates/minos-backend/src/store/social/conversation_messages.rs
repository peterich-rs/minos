use std::collections::HashMap;

use sqlx::{Postgres, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::app::tx::DbTx;
use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::{store_err, ChatMessageRow, MessageMentionRow};

/// Outcome of an insert that may short-circuit on `client_message_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertMessageOutcome {
    pub row: ChatMessageRow,
    /// `false` when an existing row was returned for the same client id.
    pub inserted: bool,
}

pub async fn get_message(
    store: &impl AsStorePool,
    message_id: &str,
) -> Result<Option<ChatMessageRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
                   FROM chat_messages
                  WHERE message_id = ?",
            )
            .bind(message_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
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

pub async fn get_message_in_tx(
    tx: &mut DbTx<'_>,
    message_id: &str,
) -> Result<Option<ChatMessageRow>, BackendError> {
    match tx {
        DbTx::Sqlite(tx) => sqlx::query_as::<_, ChatMessageRow>(
            "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
               FROM chat_messages
              WHERE message_id = ?",
        )
        .bind(message_id)
        .fetch_optional(&mut **tx)
        .await,
        DbTx::Postgres(tx) => sqlx::query_as::<_, ChatMessageRow>(
            "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
               FROM chat_messages
              WHERE message_id = $1",
        )
        .bind(message_id)
        .fetch_optional(&mut **tx)
        .await,
    }
    .map_err(store_err("social::get_message_in_tx"))
}

/// List messages with keyset pagination on `message_seq`.
///
/// - `before_seq`: older pages (`message_seq < before_seq`), DESC
/// - `after_seq`: incremental (`message_seq > after_seq`), ASC then reversed to DESC
/// - both None: latest `limit` messages DESC
pub async fn list_messages(
    store: &impl AsStorePool,
    conversation_id: &str,
    before_seq: Option<i64>,
    after_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<ChatMessageRow>, BackendError> {
    let effective_limit = i64::from(limit.min(200));
    match (before_seq, after_seq) {
        (Some(before), _) => {
            list_messages_before(store, conversation_id, before, effective_limit).await
        }
        (None, Some(after)) => {
            let mut rows =
                list_messages_after(store, conversation_id, after, effective_limit).await?;
            // Normalize to DESC (newest first) like other list paths.
            rows.reverse();
            Ok(rows)
        }
        (None, None) => {
            list_messages_before(store, conversation_id, i64::MAX, effective_limit).await
        }
    }
}

async fn list_messages_before(
    store: &impl AsStorePool,
    conversation_id: &str,
    before_seq: i64,
    limit: i64,
) -> Result<Vec<ChatMessageRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
                   FROM chat_messages
                  WHERE conversation_id = ? AND message_seq < ?
                  ORDER BY message_seq DESC
                  LIMIT ?",
            )
            .bind(conversation_id)
            .bind(before_seq)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
                   FROM chat_messages
                  WHERE conversation_id = $1 AND message_seq < $2
                  ORDER BY message_seq DESC
                  LIMIT $3",
            )
            .bind(conversation_id)
            .bind(before_seq)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_messages_before"))
}

async fn list_messages_after(
    store: &impl AsStorePool,
    conversation_id: &str,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<ChatMessageRow>, BackendError> {
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
                   FROM chat_messages
                  WHERE conversation_id = ? AND message_seq > ?
                  ORDER BY message_seq ASC
                  LIMIT ?",
            )
            .bind(conversation_id)
            .bind(after_seq)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_as::<_, ChatMessageRow>(
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source
                   FROM chat_messages
                  WHERE conversation_id = $1 AND message_seq > $2
                  ORDER BY message_seq ASC
                  LIMIT $3",
            )
            .bind(conversation_id)
            .bind(after_seq)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(store_err("social::list_messages_after"))
}

/// Allocate the next per-conversation `message_seq` and touch `updated_at_ms`.
pub async fn allocate_message_seq_in_tx(
    tx: &mut DbTx<'_>,
    conversation_id: &str,
    created_at_ms: i64,
) -> Result<i64, BackendError> {
    match tx {
        DbTx::Sqlite(tx) => {
            let seq = sqlx::query_scalar::<_, i64>(
                "UPDATE conversations
                    SET next_message_seq = next_message_seq + 1,
                        updated_at_ms = ?
                  WHERE conversation_id = ?
              RETURNING next_message_seq - 1",
            )
            .bind(created_at_ms)
            .bind(conversation_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(store_err("social::allocate_message_seq"))?;
            Ok(seq)
        }
        DbTx::Postgres(tx) => {
            let seq = sqlx::query_scalar::<_, i64>(
                "UPDATE conversations
                    SET next_message_seq = next_message_seq + 1,
                        updated_at_ms = $1
                  WHERE conversation_id = $2
              RETURNING next_message_seq - 1",
            )
            .bind(created_at_ms)
            .bind(conversation_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(store_err("social::allocate_message_seq"))?;
            Ok(seq)
        }
    }
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
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source\n           FROM chat_messages\n          WHERE message_id IN (",
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
                "SELECT message_id, conversation_id, sender_account_id, sender_agent_id, text, created_at_ms, message_seq, reply_to_message_id, recalled_at_ms, sender_type, message_source\n           FROM chat_messages\n          WHERE message_id IN (",
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

/// List polymorphic mentions for the given message ids.
///
/// Returns account-only ids for backward-compatible call sites that only need
/// human unread/push targeting. Prefer [`list_message_mentions_full`] when
/// agent targets are required.
pub async fn list_message_mentions(
    store: &impl AsStorePool,
    message_ids: &[String],
) -> Result<HashMap<String, Vec<String>>, BackendError> {
    let full = list_message_mentions_full(store, message_ids).await?;
    Ok(full
        .into_iter()
        .map(|(message_id, mentions)| (message_id, mentions.account_ids))
        .collect())
}

/// List polymorphic mentions (account + agent) for the given message ids.
pub async fn list_message_mentions_full(
    store: &impl AsStorePool,
    message_ids: &[String],
) -> Result<HashMap<String, super::MessageMentions>, BackendError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT message_id, target_kind, target_id
                   FROM chat_message_mentions
                  WHERE message_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for message_id in message_ids {
                    separated.push_bind(message_id);
                }
            }
            // Appearance order SSOT: ordinal written at insert (not target_id lex).
            builder.push(") ORDER BY message_id ASC, target_kind ASC, ordinal ASC, target_id ASC");
            builder
                .build_query_as::<MessageMentionRow>()
                .fetch_all(pool)
                .await
        }
        StorePoolRef::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(
                "SELECT message_id, target_kind, target_id
                   FROM chat_message_mentions
                  WHERE message_id IN (",
            );
            {
                let mut separated = builder.separated(", ");
                for message_id in message_ids {
                    separated.push_bind(message_id);
                }
            }
            // Appearance order SSOT: ordinal written at insert (not target_id lex).
            builder.push(") ORDER BY message_id ASC, target_kind ASC, ordinal ASC, target_id ASC");
            builder
                .build_query_as::<MessageMentionRow>()
                .fetch_all(pool)
                .await
        }
    }
    .map_err(store_err("social::list_message_mentions"))?;

    let mut mentions_by_message = HashMap::<String, super::MessageMentions>::new();
    for row in rows {
        let entry = mentions_by_message.entry(row.message_id).or_default();
        match row.target_kind.as_str() {
            "agent" => entry.agent_ids.push(row.target_id),
            _ => entry.account_ids.push(row.target_id),
        }
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
    insert_message_with_mentions(
        store,
        conversation_id,
        sender_account_id,
        text,
        created_at_ms,
        reply_to_message_id,
        &super::MessageMentions::accounts(mentioned_account_ids.iter().cloned()),
    )
    .await
}

pub async fn insert_message_with_mentions(
    store: &impl AsStorePool,
    conversation_id: &str,
    sender_account_id: &str,
    text: &str,
    created_at_ms: i64,
    reply_to_message_id: Option<&str>,
    mentions: &super::MessageMentions,
) -> Result<ChatMessageRow, BackendError> {
    insert_message_with_id_full(
        store,
        conversation_id,
        sender_account_id,
        text,
        created_at_ms,
        reply_to_message_id,
        mentions,
        None,
        &[],
        minos_protocol::MessageSource::ClientLive.as_str(),
    )
    .await
    .map(|outcome| outcome.row)
}

/// Insert a user chat message, optionally with a client-owned id for multi-end
/// idempotent dual-write. If `client_message_id` already exists, returns the
/// existing row when the full request fingerprint matches.
pub async fn insert_message_with_id(
    store: &impl AsStorePool,
    conversation_id: &str,
    sender_account_id: &str,
    text: &str,
    created_at_ms: i64,
    reply_to_message_id: Option<&str>,
    mentioned_account_ids: &[String],
    client_message_id: Option<&str>,
) -> Result<ChatMessageRow, BackendError> {
    insert_message_with_id_full(
        store,
        conversation_id,
        sender_account_id,
        text,
        created_at_ms,
        reply_to_message_id,
        &super::MessageMentions::accounts(mentioned_account_ids.iter().cloned()),
        client_message_id,
        &[],
        "client_live",
    )
    .await
    .map(|outcome| outcome.row)
}

/// Full fingerprint insert (tests + callers that already open a pool).
#[allow(clippy::too_many_arguments)]
pub async fn insert_message_with_id_full(
    store: &impl AsStorePool,
    conversation_id: &str,
    sender_account_id: &str,
    text: &str,
    created_at_ms: i64,
    reply_to_message_id: Option<&str>,
    mentions: &super::MessageMentions,
    client_message_id: Option<&str>,
    attachment_blob_ids: &[String],
    message_source: &str,
) -> Result<InsertMessageOutcome, BackendError> {
    let mut tx = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => pool
            .begin()
            .await
            .map(DbTx::Sqlite)
            .map_err(store_err("social::insert_message.begin"))?,
        StorePoolRef::Postgres(pool) => pool
            .begin()
            .await
            .map(DbTx::Postgres)
            .map_err(store_err("social::insert_message.begin"))?,
    };
    let outcome = insert_message_with_id_in_tx(
        &mut tx,
        conversation_id,
        sender_account_id,
        text,
        created_at_ms,
        reply_to_message_id,
        mentions,
        client_message_id,
        attachment_blob_ids,
        message_source,
    )
    .await?;
    if outcome.inserted && !attachment_blob_ids.is_empty() {
        crate::store::message_attachments::link_blobs_to_message_in_tx(
            &mut tx,
            &outcome.row.message_id,
            attachment_blob_ids,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(outcome)
}

/// Insert a user message on an open transaction (for Transactional Outbox).
///
/// Idempotent hit on `client_message_id` does not write; caller must still
/// `ensure_social_message_delivery_in_tx` so a prior insert-without-durable can
/// be repaired.
///
/// Fingerprint (same intent only): conversation, sender, text, reply,
/// attachment blob set, `message_source`.
#[allow(clippy::too_many_arguments)]
pub async fn insert_message_with_id_in_tx(
    tx: &mut DbTx<'_>,
    conversation_id: &str,
    sender_account_id: &str,
    text: &str,
    created_at_ms: i64,
    reply_to_message_id: Option<&str>,
    mentions: &super::MessageMentions,
    client_message_id: Option<&str>,
    attachment_blob_ids: &[String],
    message_source: &str,
) -> Result<InsertMessageOutcome, BackendError> {
    let source = normalize_message_source(message_source);
    let want_attachments =
        crate::store::message_attachments::normalize_attachment_fingerprint(attachment_blob_ids);

    let message_id = match client_message_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => {
            let id = validate_client_message_id(id)?;
            // Must read on the open tx — SQLite test pools are single-connection.
            if let Some(existing) = get_message_in_tx(tx, id).await? {
                // Idempotency key is (sender, client_message_id) + request fingerprint.
                if existing.conversation_id != conversation_id {
                    return Err(BackendError::StoreQuery {
                        operation: "social::insert_message_with_id.conflict".into(),
                        message: format!(
                            "message_id {id} already exists in a different conversation"
                        ),
                    });
                }
                if existing.sender_account_id.as_deref() != Some(sender_account_id) {
                    return Err(idempotency_conflict(
                        id,
                        "already used by a different sender",
                    ));
                }
                if existing.text != text {
                    return Err(idempotency_conflict(
                        id,
                        "reused with different message body",
                    ));
                }
                let existing_reply = existing.reply_to_message_id.as_deref();
                if existing_reply != reply_to_message_id {
                    return Err(idempotency_conflict(
                        id,
                        "reused with different reply target",
                    ));
                }
                let existing_source = normalize_message_source(&existing.message_source);
                if existing_source != source {
                    return Err(idempotency_conflict(
                        id,
                        "reused with different message_source",
                    ));
                }
                let existing_attachments =
                    crate::store::message_attachments::list_blob_ids_for_message_in_tx(tx, id)
                        .await?;
                let existing_fp =
                    crate::store::message_attachments::normalize_attachment_fingerprint(
                        &existing_attachments,
                    );
                if existing_fp != want_attachments {
                    return Err(idempotency_conflict(
                        id,
                        "reused with different attachments",
                    ));
                }
                return Ok(InsertMessageOutcome {
                    row: existing,
                    inserted: false,
                });
            }
            id.to_string()
        }
        None => Uuid::new_v4().to_string(),
    };

    let message_seq = allocate_message_seq_in_tx(tx, conversation_id, created_at_ms).await?;

    match tx {
        DbTx::Sqlite(tx) => {
            sqlx::query(
                "INSERT INTO chat_messages
                    (message_id, conversation_id, sender_account_id, text, created_at_ms,
                     message_seq, reply_to_message_id, message_source)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&message_id)
            .bind(conversation_id)
            .bind(sender_account_id)
            .bind(text)
            .bind(created_at_ms)
            .bind(message_seq)
            .bind(reply_to_message_id)
            .bind(source)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::insert_message.insert"))?;
            insert_mention_rows_sqlite(tx, &message_id, mentions).await?;
        }
        DbTx::Postgres(tx) => {
            sqlx::query(
                "INSERT INTO chat_messages
                    (message_id, conversation_id, sender_account_id, text, created_at_ms,
                     message_seq, reply_to_message_id, message_source)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&message_id)
            .bind(conversation_id)
            .bind(sender_account_id)
            .bind(text)
            .bind(created_at_ms)
            .bind(message_seq)
            .bind(reply_to_message_id)
            .bind(source)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::insert_message.insert"))?;
            insert_mention_rows_postgres(tx, &message_id, mentions).await?;
        }
    }

    Ok(InsertMessageOutcome {
        row: ChatMessageRow {
            message_id,
            conversation_id: conversation_id.to_string(),
            sender_account_id: Some(sender_account_id.to_string()),
            sender_agent_id: None,
            text: text.to_string(),
            created_at_ms,
            message_seq,
            reply_to_message_id: reply_to_message_id.map(ToOwned::to_owned),
            recalled_at_ms: None,
            sender_type: "user".to_string(),
            message_source: source.to_string(),
        },
        inserted: true,
    })
}

async fn insert_mention_rows_sqlite(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    message_id: &str,
    mentions: &super::MessageMentions,
) -> Result<(), BackendError> {
    // Preserve caller appearance order via ordinal; only de-dupe, do not sort by id.
    let mut seen_accounts = std::collections::HashSet::new();
    let mut seen_agents = std::collections::HashSet::new();
    let mut account_ordinal: i64 = 0;
    for target_id in &mentions.account_ids {
        if !seen_accounts.insert(target_id.clone()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO chat_message_mentions
                (message_id, target_kind, target_id, ordinal)
             VALUES (?, 'account', ?, ?)",
        )
        .bind(message_id)
        .bind(target_id)
        .bind(account_ordinal)
        .execute(&mut **tx)
        .await
        .map_err(store_err("social::insert_message.insert_mention"))?;
        account_ordinal += 1;
    }
    let mut agent_ordinal: i64 = 0;
    for target_id in &mentions.agent_ids {
        if !seen_agents.insert(target_id.clone()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO chat_message_mentions
                (message_id, target_kind, target_id, ordinal)
             VALUES (?, 'agent', ?, ?)",
        )
        .bind(message_id)
        .bind(target_id)
        .bind(agent_ordinal)
        .execute(&mut **tx)
        .await
        .map_err(store_err("social::insert_message.insert_mention"))?;
        agent_ordinal += 1;
    }
    Ok(())
}

async fn insert_mention_rows_postgres(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    message_id: &str,
    mentions: &super::MessageMentions,
) -> Result<(), BackendError> {
    // Preserve caller appearance order via ordinal; only de-dupe, do not sort by id.
    let mut seen_accounts = std::collections::HashSet::new();
    let mut seen_agents = std::collections::HashSet::new();
    let mut account_ordinal: i64 = 0;
    for target_id in &mentions.account_ids {
        if !seen_accounts.insert(target_id.clone()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO chat_message_mentions
                (message_id, target_kind, target_id, ordinal)
             VALUES ($1, 'account', $2, $3)",
        )
        .bind(message_id)
        .bind(target_id)
        .bind(account_ordinal)
        .execute(&mut **tx)
        .await
        .map_err(store_err("social::insert_message.insert_mention"))?;
        account_ordinal += 1;
    }
    let mut agent_ordinal: i64 = 0;
    for target_id in &mentions.agent_ids {
        if !seen_agents.insert(target_id.clone()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO chat_message_mentions
                (message_id, target_kind, target_id, ordinal)
             VALUES ($1, 'agent', $2, $3)",
        )
        .bind(message_id)
        .bind(target_id)
        .bind(agent_ordinal)
        .execute(&mut **tx)
        .await
        .map_err(store_err("social::insert_message.insert_mention"))?;
        agent_ordinal += 1;
    }
    Ok(())
}

/// Fail-closed validation for untrusted `client_message_id` values.
///
/// Clients mint UUIDs / `msg_<uuid>` / `react-…` keys; host projection uses
/// `agent-result:{conv}:{session}:{origin}` (colons required). Reject path
/// separators, `..`, empty/overlong, and any charset outside
/// `[A-Za-z0-9_.:-]{1,256}` so ids cannot be used as filesystem escapes
/// downstream.
pub fn validate_client_message_id(id: &str) -> Result<&str, BackendError> {
    const MAX_LEN: usize = 256;
    if id.is_empty() || id.len() > MAX_LEN {
        return Err(invalid_client_message_id(
            id,
            "must be 1..=256 chars of [A-Za-z0-9_.:-]",
        ));
    }
    if id == "." || id == ".." {
        return Err(invalid_client_message_id(id, "must not be '.' or '..'"));
    }
    if !id
        .bytes()
        .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b':' | b'-'))
    {
        return Err(invalid_client_message_id(
            id,
            "must match [A-Za-z0-9_.:-]{1,256}",
        ));
    }
    Ok(id)
}

fn invalid_client_message_id(id: &str, detail: &str) -> BackendError {
    let preview: String = id.chars().take(64).collect();
    BackendError::StoreQuery {
        operation: "social::insert_message_with_id.invalid_client_message_id".into(),
        message: format!("invalid client_message_id '{preview}': {detail}"),
    }
}

fn normalize_message_source(source: &str) -> &'static str {
    minos_protocol::MessageSource::parse(source).as_str()
}

fn idempotency_conflict(client_message_id: &str, detail: &str) -> BackendError {
    BackendError::StoreQuery {
        operation: "social::insert_message_with_id.idempotency_conflict".into(),
        message: format!("client_message_id {client_message_id} {detail}"),
    }
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
    _agent_id: &str,
    session_id: &str,
) -> Result<(), BackendError> {
    // Session binding only. Never set sender_agent_id on the origin row —
    // user messages must keep sender_agent_id NULL (bot identity is not the author).
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
    // Session SSOT is agent_sessions (per conversation × bot). Do not infer from
    // chat_messages.sender_agent_id — user origin rows bind agent_session_id only.
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT session_id
                   FROM agent_sessions
                  WHERE conversation_id = ?
                    AND agent_id = ?
                    AND status NOT IN ('ended', 'failed', 'stopped')
                  ORDER BY started_at_ms DESC
                  LIMIT 1",
            )
            .bind(conversation_id)
            .bind(agent_id)
            .fetch_optional(pool)
            .await
        }
        StorePoolRef::Postgres(pool) => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT session_id
                   FROM agent_sessions
                  WHERE conversation_id = $1
                    AND agent_id = $2
                    AND status NOT IN ('ended', 'failed', 'stopped')
                  ORDER BY started_at_ms DESC
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
    let mut tx = match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => pool
            .begin()
            .await
            .map(DbTx::Sqlite)
            .map_err(store_err("social::recall_message.begin"))?,
        StorePoolRef::Postgres(pool) => pool
            .begin()
            .await
            .map(DbTx::Postgres)
            .map_err(store_err("social::recall_message.begin"))?,
    };
    recall_message_in_tx(
        &mut tx,
        conversation_id,
        message_id,
        sender_account_id,
        recalled_at_ms,
    )
    .await?;
    tx.commit().await?;
    get_message(store, message_id).await
}

/// Apply recall mutations on an open transaction (for Transactional Outbox).
pub async fn recall_message_in_tx(
    tx: &mut DbTx<'_>,
    conversation_id: &str,
    message_id: &str,
    sender_account_id: &str,
    recalled_at_ms: i64,
) -> Result<(), BackendError> {
    match tx {
        DbTx::Sqlite(tx) => {
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
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::recall_message.update_message"))?;

            sqlx::query(
                "DELETE FROM chat_message_mentions
                  WHERE message_id = ?",
            )
            .bind(message_id)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::recall_message.delete_mentions"))?;

            sqlx::query(
                "UPDATE conversations
                    SET updated_at_ms = MAX(updated_at_ms, ?)
                  WHERE conversation_id = ?",
            )
            .bind(recalled_at_ms)
            .bind(conversation_id)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::recall_message.touch_conversation"))?;
        }
        DbTx::Postgres(tx) => {
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
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::recall_message.update_message"))?;

            sqlx::query(
                "DELETE FROM chat_message_mentions
                  WHERE message_id = $1",
            )
            .bind(message_id)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::recall_message.delete_mentions"))?;

            sqlx::query(
                "UPDATE conversations
                    SET updated_at_ms = GREATEST(updated_at_ms, $1)
                  WHERE conversation_id = $2",
            )
            .bind(recalled_at_ms)
            .bind(conversation_id)
            .execute(&mut **tx)
            .await
            .map_err(store_err("social::recall_message.touch_conversation"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_client_message_id;

    #[test]
    fn accepts_uuid_and_client_prefixed_ids() {
        assert!(validate_client_message_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_client_message_id("msg_550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_client_message_id("msg-disabled-sole-1").is_ok());
        assert!(validate_client_message_id("react-171000-42").is_ok());
        assert!(
            validate_client_message_id("approval-550e8400-e29b-41d4-a716-446655440000").is_ok()
        );
    }

    #[test]
    fn accepts_agent_result_formula_with_colons() {
        assert!(validate_client_message_id("agent-result:conv:sess:origin-msg").is_ok());
        assert!(validate_client_message_id(
            "agent-result:550e8400-e29b-41d4-a716-446655440000:sess:msg_abc"
        )
        .is_ok());
    }

    #[test]
    fn rejects_path_escape_and_junk() {
        let overlong = "a".repeat(257);
        for bad in [
            "",
            ".",
            "..",
            "../etc/passwd",
            "/tmp/x",
            "foo/bar",
            "foo\\bar",
            "has space",
            "null\0byte",
            overlong.as_str(),
        ] {
            assert!(
                validate_client_message_id(bad).is_err(),
                "expected reject for {bad:?}"
            );
        }
    }
}
