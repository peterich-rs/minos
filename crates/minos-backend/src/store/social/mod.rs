//! Social domain store functions, split into domain-specific submodules.
//!
//! Public API re-exports maintain backward compatibility with
//! `crate::store::social::*` paths.

pub mod agents;
pub mod conversation_messages;
pub mod conversations;
pub mod delivery;
pub mod friendships;
pub mod message_reactions;
pub mod profiles;

use minos_protocol::FriendRequestStatus;
use sqlx::FromRow;

use crate::error::BackendError;

// ─── Shared Row Types ─────────────────────────────────────────────────

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveFriendRequestTxResult {
    Resolved(FriendRequestRow),
    NotFound,
    Unauthorized,
    AlreadyResolved,
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
    /// Human author; `None` for agent-authored rows (`sender_agent_id` is SSOT).
    pub sender_account_id: Option<String>,
    pub sender_agent_id: Option<String>,
    pub text: String,
    pub created_at_ms: i64,
    pub message_seq: i64,
    pub reply_to_message_id: Option<String>,
    pub recalled_at_ms: Option<i64>,
    pub sender_type: String,
    /// `client_live` | `host_projection` | `system` — part of idempotency fingerprint.
    pub message_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub(crate) struct MessageMentionRow {
    pub(crate) message_id: String,
    pub(crate) target_kind: String,
    pub(crate) target_id: String,
}

/// Structured mention targets for one message (human accounts + bot agents).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageMentions {
    pub account_ids: Vec<String>,
    pub agent_ids: Vec<String>,
}

impl MessageMentions {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn accounts(account_ids: impl IntoIterator<Item = String>) -> Self {
        let mut account_ids: Vec<String> = account_ids.into_iter().collect();
        account_ids.sort();
        account_ids.dedup();
        Self {
            account_ids,
            agent_ids: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.account_ids.is_empty() && self.agent_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AgentRow {
    pub agent_id: String,
    pub owner_account_id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub avatar_url: Option<String>,
    /// `user` | `host_runtime` | `system`
    pub source: String,
    /// `active` | `disabled`
    pub status: String,
    pub runtime_agent: String,
    pub model: String,
    pub default_reasoning_effort: String,
    pub system_prompt: String,
    pub workspace_path: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl AgentRow {
    /// True when the bot is eligible for @ resolution and mailbox delivery.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// Test/helper constructor with digital-body defaults.
    pub fn test_stub(
        agent_id: impl Into<String>,
        owner_account_id: impl Into<String>,
        name: impl Into<String>,
        source: impl Into<String>,
        runtime_agent: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self {
            agent_id: agent_id.into(),
            owner_account_id: owner_account_id.into(),
            display_name: name.clone(),
            name,
            description: String::new(),
            avatar_url: None,
            source: source.into(),
            status: "active".into(),
            runtime_agent: runtime_agent.into(),
            model: String::new(),
            default_reasoning_effort: String::new(),
            system_prompt: String::new(),
            workspace_path: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ConversationAgentMemberRow {
    pub conversation_id: String,
    pub agent_id: String,
    pub added_by_account_id: String,
    pub joined_at_ms: i64,
}

// ─── Shared Helpers ───────────────────────────────────────────────────

pub(crate) fn friend_request_status_str(status: FriendRequestStatus) -> &'static str {
    match status {
        FriendRequestStatus::Pending => "pending",
        FriendRequestStatus::Accepted => "accepted",
        FriendRequestStatus::Rejected => "rejected",
        FriendRequestStatus::Canceled => "canceled",
    }
}

pub(crate) fn normalized_pair<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

pub(crate) fn store_err(operation: &'static str) -> impl FnOnce(sqlx::Error) -> BackendError {
    move |e| BackendError::StoreQuery {
        operation: operation.into(),
        message: e.to_string(),
    }
}

// ─── Re-exports from submodules ───────────────────────────────────────
// These maintain backward compatibility with `crate::store::social::*` paths.

// Profile functions
pub use profiles::{
    find_by_minos_id, profile_by_account, profiles_by_accounts, search_by_minos_id_prefix,
    set_display_name, set_minos_id,
};

// Friendship functions
pub use friendships::{
    are_friends, create_friend_request, create_friendship, get_friend_request,
    has_pending_friend_request_between, list_friendships_for, list_incoming_friend_requests,
    list_outgoing_friend_requests, resolve_friend_request, resolve_friend_request_transactional,
};

// Conversation functions
pub use conversations::{
    add_member_to_group, conversation_deleted_at_for_account, create_group_conversation,
    ensure_direct_conversation, ensure_group_conversation_with_id, get_conversation,
    get_member_role, is_conversation_member, list_conversation_member_profiles,
    list_conversation_members, list_conversations_for, mark_conversation_deleted_for_account,
    mark_conversation_read_to_latest, mark_conversation_read_to_seq, remove_member_from_group,
    upsert_group_conversation, MembershipChangeResult,
};

// Conversation message functions
pub use conversation_messages::{
    bind_session_to_message, bind_session_to_message_for_agent, get_message,
    has_bound_message_for_session, insert_message, insert_message_with_id,
    insert_message_with_id_full, insert_message_with_id_in_tx, insert_message_with_mentions,
    list_message_mentions, list_message_mentions_full, list_messages, list_messages_by_ids,
    lookup_latest_session_id_for_conversation, lookup_latest_session_id_for_conversation_agent,
    lookup_session_id_for_message, recall_message, recall_message_in_tx,
    suppress_live_ui_fanout_for_session, InsertMessageOutcome,
};

// Agent functions
pub use agents::{
    add_agent_to_conversation, agents_by_ids, delete_agent, ensure_host_runtime_agent,
    find_active_agent_name_conflict, find_host_runtime_agent, get_agent, insert_agent_message,
    insert_agent_message_with_session, insert_agent_message_with_session_in_tx,
    insert_bot_revision, is_agent_in_conversation, list_agents_for_owner, list_conversation_agents,
    list_conversation_agents_active, register_agent, register_agent_full,
    remove_agent_from_conversation, update_agent, update_agent_full, upsert_bot_deployment,
    RegisterAgentParams, UpdateAgentParams, AGENT_SOURCE_HOST_RUNTIME, AGENT_SOURCE_SYSTEM,
    AGENT_SOURCE_USER, AGENT_STATUS_ACTIVE, AGENT_STATUS_DISABLED, HOST_RUNTIME_AGENT_DESCRIPTION,
};

// Transactional outbox delivery for social messages
pub use delivery::{
    ensure_membership_change_delivery_in_tx, ensure_reaction_delivery_in_tx,
    ensure_social_message_delivery_in_tx, PendingDurablePublish,
};

// Cloud reactions
pub use message_reactions::{
    aggregate_for_message, aggregate_groups, get_reaction_client_op,
    insert_reaction_client_op_in_tx, list_for_message_in_tx, list_for_messages,
    set_reaction_client_op_action_in_tx, toggle_user_in_tx, try_claim_reaction_client_op_in_tx,
    MessageReactionRow, ReactionClientOpRow, ToggleReactionOutcome,
};
// B6 event_id helper
pub use delivery::reaction_event_id;

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{insert_account, memory_pool, T0};

    async fn seed_group(pool: &sqlx::SqlitePool) -> (String, String, String, String) {
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
        assert_eq!(last_read, Some((1, T0 + 1)));

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
    async fn insert_message_persists_agent_mentions() {
        let pool = memory_pool().await;
        let (conversation_id, alice, bob, _carol) = seed_group(&pool).await;
        let agent = register_agent(
            &pool,
            &alice,
            "Codex",
            "",
            "codex",
            "",
            None,
            T0,
        )
        .await
        .unwrap();
        add_agent_to_conversation(&pool, &conversation_id, &agent.agent_id, &alice, T0)
            .await
            .unwrap();

        let mentions = MessageMentions {
            account_ids: vec![bob.clone()],
            agent_ids: vec![agent.agent_id.clone()],
        };
        let message = insert_message_with_mentions(
            &pool,
            &conversation_id,
            &alice,
            "@bob @codex please review",
            T0 + 6,
            None,
            &mentions,
        )
        .await
        .unwrap();

        let message_id = message.message_id.clone();
        let full = list_message_mentions_full(&pool, &[message_id.clone()])
            .await
            .unwrap();
        let got = full.get(&message_id).expect("mentions");
        assert_eq!(got.account_ids, vec![bob]);
        assert_eq!(got.agent_ids, vec![agent.agent_id]);
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
            "\u{6536}\u{5230}",
            T0 + 2,
            Some(&original.message_id),
            &[],
        )
        .await
        .unwrap();

        let rows = list_messages(&pool, &conversation_id, None, None, 20)
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

    #[tokio::test]
    async fn upsert_group_conversation_creates_and_updates_title() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let conv_id = "work-conv-1";

        let created = upsert_group_conversation(&pool, conv_id, &alice, "First Title", &[], T0)
            .await
            .unwrap();
        assert_eq!(created.conversation_id, conv_id);
        assert_eq!(created.title.as_deref(), Some("First Title"));
        assert!(is_conversation_member(&pool, conv_id, &alice)
            .await
            .unwrap());

        let updated =
            upsert_group_conversation(&pool, conv_id, &alice, "Renamed Title", &[], T0 + 1)
                .await
                .unwrap();
        assert_eq!(updated.title.as_deref(), Some("Renamed Title"));

        // Messages survive title upsert.
        insert_message(&pool, conv_id, &alice, "keep me", T0 + 2, None, &[])
            .await
            .unwrap();
        upsert_group_conversation(&pool, conv_id, &alice, "Again", &[], T0 + 3)
            .await
            .unwrap();
        let messages = list_messages(&pool, conv_id, None, None, 10).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "keep me");
    }

    #[tokio::test]
    async fn upsert_group_conversation_skips_placeholder_title_overwrite() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice-ph@example.com").await;
        let conv_id = "work-conv-placeholder";

        upsert_group_conversation(&pool, conv_id, &alice, "Real Name", &[], T0)
            .await
            .unwrap();
        let after_placeholder =
            upsert_group_conversation(&pool, conv_id, &alice, "Conversation", &[], T0 + 1)
                .await
                .unwrap();
        assert_eq!(
            after_placeholder.title.as_deref(),
            Some("Real Name"),
            "placeholder title must not clobber a real Hub title"
        );

        let after_real =
            upsert_group_conversation(&pool, conv_id, &alice, "Renamed Real", &[], T0 + 2)
                .await
                .unwrap();
        assert_eq!(after_real.title.as_deref(), Some("Renamed Real"));
    }

    #[tokio::test]
    async fn insert_message_with_client_id_is_idempotent() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "Sync", &[], T0)
            .await
            .unwrap();
        let client_id = "msg_client_abc";

        let first = insert_message_with_id(
            &pool,
            &conversation.conversation_id,
            &alice,
            "hello",
            T0 + 1,
            None,
            &[],
            Some(client_id),
        )
        .await
        .unwrap();
        // Same fingerprint → idempotent success.
        let second = insert_message_with_id(
            &pool,
            &conversation.conversation_id,
            &alice,
            "hello",
            T0 + 2,
            None,
            &[],
            Some(client_id),
        )
        .await
        .unwrap();

        assert_eq!(first.message_id, client_id);
        assert_eq!(second.message_id, first.message_id);
        assert_eq!(second.text, "hello");
        let messages = list_messages(&pool, &conversation.conversation_id, None, None, 10)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);

        // Different body with same client_message_id → conflict (not silent success).
        let conflict = insert_message_with_id(
            &pool,
            &conversation.conversation_id,
            &alice,
            "different body",
            T0 + 3,
            None,
            &[],
            Some(client_id),
        )
        .await;
        assert!(
            matches!(
                &conflict,
                Err(crate::error::BackendError::StoreQuery { operation, .. })
                    if operation == "social::insert_message_with_id.idempotency_conflict"
            ),
            "expected body fingerprint conflict, got {conflict:?}"
        );

        // Different message_source with same body → conflict.
        let source_conflict = insert_message_with_id_full(
            &pool,
            &conversation.conversation_id,
            &alice,
            "hello",
            T0 + 4,
            None,
            &crate::store::social::MessageMentions::empty(),
            Some(client_id),
            &[],
            "host_projection",
        )
        .await;
        assert!(
            matches!(
                &source_conflict,
                Err(crate::error::BackendError::StoreQuery { operation, message })
                    if operation == "social::insert_message_with_id.idempotency_conflict"
                        && message.contains("message_source")
            ),
            "expected source fingerprint conflict, got {source_conflict:?}"
        );

        // Different attachment set with same body → conflict.
        let attach_conflict = insert_message_with_id_full(
            &pool,
            &conversation.conversation_id,
            &alice,
            "hello",
            T0 + 5,
            None,
            &crate::store::social::MessageMentions::empty(),
            Some(client_id),
            &["blob-a".into()],
            "client_live",
        )
        .await;
        assert!(
            matches!(
                &attach_conflict,
                Err(crate::error::BackendError::StoreQuery { operation, message })
                    if operation == "social::insert_message_with_id.idempotency_conflict"
                        && message.contains("attachments")
            ),
            "expected attachments fingerprint conflict, got {attach_conflict:?}"
        );
    }

    #[tokio::test]
    async fn ensure_host_runtime_agent_is_stable_per_owner_runtime() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let first = ensure_host_runtime_agent(&pool, &alice, "codex", "Codex", "", None, T0)
            .await
            .unwrap();
        let second =
            ensure_host_runtime_agent(&pool, &alice, "codex", "Codex", "gpt", None, T0 + 1)
                .await
                .unwrap();
        assert_eq!(first.agent_id, second.agent_id);
        assert_eq!(first.source, AGENT_SOURCE_HOST_RUNTIME);
        assert_eq!(first.runtime_agent, "codex");
    }

    #[tokio::test]
    async fn insert_agent_message_with_client_id_is_idempotent() {
        let pool = memory_pool().await;
        let alice = insert_account(&pool, "alice@example.com").await;
        let conversation = create_group_conversation(&pool, &alice, "Agents", &[], T0)
            .await
            .unwrap();
        let agent = ensure_host_runtime_agent(&pool, &alice, "claude", "Claude", "", None, T0)
            .await
            .unwrap();
        add_agent_to_conversation(
            &pool,
            &conversation.conversation_id,
            &agent.agent_id,
            &alice,
            T0,
        )
        .await
        .unwrap();
        let client_id = "agent-result:conv:sess:turn1";
        let first = insert_agent_message_with_session(
            &pool,
            &conversation.conversation_id,
            &agent.agent_id,
            "done",
            T0 + 1,
            None,
            Some("sess-1"),
            &[],
            &[],
            Some(client_id),
        )
        .await
        .unwrap();
        // Same fingerprint → idempotent hit returns original text.
        let second = insert_agent_message_with_session(
            &pool,
            &conversation.conversation_id,
            &agent.agent_id,
            "done",
            T0 + 2,
            None,
            Some("sess-1"),
            &[],
            &[],
            Some(client_id),
        )
        .await
        .unwrap();
        assert_eq!(first.message_id, client_id);
        assert_eq!(second.message_id, first.message_id);
        assert_eq!(second.text, "done");
        assert_eq!(
            list_messages(&pool, &conversation.conversation_id, None, None, 10)
                .await
                .unwrap()
                .len(),
            1
        );
        // Different body → conflict (no silent overwrite).
        let conflict = insert_agent_message_with_session(
            &pool,
            &conversation.conversation_id,
            &agent.agent_id,
            "ignored",
            T0 + 3,
            None,
            Some("sess-1"),
            &[],
            &[],
            Some(client_id),
        )
        .await;
        assert!(
            matches!(
                &conflict,
                Err(crate::error::BackendError::StoreQuery { operation, .. })
                    if operation == "social::insert_agent_message.idempotency_conflict"
            ),
            "expected agent body fingerprint conflict, got {conflict:?}"
        );
    }
}
