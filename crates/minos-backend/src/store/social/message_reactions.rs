//! Cloud message reactions store (multi-account SSOT).

use minos_protocol::{ReactionActor, ReactionGroup};
use sqlx::FromRow;
use uuid::Uuid;

use crate::app::tx::DbTx;
use crate::error::BackendError;
use crate::store::{AsStorePool, StorePoolRef};

use super::store_err;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct MessageReactionRow {
    pub reaction_id: String,
    pub message_id: String,
    pub conversation_id: String,
    pub emoji: String,
    pub actor_kind: String,
    pub actor_id: String,
    pub display_name: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleReactionOutcome {
    pub action: String, // "add" | "remove"
    pub reactions: Vec<ReactionGroup>,
}

pub async fn list_for_messages(
    store: &impl AsStorePool,
    message_ids: &[String],
) -> Result<Vec<MessageReactionRow>, BackendError> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let placeholders = message_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT reaction_id, message_id, conversation_id, emoji, actor_kind, actor_id, display_name, created_at_ms
                   FROM message_reactions
                  WHERE message_id IN ({placeholders})
               ORDER BY created_at_ms ASC"
            );
            let mut q = sqlx::query_as::<_, MessageReactionRow>(&sql);
            for id in message_ids {
                q = q.bind(id);
            }
            q.fetch_all(pool)
                .await
                .map_err(store_err("message_reactions::list_for_messages"))
        }
        StorePoolRef::Postgres(pool) => {
            let placeholders = message_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("${}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT reaction_id, message_id, conversation_id, emoji, actor_kind, actor_id, display_name, created_at_ms
                   FROM message_reactions
                  WHERE message_id IN ({placeholders})
               ORDER BY created_at_ms ASC"
            );
            let mut q = sqlx::query_as::<_, MessageReactionRow>(&sql);
            for id in message_ids {
                q = q.bind(id);
            }
            q.fetch_all(pool)
                .await
                .map_err(store_err("message_reactions::list_for_messages"))
        }
    }
}

/// Aggregate rows into viewer-resolved groups, ordered by first-seen emoji.
pub fn aggregate_groups(
    rows: &[MessageReactionRow],
    viewer_account_id: Option<&str>,
) -> Vec<ReactionGroup> {
    use std::collections::BTreeMap;
    let mut order: Vec<String> = Vec::new();
    let mut by_emoji: BTreeMap<String, Vec<&MessageReactionRow>> = BTreeMap::new();
    for row in rows {
        if !by_emoji.contains_key(&row.emoji) {
            order.push(row.emoji.clone());
        }
        by_emoji.entry(row.emoji.clone()).or_default().push(row);
    }
    order
        .into_iter()
        .filter_map(|emoji| {
            let actors_rows = by_emoji.get(&emoji)?;
            let actors: Vec<ReactionActor> = actors_rows
                .iter()
                .map(|r| ReactionActor {
                    actor_id: r.actor_id.clone(),
                    actor_kind: r.actor_kind.clone(),
                    display_name: r.display_name.clone(),
                })
                .collect();
            let reacted_by_me = viewer_account_id
                .map(|vid| {
                    actors
                        .iter()
                        .any(|a| a.actor_kind == "user" && a.actor_id == vid)
                })
                .unwrap_or(false);
            Some(ReactionGroup {
                emoji,
                count: u32::try_from(actors.len()).unwrap_or(0),
                reacted_by_me,
                actors,
            })
        })
        .collect()
}

pub async fn aggregate_for_message(
    store: &impl AsStorePool,
    message_id: &str,
    viewer_account_id: Option<&str>,
) -> Result<Vec<ReactionGroup>, BackendError> {
    let rows = list_for_messages(store, &[message_id.to_string()]).await?;
    Ok(aggregate_groups(&rows, viewer_account_id))
}

/// Toggle reaction for a user actor inside an open transaction.
pub async fn toggle_user_in_tx(
    tx: &mut DbTx<'_>,
    conversation_id: &str,
    message_id: &str,
    account_id: &str,
    display_name: &str,
    emoji: &str,
    now_ms: i64,
) -> Result<String, BackendError> {
    let emoji = emoji.trim();
    if emoji.is_empty() || emoji.chars().count() > 32 {
        return Err(BackendError::StoreQuery {
            operation: "message_reactions::toggle_user_in_tx".into(),
            message: "invalid emoji".into(),
        });
    }

    let existing = match tx {
        DbTx::Sqlite(tx) => sqlx::query_scalar::<_, String>(
            "SELECT reaction_id FROM message_reactions
              WHERE message_id = ? AND emoji = ? AND actor_kind = 'user' AND actor_id = ?",
        )
        .bind(message_id)
        .bind(emoji)
        .bind(account_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_err("message_reactions::toggle_find_sqlite"))?,
        DbTx::Postgres(tx) => sqlx::query_scalar::<_, String>(
            "SELECT reaction_id FROM message_reactions
              WHERE message_id = $1 AND emoji = $2 AND actor_kind = 'user' AND actor_id = $3",
        )
        .bind(message_id)
        .bind(emoji)
        .bind(account_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_err("message_reactions::toggle_find_postgres"))?,
    };

    if let Some(reaction_id) = existing {
        match tx {
            DbTx::Sqlite(tx) => {
                sqlx::query("DELETE FROM message_reactions WHERE reaction_id = ?")
                    .bind(&reaction_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(store_err("message_reactions::toggle_delete_sqlite"))?;
            }
            DbTx::Postgres(tx) => {
                sqlx::query("DELETE FROM message_reactions WHERE reaction_id = $1")
                    .bind(&reaction_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(store_err("message_reactions::toggle_delete_postgres"))?;
            }
        }
        return Ok("remove".into());
    }

    let reaction_id = Uuid::new_v4().to_string();
    match tx {
        DbTx::Sqlite(tx) => {
            sqlx::query(
                "INSERT INTO message_reactions
                    (reaction_id, message_id, conversation_id, emoji, actor_kind, actor_id, display_name, created_at_ms)
                 VALUES (?, ?, ?, ?, 'user', ?, ?, ?)",
            )
            .bind(&reaction_id)
            .bind(message_id)
            .bind(conversation_id)
            .bind(emoji)
            .bind(account_id)
            .bind(display_name)
            .bind(now_ms)
            .execute(&mut **tx)
            .await
            .map_err(store_err("message_reactions::toggle_insert_sqlite"))?;
        }
        DbTx::Postgres(tx) => {
            sqlx::query(
                "INSERT INTO message_reactions
                    (reaction_id, message_id, conversation_id, emoji, actor_kind, actor_id, display_name, created_at_ms)
                 VALUES ($1, $2, $3, $4, 'user', $5, $6, $7)",
            )
            .bind(&reaction_id)
            .bind(message_id)
            .bind(conversation_id)
            .bind(emoji)
            .bind(account_id)
            .bind(display_name)
            .bind(now_ms)
            .execute(&mut **tx)
            .await
            .map_err(store_err("message_reactions::toggle_insert_postgres"))?;
        }
    }
    Ok("add".into())
}

/// List reactions inside a transaction (post-toggle aggregate).
pub async fn list_for_message_in_tx(
    tx: &mut DbTx<'_>,
    message_id: &str,
) -> Result<Vec<MessageReactionRow>, BackendError> {
    match tx {
        DbTx::Sqlite(tx) => sqlx::query_as::<_, MessageReactionRow>(
            "SELECT reaction_id, message_id, conversation_id, emoji, actor_kind, actor_id, display_name, created_at_ms
               FROM message_reactions
              WHERE message_id = ?
           ORDER BY created_at_ms ASC",
        )
        .bind(message_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(store_err("message_reactions::list_in_tx_sqlite")),
        DbTx::Postgres(tx) => sqlx::query_as::<_, MessageReactionRow>(
            "SELECT reaction_id, message_id, conversation_id, emoji, actor_kind, actor_id, display_name, created_at_ms
               FROM message_reactions
              WHERE message_id = $1
           ORDER BY created_at_ms ASC",
        )
        .bind(message_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(store_err("message_reactions::list_in_tx_postgres")),
    }
}

/// Prior successful reaction op for Intent Outbox idempotency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionClientOpRow {
    pub client_op_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub emoji: String,
    pub action: String,
    pub account_id: String,
    pub created_at_ms: i64,
}

pub async fn get_reaction_client_op(
    store: &impl AsStorePool,
    client_op_id: &str,
) -> Result<Option<ReactionClientOpRow>, BackendError> {
    let client_op_id = client_op_id.trim();
    if client_op_id.is_empty() {
        return Ok(None);
    }
    match store.as_store_pool() {
        StorePoolRef::Sqlite(pool) => {
            let row = sqlx::query_as::<_, (String, String, String, String, String, String, i64)>(
                "SELECT client_op_id, conversation_id, message_id, emoji, action, account_id, created_at_ms
                   FROM reaction_client_ops
                  WHERE client_op_id = ?",
            )
            .bind(client_op_id)
            .fetch_optional(pool)
            .await
            .map_err(store_err("message_reactions::get_reaction_client_op"))?;
            Ok(row.map(
                |(
                    client_op_id,
                    conversation_id,
                    message_id,
                    emoji,
                    action,
                    account_id,
                    created_at_ms,
                )| {
                    ReactionClientOpRow {
                        client_op_id,
                        conversation_id,
                        message_id,
                        emoji,
                        action,
                        account_id,
                        created_at_ms,
                    }
                },
            ))
        }
        StorePoolRef::Postgres(pool) => {
            let row = sqlx::query_as::<_, (String, String, String, String, String, String, i64)>(
                "SELECT client_op_id, conversation_id, message_id, emoji, action, account_id, created_at_ms
                   FROM reaction_client_ops
                  WHERE client_op_id = $1",
            )
            .bind(client_op_id)
            .fetch_optional(pool)
            .await
            .map_err(store_err("message_reactions::get_reaction_client_op"))?;
            Ok(row.map(
                |(
                    client_op_id,
                    conversation_id,
                    message_id,
                    emoji,
                    action,
                    account_id,
                    created_at_ms,
                )| {
                    ReactionClientOpRow {
                        client_op_id,
                        conversation_id,
                        message_id,
                        emoji,
                        action,
                        account_id,
                        created_at_ms,
                    }
                },
            ))
        }
    }
}

/// Claim `client_op_id` uniquely inside a write tx.
///
/// Returns `true` when this transaction owns the op (insert succeeded).
/// Returns `false` when another writer already claimed the same id — caller
/// must **not** toggle again and should return the prior aggregate.
pub async fn try_claim_reaction_client_op_in_tx(
    tx: &mut DbTx<'_>,
    client_op_id: &str,
    conversation_id: &str,
    message_id: &str,
    emoji: &str,
    action: &str,
    account_id: &str,
    created_at_ms: i64,
) -> Result<bool, BackendError> {
    let rows = match tx {
        DbTx::Sqlite(tx) => sqlx::query(
            "INSERT OR IGNORE INTO reaction_client_ops
                    (client_op_id, conversation_id, message_id, emoji, action, account_id, created_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(client_op_id)
        .bind(conversation_id)
        .bind(message_id)
        .bind(emoji)
        .bind(action)
        .bind(account_id)
        .bind(created_at_ms)
        .execute(&mut **tx)
        .await
        .map_err(store_err("message_reactions::try_claim_reaction_client_op"))?
        .rows_affected(),
        DbTx::Postgres(tx) => sqlx::query(
            "INSERT INTO reaction_client_ops
                    (client_op_id, conversation_id, message_id, emoji, action, account_id, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (client_op_id) DO NOTHING",
        )
        .bind(client_op_id)
        .bind(conversation_id)
        .bind(message_id)
        .bind(emoji)
        .bind(action)
        .bind(account_id)
        .bind(created_at_ms)
        .execute(&mut **tx)
        .await
        .map_err(store_err("message_reactions::try_claim_reaction_client_op"))?
        .rows_affected(),
    };
    Ok(rows == 1)
}

/// Update action after a successful claim + toggle (same tx).
pub async fn set_reaction_client_op_action_in_tx(
    tx: &mut DbTx<'_>,
    client_op_id: &str,
    action: &str,
) -> Result<(), BackendError> {
    match tx {
        DbTx::Sqlite(tx) => {
            sqlx::query("UPDATE reaction_client_ops SET action = ?1 WHERE client_op_id = ?2")
                .bind(action)
                .bind(client_op_id)
                .execute(&mut **tx)
                .await
                .map_err(store_err(
                    "message_reactions::set_reaction_client_op_action",
                ))?;
        }
        DbTx::Postgres(tx) => {
            sqlx::query("UPDATE reaction_client_ops SET action = $1 WHERE client_op_id = $2")
                .bind(action)
                .bind(client_op_id)
                .execute(&mut **tx)
                .await
                .map_err(store_err(
                    "message_reactions::set_reaction_client_op_action",
                ))?;
        }
    }
    Ok(())
}

/// Backward-compatible name: claim with known action (no rows_affected check).
pub async fn insert_reaction_client_op_in_tx(
    tx: &mut DbTx<'_>,
    client_op_id: &str,
    conversation_id: &str,
    message_id: &str,
    emoji: &str,
    action: &str,
    account_id: &str,
    created_at_ms: i64,
) -> Result<(), BackendError> {
    let _ = try_claim_reaction_client_op_in_tx(
        tx,
        client_op_id,
        conversation_id,
        message_id,
        emoji,
        action,
        account_id,
        created_at_ms,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::social::{create_group_conversation, insert_message};
    use crate::store::test_support::{insert_account, memory_pool, T0};

    #[tokio::test]
    async fn toggle_add_remove_idempotent() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "react-alice@example.com").await;
        let bob = insert_account(&pool, "react-bob@example.com").await;
        let conv = create_group_conversation(&pool, &alice, "react", &[bob], T0)
            .await
            .unwrap();
        let msg = insert_message(
            &pool,
            &conv.conversation_id,
            &alice,
            "hi",
            T0 + 1,
            None,
            &[],
        )
        .await
        .unwrap();

        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let action = toggle_user_in_tx(
            &mut tx,
            &conv.conversation_id,
            &msg.message_id,
            &alice,
            "Alice",
            "👍",
            T0 + 2,
        )
        .await
        .unwrap();
        assert_eq!(action, "add");
        let rows = list_for_message_in_tx(&mut tx, &msg.message_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let groups = aggregate_groups(&rows, Some(&alice));
        assert_eq!(groups.len(), 1);
        assert!(groups[0].reacted_by_me);
        assert_eq!(groups[0].count, 1);

        let action = toggle_user_in_tx(
            &mut tx,
            &conv.conversation_id,
            &msg.message_id,
            &alice,
            "Alice",
            "👍",
            T0 + 3,
        )
        .await
        .unwrap();
        assert_eq!(action, "remove");
        let rows = list_for_message_in_tx(&mut tx, &msg.message_id)
            .await
            .unwrap();
        assert!(rows.is_empty());
        tx.commit().await.unwrap();
    }
}
