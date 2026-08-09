//! Transactional Outbox helpers for social chat messages.
//!
//! Business rows (`chat_messages`) and delivery log (`durable_event_log` +
//! `outbox_events`) must commit in the same transaction. Publish happens after
//! commit via [`PendingDurablePublish`].

use minos_protocol::{ChatMessageSummary, DurableEvent, SenderRef, SenderType};
use uuid::Uuid;

use crate::app::tx::DbTx;
use crate::error::BackendError;
use crate::store::{durable_event_log, outbox_events};

use super::store_err;

/// One durable event that should be published after the writer commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDurablePublish {
    pub topic_kind: String,
    pub event_id: String,
    /// Present when this call enqueued a new outbox row (caller may ack after publish).
    pub outbox_id: Option<String>,
}

/// Max chars for account-topic T2 preview (inbox / push); full body is conversation-only.
const ACCOUNT_PREVIEW_MAX_CHARS: usize = 120;

fn sender_ref_for_message(message: &ChatMessageSummary) -> SenderRef {
    match message.sender_type {
        SenderType::Agent => SenderRef::Agent {
            agent_id: message.sender.account_id.clone(),
            session_id: None,
        },
        SenderType::User => SenderRef::User {
            account_id: message.sender.account_id.clone(),
        },
    }
}

/// Truncate message text for account-topic digest (T2).
fn account_preview(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in trimmed.chars().enumerate() {
        if i >= ACCOUNT_PREVIEW_MAX_CHARS {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// Build account-topic thin digest for one member (no full ChatMessageSummary).
fn account_append_digest(account_id: &str, message: &ChatMessageSummary) -> DurableEvent {
    let mentioned = message
        .mentioned_account_ids
        .iter()
        .any(|id| id == account_id);
    DurableEvent::AccountConversationMessageAppended {
        account_id: account_id.to_string(),
        conversation_id: message.conversation_id.clone(),
        message_id: message.message_id.clone(),
        sender: sender_ref_for_message(message),
        at_ms: message.created_at_ms,
        preview: account_preview(&message.text),
        sender_display_name: message.sender.display_name.clone(),
        mentioned,
        message_seq: Some(message.message_seq),
    }
}

fn account_recall_digest(
    account_id: &str,
    message: &ChatMessageSummary,
    at_ms: i64,
) -> DurableEvent {
    DurableEvent::AccountConversationMessageRecalled {
        account_id: account_id.to_string(),
        conversation_id: message.conversation_id.clone(),
        message_id: message.message_id.clone(),
        at_ms,
        preview: Some("Message recalled".into()),
        message_seq: Some(message.message_seq),
    }
}

fn conversation_event_id(message: &ChatMessageSummary) -> String {
    let action = if message.recalled_at_ms.is_some() {
        "recalled"
    } else {
        "appended"
    };
    format!(
        "social-conv-{action}-{}-{}",
        message.conversation_id, message.message_id
    )
}

fn account_conversation_event_id(account_id: &str, message: &ChatMessageSummary) -> String {
    let action = if message.recalled_at_ms.is_some() {
        "recalled"
    } else {
        "appended"
    };
    format!("social-{action}-{account_id}-{}", message.message_id)
}

async fn durable_exists_in_tx(
    tx: &mut DbTx<'_>,
    topic_kind: &str,
    event_id: &str,
) -> Result<bool, BackendError> {
    let exists = match tx {
        DbTx::Sqlite(tx) => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
               FROM durable_event_log
              WHERE topic_kind = ?
                AND event_id = ?",
        )
        .bind(topic_kind)
        .bind(event_id)
        .fetch_one(&mut **tx)
        .await
        .map(|n| n > 0),
        DbTx::Postgres(tx) => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
               FROM durable_event_log
              WHERE topic_kind = $1
                AND event_id = $2",
        )
        .bind(topic_kind)
        .bind(event_id)
        .fetch_one(&mut **tx)
        .await
        .map(|n| n > 0),
    }
    .map_err(store_err("social::delivery.durable_exists"))?;
    Ok(exists)
}

/// Publish membership change to conversation topic + affected account inbox.
pub async fn ensure_membership_change_delivery_in_tx(
    tx: &mut DbTx<'_>,
    conversation_id: &str,
    member_account_id: &str,
    actor_account_id: &str,
    action: &str,
    role: Option<&str>,
    membership_version: i64,
    at_ms: i64,
    // Remaining members after the change (conversation topic authorized at subscribe).
    remaining_member_ids: &[String],
) -> Result<Vec<PendingDurablePublish>, BackendError> {
    let _ = remaining_member_ids;
    let mut pending = Vec::with_capacity(2);
    let conv_event = DurableEvent::ConversationMemberChanged {
        conversation_id: conversation_id.to_string(),
        member_account_id: member_account_id.to_string(),
        action: action.to_string(),
        role: role.map(str::to_string),
        actor_account_id: actor_account_id.to_string(),
        membership_version,
        at_ms,
    };
    let conv_event_id = format!(
        "membership-conv-{conversation_id}-{member_account_id}-{action}-v{membership_version}"
    );
    pending.push(ensure_one_in_tx(tx, &conv_event_id, &conv_event, at_ms).await?);

    let account_event = DurableEvent::AccountConversationMembershipChanged {
        account_id: member_account_id.to_string(),
        conversation_id: conversation_id.to_string(),
        action: action.to_string(),
        membership_version,
        at_ms,
    };
    let account_event_id = format!(
        "membership-acct-{member_account_id}-{conversation_id}-{action}-v{membership_version}"
    );
    pending.push(ensure_one_in_tx(tx, &account_event_id, &account_event, at_ms).await?);

    Ok(pending)
}

async fn ensure_one_in_tx(
    tx: &mut DbTx<'_>,
    event_id: &str,
    event: &DurableEvent,
    at_ms: i64,
) -> Result<PendingDurablePublish, BackendError> {
    let topic_kind = event.topic().kind().as_str().to_string();
    if durable_exists_in_tx(tx, &topic_kind, event_id).await? {
        return Ok(PendingDurablePublish {
            topic_kind,
            event_id: event_id.to_string(),
            outbox_id: None,
        });
    }
    let cursor = durable_event_log::record_in_tx(tx, event_id, event, at_ms).await?;
    let outbox_id = Uuid::new_v4().to_string();
    outbox_events::enqueue_in_tx(
        tx,
        &outbox_id,
        cursor.topic.kind().as_str(),
        &cursor.event_id,
        outbox_events::OutboxLane::SocialDurable,
        at_ms,
    )
    .await?;
    Ok(PendingDurablePublish {
        topic_kind: cursor.topic.kind().as_str().to_string(),
        event_id: cursor.event_id,
        outbox_id: Some(outbox_id),
    })
}

/// Record conversation + account durable events (and outbox rows) for a social
/// message on the caller's open transaction.
///
/// Idempotent on deterministic event ids: safe for `client_message_id` retries
/// and repair of insert-without-durable holes.
pub async fn ensure_social_message_delivery_in_tx(
    tx: &mut DbTx<'_>,
    message: &ChatMessageSummary,
    member_account_ids: &[String],
) -> Result<Vec<PendingDurablePublish>, BackendError> {
    let at_ms = message.recalled_at_ms.unwrap_or(message.created_at_ms);
    let mut pending = Vec::with_capacity(1 + member_account_ids.len());

    let conversation_event = if message.recalled_at_ms.is_some() {
        DurableEvent::ConversationMessageRecalled {
            conversation_id: message.conversation_id.clone(),
            message_id: message.message_id.clone(),
            at_ms,
            message: Some(message.clone()),
        }
    } else {
        DurableEvent::ConversationMessageAppended {
            conversation_id: message.conversation_id.clone(),
            message_id: message.message_id.clone(),
            sender: sender_ref_for_message(message),
            at_ms,
            message: Some(message.clone()),
        }
    };
    pending.push(
        ensure_one_in_tx(
            tx,
            &conversation_event_id(message),
            &conversation_event,
            at_ms,
        )
        .await?,
    );

    for account_id in member_account_ids {
        // T2 account digest only — full ChatMessageSummary stays on conversation topic.
        let account_event = if message.recalled_at_ms.is_some() {
            account_recall_digest(account_id, message, at_ms)
        } else {
            account_append_digest(account_id, message)
        };
        let event_id = account_conversation_event_id(account_id, message);
        pending.push(ensure_one_in_tx(tx, &event_id, &account_event, at_ms).await?);
    }

    Ok(pending)
}

/// Deterministic reaction durable event_id (B6).
///
/// Formula:
/// `social-reaction-{conversation_id}-{message_id}-{emoji}-{actor_key}-{action}-{client_op_id}`
///
/// Same `client_op_id` retries share one id → `ensure_one_in_tx` no-op.
/// Different ops (including concurrent same-emoji toggles) use distinct
/// `client_op_id` values. Never includes `Uuid::new_v4()` or `at_ms`.
pub fn reaction_event_id(
    conversation_id: &str,
    message_id: &str,
    emoji: &str,
    actor_key: &str,
    action: &str,
    client_op_id: &str,
) -> String {
    format!(
        "social-reaction-{conversation_id}-{message_id}-{emoji}-{actor_key}-{action}-{client_op_id}"
    )
}

fn reaction_actor_key(actor: &SenderRef) -> String {
    match actor {
        SenderRef::User { account_id } => format!("user-{account_id}"),
        SenderRef::Agent { agent_id, .. } => format!("agent-{agent_id}"),
        SenderRef::System => "system".to_string(),
    }
}

/// Conversation-topic only reaction durable (no account fanout / rail unread).
///
/// `reactions` must be **viewer-neutral** (`reacted_by_me = false`); clients
/// recompute `reacted_by_me` from `actors` + local account id.
///
/// `client_op_id` is required and must equal the client outbox entry id (C5).
pub async fn ensure_reaction_delivery_in_tx(
    tx: &mut DbTx<'_>,
    conversation_id: &str,
    message_id: &str,
    emoji: &str,
    action: &str,
    actor: SenderRef,
    at_ms: i64,
    reactions: Vec<minos_protocol::ReactionGroup>,
    client_op_id: &str,
) -> Result<PendingDurablePublish, BackendError> {
    let client_op_id = client_op_id.trim();
    if client_op_id.is_empty() {
        return Err(BackendError::StoreQuery {
            operation: "social::delivery.reaction_client_op_id".into(),
            message: "client_op_id is required".into(),
        });
    }
    let actor_key = reaction_actor_key(&actor);
    let event_id = reaction_event_id(
        conversation_id,
        message_id,
        emoji,
        &actor_key,
        action,
        client_op_id,
    );
    let event = DurableEvent::ConversationMessageReactionUpdated {
        conversation_id: conversation_id.to_string(),
        message_id: message_id.to_string(),
        emoji: emoji.to_string(),
        action: action.to_string(),
        actor,
        at_ms,
        reactions,
    };
    ensure_one_in_tx(tx, &event_id, &event, at_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tx::Storage;
    use crate::store::social::{
        create_group_conversation, ensure_social_message_delivery_in_tx,
        insert_message_with_id_in_tx, list_conversation_members,
    };
    use crate::store::test_support::{insert_account, memory_pool, T0};
    use minos_protocol::{ChatMessageSummary, ReactionGroup, SenderType, UserSummary};

    #[tokio::test]
    async fn reaction_delivery_is_conversation_only() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "react-del-alice@example.com").await;
        let bob = insert_account(&pool, "react-del-bob@example.com").await;
        let conversation =
            create_group_conversation(&pool, &alice, "react-del", &[bob.clone()], T0)
                .await
                .unwrap();
        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let pending = ensure_reaction_delivery_in_tx(
            &mut tx,
            &conversation.conversation_id,
            "msg-1",
            "👍",
            "add",
            SenderRef::User {
                account_id: alice.clone(),
            },
            T0 + 1,
            vec![ReactionGroup {
                emoji: "👍".into(),
                count: 1,
                reacted_by_me: false,
                actors: vec![],
            }],
            "op-conv-only-1",
        )
        .await
        .unwrap();
        assert!(pending.outbox_id.is_some());
        assert_eq!(pending.topic_kind, "conversation");
        assert_eq!(
            pending.event_id,
            reaction_event_id(
                &conversation.conversation_id,
                "msg-1",
                "👍",
                &format!("user-{alice}"),
                "add",
                "op-conv-only-1",
            )
        );
        // B6: formula ends with client_op_id, not a random UUID.
        assert!(pending.event_id.ends_with("op-conv-only-1"));
        tx.commit().await.unwrap();

        // Zero account durables for reactions.
        let account_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM durable_event_log WHERE topic_kind = 'account'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(account_count, 0);
        let conv_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM durable_event_log WHERE topic_kind = 'conversation'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(conv_count, 1);
    }

    #[tokio::test]
    async fn reaction_same_client_op_id_is_idempotent() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "react-idem-alice@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "react-idem", &[], T0)
            .await
            .unwrap();
        let groups = vec![ReactionGroup {
            emoji: "🎉".into(),
            count: 1,
            reacted_by_me: false,
            actors: vec![],
        }];
        let actor = SenderRef::User {
            account_id: alice.clone(),
        };

        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let first = ensure_reaction_delivery_in_tx(
            &mut tx,
            &conversation.conversation_id,
            "msg-idem",
            "🎉",
            "add",
            actor.clone(),
            T0 + 1,
            groups.clone(),
            "client-op-same",
        )
        .await
        .unwrap();
        assert!(first.outbox_id.is_some());
        tx.commit().await.unwrap();

        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let second = ensure_reaction_delivery_in_tx(
            &mut tx,
            &conversation.conversation_id,
            "msg-idem",
            "🎉",
            "add",
            actor,
            T0 + 2,
            groups,
            "client-op-same",
        )
        .await
        .unwrap();
        // Same client_op_id → durable exists → no new outbox row.
        assert!(second.outbox_id.is_none());
        assert_eq!(first.event_id, second.event_id);
        tx.commit().await.unwrap();

        let conv_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM durable_event_log WHERE topic_kind = 'conversation'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(conv_count, 1);
    }

    #[tokio::test]
    async fn reaction_different_client_op_id_creates_distinct_events() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "react-diff-alice@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "react-diff", &[], T0)
            .await
            .unwrap();
        let groups = vec![ReactionGroup {
            emoji: "👍".into(),
            count: 1,
            reacted_by_me: false,
            actors: vec![],
        }];
        let actor = SenderRef::User {
            account_id: alice.clone(),
        };

        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let a = ensure_reaction_delivery_in_tx(
            &mut tx,
            &conversation.conversation_id,
            "msg-diff",
            "👍",
            "add",
            actor.clone(),
            T0 + 1,
            groups.clone(),
            "client-op-a",
        )
        .await
        .unwrap();
        let b = ensure_reaction_delivery_in_tx(
            &mut tx,
            &conversation.conversation_id,
            "msg-diff",
            "👍",
            "remove",
            actor,
            T0 + 2,
            groups,
            "client-op-b",
        )
        .await
        .unwrap();
        assert!(a.outbox_id.is_some());
        assert!(b.outbox_id.is_some());
        assert_ne!(a.event_id, b.event_id);
        tx.commit().await.unwrap();

        let conv_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM durable_event_log WHERE topic_kind = 'conversation'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(conv_count, 2);
    }

    #[test]
    fn reaction_event_id_formula_is_stable() {
        let id = reaction_event_id("c1", "m1", "👍", "user-a", "add", "op-9");
        assert_eq!(id, "social-reaction-c1-m1-👍-user-a-add-op-9");
        // Retry with same inputs must yield identical id (no UUID / at_ms).
        assert_eq!(
            id,
            reaction_event_id("c1", "m1", "👍", "user-a", "add", "op-9")
        );
    }

    #[tokio::test]
    async fn insert_and_delivery_share_one_transaction() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let bob = insert_account(&pool, "bob@example.com").await;
        let conversation =
            create_group_conversation(&pool, &alice, "tx-outbox", &[bob.clone()], T0)
                .await
                .unwrap();
        let members = list_conversation_members(&pool, &conversation.conversation_id)
            .await
            .unwrap();

        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let outcome = insert_message_with_id_in_tx(
            &mut tx,
            &conversation.conversation_id,
            &alice,
            "hello durable",
            T0 + 1,
            None,
            &crate::store::social::MessageMentions::empty(),
            Some("client-msg-1"),
            &[],
            "client_live",
        )
        .await
        .unwrap();
        assert!(outcome.inserted);

        let message = ChatMessageSummary {
            message_id: outcome.row.message_id.clone(),
            conversation_id: outcome.row.conversation_id.clone(),
            sender: UserSummary {
                account_id: alice.clone(),
                minos_id: "alice".into(),
                display_name: "Alice".into(),
            },
            text: outcome.row.text.clone(),
            created_at_ms: outcome.row.created_at_ms,
            message_seq: outcome.row.message_seq,
            reply_to: None,
            recalled_at_ms: None,
            mentioned_account_ids: vec![],
            mentioned_agent_ids: vec![],
            sender_type: SenderType::User,
            reactions: vec![],
            attachments: vec![],
        };
        let pending = ensure_social_message_delivery_in_tx(&mut tx, &message, &members)
            .await
            .unwrap();
        // 1 conversation + 2 members
        assert_eq!(pending.len(), 3);
        assert!(pending.iter().all(|p| p.outbox_id.is_some()));
        tx.commit().await.unwrap();

        // Durable rows must exist after commit.
        for item in &pending {
            let row = crate::store::durable_event_log::get(&pool, &item.topic_kind, &item.event_id)
                .await
                .unwrap();
            assert!(row.is_some(), "missing durable {}", item.event_id);
        }
    }

    #[tokio::test]
    async fn client_message_id_retry_repairs_missing_durable() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "repair", &[], T0)
            .await
            .unwrap();
        let members = list_conversation_members(&pool, &conversation.conversation_id)
            .await
            .unwrap();

        // Simulate hole: insert without durable.
        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let outcome = insert_message_with_id_in_tx(
            &mut tx,
            &conversation.conversation_id,
            &alice,
            "orphaned",
            T0 + 1,
            None,
            &crate::store::social::MessageMentions::empty(),
            Some("repair-id"),
            &[],
            "client_live",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(outcome.inserted);

        // Idempotent hit (same body fingerprint) + ensure delivery repairs the hole.
        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let again = insert_message_with_id_in_tx(
            &mut tx,
            &conversation.conversation_id,
            &alice,
            "orphaned",
            T0 + 2,
            None,
            &crate::store::social::MessageMentions::empty(),
            Some("repair-id"),
            &[],
            "client_live",
        )
        .await
        .unwrap();
        assert!(!again.inserted);
        let message = ChatMessageSummary {
            message_id: again.row.message_id.clone(),
            conversation_id: again.row.conversation_id.clone(),
            sender: UserSummary {
                account_id: alice.clone(),
                minos_id: "alice".into(),
                display_name: "Alice".into(),
            },
            text: again.row.text.clone(),
            created_at_ms: again.row.created_at_ms,
            message_seq: again.row.message_seq,
            reply_to: None,
            recalled_at_ms: None,
            mentioned_account_ids: vec![],
            mentioned_agent_ids: vec![],
            sender_type: SenderType::User,
            reactions: vec![],
            attachments: vec![],
        };
        let pending = ensure_social_message_delivery_in_tx(&mut tx, &message, &members)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(pending.len(), 1 + members.len());
        assert!(pending.iter().all(|p| p.outbox_id.is_some()));

        // Second repair is idempotent (no new outbox).
        let mut tx = Storage::begin(&pool).await.unwrap();
        let pending2 = ensure_social_message_delivery_in_tx(&mut tx, &message, &members)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert!(pending2.iter().all(|p| p.outbox_id.is_none()));
    }

    #[tokio::test]
    async fn account_fanout_is_thin_digest_conversation_is_full() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice-digest@example.com").await;
        let bob = insert_account(&pool, "bob-digest@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "digest", &[bob.clone()], T0)
            .await
            .unwrap();
        let members = list_conversation_members(&pool, &conversation.conversation_id)
            .await
            .unwrap();

        let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
        let outcome = insert_message_with_id_in_tx(
            &mut tx,
            &conversation.conversation_id,
            &alice,
            "hello thin digest body that should not appear nested under account",
            T0 + 1,
            None,
            &crate::store::social::MessageMentions::accounts([bob.clone()]),
            Some("digest-client-1"),
            &[],
            "client_live",
        )
        .await
        .unwrap();
        assert!(outcome.inserted);

        let message = ChatMessageSummary {
            message_id: outcome.row.message_id.clone(),
            conversation_id: outcome.row.conversation_id.clone(),
            sender: UserSummary {
                account_id: alice.clone(),
                minos_id: "alice".into(),
                display_name: "Alice".into(),
            },
            text: outcome.row.text.clone(),
            created_at_ms: outcome.row.created_at_ms,
            message_seq: outcome.row.message_seq,
            reply_to: None,
            recalled_at_ms: None,
            mentioned_account_ids: vec![bob.clone()],
            mentioned_agent_ids: vec![],
            sender_type: SenderType::User,
            reactions: vec![],
            attachments: vec![],
        };
        let pending = ensure_social_message_delivery_in_tx(&mut tx, &message, &members)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(pending.len(), 1 + members.len());

        // Conversation topic: full message nested.
        let conv_event_id = conversation_event_id(&message);
        let conv_row = crate::store::durable_event_log::get(&pool, "conversation", &conv_event_id)
            .await
            .unwrap()
            .expect("conversation durable");
        let conv_event: DurableEvent =
            serde_json::from_value(conv_row.payload_json.clone()).unwrap();
        match conv_event {
            DurableEvent::ConversationMessageAppended {
                message: Some(full),
                ..
            } => {
                assert_eq!(full.text, message.text);
                assert!(!full.message_id.is_empty());
            }
            other => panic!("expected ConversationMessageAppended full, got {other:?}"),
        }

        // Account topics: thin digest only (preview, no nested message).
        for account_id in &members {
            let event_id = account_conversation_event_id(account_id, &message);
            let row = crate::store::durable_event_log::get(&pool, "account", &event_id)
                .await
                .unwrap()
                .expect("account durable");
            assert!(
                row.payload_json.get("message").is_none(),
                "account payload must not nest full message"
            );
            let event: DurableEvent = serde_json::from_value(row.payload_json.clone()).unwrap();
            match event {
                DurableEvent::AccountConversationMessageAppended {
                    preview,
                    sender_display_name,
                    mentioned,
                    message_seq,
                    conversation_id,
                    message_id,
                    ..
                } => {
                    assert_eq!(conversation_id, message.conversation_id);
                    assert_eq!(message_id, message.message_id);
                    assert_eq!(preview, message.text);
                    assert_eq!(sender_display_name, "Alice");
                    assert_eq!(mentioned, account_id == &bob);
                    assert_eq!(message_seq, Some(message.message_seq));
                }
                other => panic!("expected thin AccountConversationMessageAppended, got {other:?}"),
            }
        }
    }

    #[test]
    fn account_preview_truncates_long_text() {
        let long = "x".repeat(200);
        let preview = account_preview(&long);
        assert_eq!(preview.chars().count(), ACCOUNT_PREVIEW_MAX_CHARS + 1); // + ellipsis
        assert!(preview.ends_with('…'));
    }

    #[tokio::test]
    async fn message_seq_is_monotonic_per_conversation() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "seq", &[], T0)
            .await
            .unwrap();
        let mut seqs = Vec::new();
        for i in 0..3 {
            let mut tx = pool.begin().await.map(DbTx::Sqlite).unwrap();
            let outcome = insert_message_with_id_in_tx(
                &mut tx,
                &conversation.conversation_id,
                &alice,
                &format!("m{i}"),
                T0 + i,
                None,
                &crate::store::social::MessageMentions::empty(),
                None,
                &[],
                "client_live",
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            seqs.push(outcome.row.message_seq);
        }
        assert_eq!(seqs, vec![1, 2, 3]);
        let page = crate::store::social::list_messages(
            &pool,
            &conversation.conversation_id,
            Some(3),
            None,
            10,
        )
        .await
        .unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].message_seq, 2);
        assert_eq!(page[1].message_seq, 1);
    }
}
