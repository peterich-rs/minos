use std::sync::Arc;

use async_trait::async_trait;
use minos_domain::DeviceId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app::tx::{DbTx, Storage};
use crate::error::BackendError;
use crate::store;
use crate::store::{AsStorePool, StoreHandle, StorePoolRef};

/// Minimal hex-encoding helper -- avoids pulling in the `hex` crate for a
/// handful of call-sites in this module.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{b:02x}").expect("String write never fails");
    }
    out
}

// ---------------------------------------------------------------------------
// Row types for domain entities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRow {
    pub account_id: String,
    pub email: String,
    pub minos_id: Option<String>,
    pub display_name: Option<String>,
    pub created_at_ms: i64,
    pub last_login_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRow {
    pub account_id: String,
    pub password_hash: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallationRow {
    pub installation_id: String,
    pub kind: String,
    pub platform: Option<String>,
    pub public_key: Option<String>,
    pub account_id: Option<String>,
    pub display_name: Option<String>,
    pub created_at_ms: i64,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTokenRow {
    pub token_hash: String,
    pub account_id: String,
    pub installation_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub rotated_to_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostTokenRow {
    pub token_hash: String,
    pub host_installation_id: String,
    pub issued_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingCodeRow {
    pub code_hash: String,
    pub host_installation_id: String,
    pub account_id: Option<String>,
    pub linked_via_installation_id: Option<String>,
    pub status: String,
    pub client_request_id: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub confirmed_at_ms: Option<i64>,
    pub redeemed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostLinkRow {
    pub pair_id: String,
    pub account_id: String,
    pub host_installation_id: String,
    pub linked_via_installation_id: String,
    pub link_display_name: Option<String>,
    pub acl_json: String,
    pub paired_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRow {
    pub agent_id: String,
    pub runtime_kind: String,
    pub display_name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRow {
    pub project_id: String,
    pub account_id: String,
    pub name: String,
    pub workspace_root: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub archived_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRow {
    pub conversation_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub created_by_account_id: String,
    pub direct_account_low: Option<String>,
    pub direct_account_high: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRow {
    pub message_id: String,
    pub conversation_id: String,
    pub sender_kind: String,
    pub sender_account_id: Option<String>,
    pub sender_agent_id: Option<String>,
    pub body_json: String,
    pub reply_to_message_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub created_at_ms: i64,
    pub recalled_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableEventRow {
    pub event_id: String,
    pub topic: String,
    pub topic_kind: String,
    pub topic_seq: i64,
    pub partition_key: String,
    pub payload_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicCursor {
    pub topic: String,
    pub topic_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxRow {
    pub outbox_id: String,
    pub topic_kind: String,
    pub event_id: String,
    pub status: String,
    pub available_at_ms: i64,
    pub attempts: i32,
    pub claimed_by: Option<String>,
    pub claimed_at_ms: Option<i64>,
    pub ack_at_ms: Option<i64>,
    pub dead_at_ms: Option<i64>,
    pub last_error_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRow {
    pub audit_id: String,
    pub actor_kind: String,
    pub account_id: Option<String>,
    pub installation_id: Option<String>,
    pub event_type: String,
    pub metadata: Option<String>,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushTokenRow {
    pub token_hash: String,
    pub account_id: String,
    pub installation_id: String,
    pub kind: String,
    pub locale: Option<String>,
    pub created_at_ms: i64,
    pub last_used_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}

// ---------------------------------------------------------------------------
// Existing repository traits (preserved from before P0.S2)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AgentSessionsRepository: Send + Sync {
    async fn get_for_account(
        &self,
        session_id: &str,
        account_id: &str,
    ) -> Result<Option<store::agent_sessions::AgentSessionRow>, BackendError>;

    async fn list_for_account(
        &self,
        account_id: &str,
        before_started_at_ms: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_sessions::AgentSessionRow>, BackendError>;

    async fn list_for_account_conversation(
        &self,
        conversation_id: &str,
        account_id: &str,
        before_started_at_ms: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_sessions::AgentSessionRow>, BackendError>;

    async fn list_for_account_project(
        &self,
        project_id: &str,
        account_id: &str,
        before_started_at_ms: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_sessions::AgentSessionRow>, BackendError>;
}

#[async_trait]
pub trait AgentTurnsRepository: Send + Sync {
    async fn get_for_account(
        &self,
        turn_id: &str,
        account_id: &str,
    ) -> Result<Option<store::agent_turns::AgentTurnRow>, BackendError>;

    async fn list_for_session(
        &self,
        session_id: &str,
        after_turn_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_turns::AgentTurnRow>, BackendError>;
}

#[async_trait]
pub trait AgentTurnEventsRepository: Send + Sync {
    async fn list_for_turn(
        &self,
        turn_id: &str,
        after_event_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_turn_events::AgentTurnEventRow>, BackendError>;
}

#[async_trait]
pub trait ApprovalRequestsRepository: Send + Sync {
    async fn get(
        &self,
        request_id: &str,
    ) -> Result<Option<store::approval_requests::ApprovalRequestRow>, BackendError>;

    async fn resolve(
        &self,
        request_id: &str,
        state: store::approval_requests::ApprovalRequestState,
        resolved_at_ms: i64,
        resolution_json: Option<&Value>,
    ) -> Result<bool, BackendError>;
}

#[async_trait]
pub trait AccountHostPairingsRepository: Send + Sync {
    async fn exists(
        &self,
        host_device_id: DeviceId,
        account_id: &str,
    ) -> Result<bool, BackendError>;
}

// ---------------------------------------------------------------------------
// New P0.S2 repository traits
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AccountsRepository: Send + Sync {
    async fn create(
        &self,
        email: &str,
        password_hash: &str,
        display_name: Option<&str>,
        at_ms: i64,
    ) -> Result<AccountRow, BackendError>;

    async fn find_by_email(&self, email: &str) -> Result<Option<AccountRow>, BackendError>;

    async fn find_by_id(&self, account_id: &str) -> Result<Option<AccountRow>, BackendError>;

    async fn touch_last_login(&self, account_id: &str, at_ms: i64) -> Result<(), BackendError>;

    async fn update_password_hash(
        &self,
        account_id: &str,
        hash: &str,
        at_ms: i64,
    ) -> Result<(), BackendError>;
}

#[async_trait]
pub trait AccountCredentialsRepository: Send + Sync {
    async fn upsert(
        &self,
        account_id: &str,
        password_hash: &str,
        at_ms: i64,
    ) -> Result<CredentialRow, BackendError>;

    async fn find_by_account(
        &self,
        account_id: &str,
    ) -> Result<Option<CredentialRow>, BackendError>;
}

#[async_trait]
pub trait InstallationsRepository: Send + Sync {
    async fn upsert(
        &self,
        installation_id: &str,
        kind: &str,
        platform: Option<&str>,
        public_key: Option<&str>,
        account_id: Option<&str>,
        display_name: Option<&str>,
        at_ms: i64,
    ) -> Result<InstallationRow, BackendError>;

    async fn find(&self, installation_id: &str) -> Result<Option<InstallationRow>, BackendError>;

    async fn touch_last_seen(&self, installation_id: &str, at_ms: i64) -> Result<(), BackendError>;
}

#[async_trait]
pub trait RefreshTokensRepository: Send + Sync {
    async fn insert(
        &self,
        plaintext: &str,
        account_id: &str,
        installation_id: &str,
        ttl_ms: i64,
        at_ms: i64,
    ) -> Result<RefreshTokenRow, BackendError>;

    async fn rotate(
        &self,
        old_plaintext: &str,
        new_plaintext: &str,
        ttl_ms: i64,
        at_ms: i64,
    ) -> Result<RefreshTokenRow, BackendError>;

    async fn find_active(&self, plaintext: &str) -> Result<Option<RefreshTokenRow>, BackendError>;

    async fn revoke_all_for_account(
        &self,
        account_id: &str,
        at_ms: i64,
    ) -> Result<u64, BackendError>;
}

#[async_trait]
pub trait HostInstallationTokensRepository: Send + Sync {
    async fn insert(&self, host_installation_id: &str, at_ms: i64) -> Result<String, BackendError>;

    async fn find_active(&self, token_hash: &str) -> Result<Option<HostTokenRow>, BackendError>;

    async fn revoke(&self, token_hash: &str, at_ms: i64) -> Result<bool, BackendError>;
}

#[async_trait]
pub trait PairingCodesRepository: Send + Sync {
    async fn insert(
        &self,
        code_hash: &str,
        host_installation_id: &str,
        ttl_ms: i64,
        client_request_id: Option<&str>,
        at_ms: i64,
    ) -> Result<PairingCodeRow, BackendError>;

    async fn find_by_code(&self, code_hash: &str) -> Result<Option<PairingCodeRow>, BackendError>;

    async fn update_status(
        &self,
        code_hash: &str,
        new_status: &str,
        at_ms: i64,
    ) -> Result<bool, BackendError>;

    async fn expire_stale(&self, now_ms: i64) -> Result<u64, BackendError>;
}

#[async_trait]
pub trait HostLinksRepository: Send + Sync {
    async fn insert(
        &self,
        account_id: &str,
        host_installation_id: &str,
        linked_via: &str,
        display_name: Option<&str>,
        acl_json: &str,
        at_ms: i64,
    ) -> Result<HostLinkRow, BackendError>;

    async fn exists(
        &self,
        account_id: &str,
        host_installation_id: &str,
    ) -> Result<bool, BackendError>;

    async fn list_for_account(&self, account_id: &str) -> Result<Vec<HostLinkRow>, BackendError>;

    async fn list_for_host(
        &self,
        host_installation_id: &str,
    ) -> Result<Vec<HostLinkRow>, BackendError>;

    async fn pick_default_host(&self, account_id: &str) -> Result<Option<String>, BackendError>;

    async fn remove(
        &self,
        account_id: &str,
        host_installation_id: &str,
    ) -> Result<bool, BackendError>;
}

#[async_trait]
pub trait AgentsRepository: Send + Sync {
    async fn list_enabled(&self) -> Result<Vec<AgentRow>, BackendError>;

    async fn find(&self, agent_id: &str) -> Result<Option<AgentRow>, BackendError>;
}

#[async_trait]
pub trait ProjectsRepository: Send + Sync {
    async fn create(
        &self,
        account_id: &str,
        name: &str,
        workspace_root: &str,
        at_ms: i64,
    ) -> Result<ProjectRow, BackendError>;

    async fn find(&self, project_id: &str) -> Result<Option<ProjectRow>, BackendError>;

    async fn list_for_account(
        &self,
        account_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<ProjectRow>, BackendError>;

    async fn update(
        &self,
        project_id: &str,
        name: Option<&str>,
        workspace_root: Option<&str>,
        at_ms: i64,
    ) -> Result<ProjectRow, BackendError>;

    async fn archive(&self, project_id: &str, at_ms: i64) -> Result<bool, BackendError>;
}

#[async_trait]
pub trait ConversationsRepository: Send + Sync {
    async fn create(
        &self,
        kind: &str,
        created_by_account_id: &str,
        title: Option<&str>,
        project_id: Option<&str>,
        at_ms: i64,
    ) -> Result<ConversationRow, BackendError>;

    async fn find(&self, conversation_id: &str) -> Result<Option<ConversationRow>, BackendError>;

    async fn find_direct(
        &self,
        account_low: &str,
        account_high: &str,
    ) -> Result<Option<ConversationRow>, BackendError>;

    async fn list_for_account(
        &self,
        account_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<ConversationRow>, BackendError>;

    async fn is_member(
        &self,
        conversation_id: &str,
        account_id: &str,
    ) -> Result<bool, BackendError>;

    async fn project_id(&self, conversation_id: &str) -> Result<Option<String>, BackendError>;

    async fn update_at(&self, conversation_id: &str, at_ms: i64) -> Result<(), BackendError>;
}

#[async_trait]
pub trait ConversationMessagesRepository: Send + Sync {
    async fn insert(
        &self,
        conversation_id: &str,
        sender_kind: &str,
        sender_account_id: Option<&str>,
        sender_agent_id: Option<&str>,
        body_json: &str,
        reply_to_message_id: Option<&str>,
        agent_session_id: Option<&str>,
        at_ms: i64,
    ) -> Result<MessageRow, BackendError>;

    async fn find(&self, message_id: &str) -> Result<Option<MessageRow>, BackendError>;

    async fn list_for_conversation(
        &self,
        conversation_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<MessageRow>, BackendError>;

    async fn recall(&self, message_id: &str, at_ms: i64) -> Result<bool, BackendError>;

    async fn insert_mentions(
        &self,
        message_id: &str,
        account_ids: &[&str],
    ) -> Result<(), BackendError>;
}

#[async_trait]
pub trait DurableEventStore: Send + Sync {
    async fn record(
        &self,
        topic: &str,
        topic_kind: &str,
        partition_key: &str,
        payload_json: &str,
        at_ms: i64,
    ) -> Result<TopicCursor, BackendError>;

    async fn read_after(
        &self,
        topic: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<DurableEventRow>, BackendError>;

    async fn retention_floor(&self, topic: &str) -> Result<i64, BackendError>;
}

#[async_trait]
pub trait OutboxRepository: Send + Sync {
    async fn enqueue(
        &self,
        topic_kind: &str,
        event_id: &str,
        available_at_ms: i64,
    ) -> Result<String, BackendError>;

    async fn claim(&self, worker_id: &str, batch: u32) -> Result<Vec<OutboxRow>, BackendError>;

    async fn ack(&self, outbox_id: &str, at_ms: i64) -> Result<bool, BackendError>;

    async fn retry(
        &self,
        outbox_id: &str,
        available_at_ms: i64,
        last_error: &str,
    ) -> Result<bool, BackendError>;

    async fn dead_letter(
        &self,
        outbox_id: &str,
        at_ms: i64,
        last_error: &str,
    ) -> Result<bool, BackendError>;
}

#[async_trait]
pub trait AuditRepository: Send + Sync {
    async fn insert(
        &self,
        actor_kind: &str,
        account_id: Option<&str>,
        installation_id: Option<&str>,
        event_type: &str,
        metadata: Option<&str>,
        at_ms: i64,
    ) -> Result<AuditRow, BackendError>;

    async fn list_since(
        &self,
        account_id: &str,
        since_ms: i64,
        limit: u32,
    ) -> Result<Vec<AuditRow>, BackendError>;
}

#[async_trait]
pub trait PushTokensRepository: Send + Sync {
    async fn upsert(
        &self,
        account_id: &str,
        installation_id: &str,
        kind: &str,
        token: &str,
        locale: Option<&str>,
        at_ms: i64,
    ) -> Result<PushTokenRow, BackendError>;

    async fn revoke(&self, token_hash: &str, at_ms: i64) -> Result<bool, BackendError>;

    async fn list_for_account(&self, account_id: &str) -> Result<Vec<PushTokenRow>, BackendError>;
}

// ---------------------------------------------------------------------------
// RepositorySet
// ---------------------------------------------------------------------------

pub struct RepositorySet {
    // -- existing fields (store-backed, wired in P1-P4) --
    pub agent_sessions: Arc<dyn AgentSessionsRepository>,
    pub agent_turns: Arc<dyn AgentTurnsRepository>,
    pub agent_turn_events: Arc<dyn AgentTurnEventsRepository>,
    pub approval_requests: Arc<dyn ApprovalRequestsRepository>,
    pub account_host_pairings: Arc<dyn AccountHostPairingsRepository>,

    // -- P0.S2 placeholders (stub impls, real store-backed impls in P1-P4) --
    pub accounts: Arc<dyn AccountsRepository>,
    pub account_credentials: Arc<dyn AccountCredentialsRepository>,
    pub installations: Arc<dyn InstallationsRepository>,
    pub refresh_tokens: Arc<dyn RefreshTokensRepository>,
    pub host_installation_tokens: Arc<dyn HostInstallationTokensRepository>,
    pub pairing_codes: Arc<dyn PairingCodesRepository>,
    pub host_links: Arc<dyn HostLinksRepository>,
    pub agents: Arc<dyn AgentsRepository>,
    pub projects: Arc<dyn ProjectsRepository>,
    pub conversations: Arc<dyn ConversationsRepository>,
    pub conversation_messages: Arc<dyn ConversationMessagesRepository>,
    pub durable_event_store: Arc<dyn DurableEventStore>,
    pub outbox: Arc<dyn OutboxRepository>,
    pub audit: Arc<dyn AuditRepository>,
    pub push_tokens: Arc<dyn PushTokensRepository>,
}

impl RepositorySet {
    #[must_use]
    pub fn from_store(store: StoreHandle) -> Self {
        Self {
            agent_sessions: Arc::new(StoreBackedAgentSessionsRepository {
                store: store.clone(),
            }),
            agent_turns: Arc::new(StoreBackedAgentTurnsRepository {
                store: store.clone(),
            }),
            agent_turn_events: Arc::new(StoreBackedAgentTurnEventsRepository {
                store: store.clone(),
            }),
            approval_requests: Arc::new(StoreBackedApprovalRequestsRepository {
                store: store.clone(),
            }),
            account_host_pairings: Arc::new(StoreBackedAccountHostPairingsRepository {
                store: store.clone(),
            }),
            accounts: Arc::new(StoreBackedAccountsRepository {
                store: store.clone(),
            }),
            account_credentials: Arc::new(StoreBackedAccountCredentialsRepository {
                store: store.clone(),
            }),
            installations: Arc::new(StoreBackedInstallationsRepository {
                store: store.clone(),
            }),
            refresh_tokens: Arc::new(StoreBackedRefreshTokensRepository {
                store: store.clone(),
            }),
            host_installation_tokens: Arc::new(StoreBackedHostInstallationTokensRepository {
                store: store.clone(),
            }),
            pairing_codes: Arc::new(StoreBackedPairingCodesRepository {
                store: store.clone(),
            }),
            host_links: Arc::new(StoreBackedHostLinksRepository {
                store: store.clone(),
            }),
            agents: Arc::new(StoreBackedAgentsRepository {
                store: store.clone(),
            }),
            projects: Arc::new(StoreBackedProjectsRepository {
                store: store.clone(),
            }),
            conversations: Arc::new(StoreBackedConversationsRepository {
                store: store.clone(),
            }),
            conversation_messages: Arc::new(StoreBackedConversationMessagesRepository {
                store: store.clone(),
            }),
            durable_event_store: Arc::new(StoreBackedDurableEventStore {
                store: store.clone(),
            }),
            outbox: Arc::new(StoreBackedOutboxRepository {
                store: store.clone(),
            }),
            audit: Arc::new(StoreBackedAuditRepository {
                store: store.clone(),
            }),
            push_tokens: Arc::new(StoreBackedPushTokensRepository { store }),
        }
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations (existing, preserved)
// ---------------------------------------------------------------------------

struct StoreBackedAgentSessionsRepository {
    store: StoreHandle,
}

#[async_trait]
impl AgentSessionsRepository for StoreBackedAgentSessionsRepository {
    async fn get_for_account(
        &self,
        session_id: &str,
        account_id: &str,
    ) -> Result<Option<store::agent_sessions::AgentSessionRow>, BackendError> {
        store::agent_sessions::get_for_account(&self.store, session_id, account_id).await
    }

    async fn list_for_account(
        &self,
        account_id: &str,
        before_started_at_ms: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_sessions::AgentSessionRow>, BackendError> {
        store::agent_sessions::list_for_account(
            &self.store,
            account_id,
            before_started_at_ms,
            limit,
        )
        .await
    }

    async fn list_for_account_conversation(
        &self,
        conversation_id: &str,
        account_id: &str,
        before_started_at_ms: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_sessions::AgentSessionRow>, BackendError> {
        store::agent_sessions::list_for_account_conversation(
            &self.store,
            conversation_id,
            account_id,
            before_started_at_ms,
            limit,
        )
        .await
    }

    async fn list_for_account_project(
        &self,
        project_id: &str,
        account_id: &str,
        before_started_at_ms: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_sessions::AgentSessionRow>, BackendError> {
        store::agent_sessions::list_for_account_project(
            &self.store,
            project_id,
            account_id,
            before_started_at_ms,
            limit,
        )
        .await
    }
}

struct StoreBackedAgentTurnsRepository {
    store: StoreHandle,
}

#[async_trait]
impl AgentTurnsRepository for StoreBackedAgentTurnsRepository {
    async fn get_for_account(
        &self,
        turn_id: &str,
        account_id: &str,
    ) -> Result<Option<store::agent_turns::AgentTurnRow>, BackendError> {
        store::agent_turns::get_for_account(&self.store, turn_id, account_id).await
    }

    async fn list_for_session(
        &self,
        session_id: &str,
        after_turn_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_turns::AgentTurnRow>, BackendError> {
        store::agent_turns::list_for_session(&self.store, session_id, after_turn_seq, limit).await
    }
}

struct StoreBackedAgentTurnEventsRepository {
    store: StoreHandle,
}

#[async_trait]
impl AgentTurnEventsRepository for StoreBackedAgentTurnEventsRepository {
    async fn list_for_turn(
        &self,
        turn_id: &str,
        after_event_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<store::agent_turn_events::AgentTurnEventRow>, BackendError> {
        store::agent_turn_events::list_for_turn(&self.store, turn_id, after_event_seq, limit).await
    }
}

struct StoreBackedApprovalRequestsRepository {
    store: StoreHandle,
}

#[async_trait]
impl ApprovalRequestsRepository for StoreBackedApprovalRequestsRepository {
    async fn get(
        &self,
        request_id: &str,
    ) -> Result<Option<store::approval_requests::ApprovalRequestRow>, BackendError> {
        store::approval_requests::get(&self.store, request_id).await
    }

    async fn resolve(
        &self,
        request_id: &str,
        state: store::approval_requests::ApprovalRequestState,
        resolved_at_ms: i64,
        resolution_json: Option<&Value>,
    ) -> Result<bool, BackendError> {
        store::approval_requests::resolve(
            &self.store,
            request_id,
            state,
            resolved_at_ms,
            resolution_json,
        )
        .await
    }
}

struct StoreBackedAccountHostPairingsRepository {
    store: StoreHandle,
}

#[async_trait]
impl AccountHostPairingsRepository for StoreBackedAccountHostPairingsRepository {
    async fn exists(
        &self,
        host_device_id: DeviceId,
        account_id: &str,
    ) -> Result<bool, BackendError> {
        store::host_links::exists(&self.store, host_device_id, account_id).await
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — AccountsRepository
// ---------------------------------------------------------------------------

fn convert_account_row(row: store::accounts::AccountRow) -> AccountRow {
    AccountRow {
        account_id: row.account_id,
        email: row.email,
        minos_id: Some(row.minos_id),
        display_name: row.display_name,
        created_at_ms: row.created_at,
        last_login_at_ms: row.last_login_at,
    }
}

struct StoreBackedAccountsRepository {
    store: StoreHandle,
}

#[async_trait]
impl AccountsRepository for StoreBackedAccountsRepository {
    async fn create(
        &self,
        email: &str,
        password_hash: &str,
        _display_name: Option<&str>,
        _at_ms: i64,
    ) -> Result<AccountRow, BackendError> {
        let row = store::accounts::create(&self.store, email, password_hash).await?;
        Ok(convert_account_row(row))
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<AccountRow>, BackendError> {
        Ok(store::accounts::find_by_email(&self.store, email)
            .await?
            .map(convert_account_row))
    }

    async fn find_by_id(&self, account_id: &str) -> Result<Option<AccountRow>, BackendError> {
        Ok(store::accounts::find_by_id(&self.store, account_id)
            .await?
            .map(convert_account_row))
    }

    async fn touch_last_login(&self, account_id: &str, _at_ms: i64) -> Result<(), BackendError> {
        store::accounts::touch_last_login(&self.store, account_id).await
    }

    async fn update_password_hash(
        &self,
        account_id: &str,
        hash: &str,
        _at_ms: i64,
    ) -> Result<(), BackendError> {
        store::accounts::set_password_hash(&self.store, account_id, hash).await
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — AccountCredentialsRepository
// ---------------------------------------------------------------------------

struct StoreBackedAccountCredentialsRepository {
    store: StoreHandle,
}

#[async_trait]
impl AccountCredentialsRepository for StoreBackedAccountCredentialsRepository {
    async fn upsert(
        &self,
        account_id: &str,
        password_hash: &str,
        _at_ms: i64,
    ) -> Result<CredentialRow, BackendError> {
        store::accounts::set_password_hash(&self.store, account_id, password_hash).await?;
        Ok(CredentialRow {
            account_id: account_id.to_string(),
            password_hash: password_hash.to_string(),
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        })
    }

    async fn find_by_account(
        &self,
        account_id: &str,
    ) -> Result<Option<CredentialRow>, BackendError> {
        match store::accounts::find_by_id(&self.store, account_id).await? {
            Some(row) => Ok(Some(CredentialRow {
                account_id: row.account_id,
                password_hash: row.password_hash,
                updated_at_ms: row.created_at,
            })),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — InstallationsRepository
// ---------------------------------------------------------------------------

fn convert_installation_row(row: store::device_installations::DeviceRow) -> InstallationRow {
    InstallationRow {
        installation_id: row.device_id.to_string(),
        // Storage vocabulary (mobile/browser/desktop/host), not wire role.
        kind: row.role.to_installation_kind().to_string(),
        platform: None,
        public_key: row.public_key,
        account_id: row.account_id,
        display_name: Some(row.display_name),
        created_at_ms: row.created_at,
        last_seen_at_ms: row.last_seen_at,
    }
}

struct StoreBackedInstallationsRepository {
    store: StoreHandle,
}

#[async_trait]
impl InstallationsRepository for StoreBackedInstallationsRepository {
    async fn upsert(
        &self,
        installation_id: &str,
        kind: &str,
        _platform: Option<&str>,
        public_key: Option<&str>,
        account_id: Option<&str>,
        display_name: Option<&str>,
        at_ms: i64,
    ) -> Result<InstallationRow, BackendError> {
        let device_id = Uuid::parse_str(installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "installation_id".into(),
                message: e.to_string(),
            })?;
        // Accept either installation_kind (`mobile`) or wire role (`mobile-client`).
        let role = minos_domain::DeviceRole::from_installation_kind(kind)
            .or_else(|_| kind.parse::<minos_domain::DeviceRole>())
            .map_err(|e| BackendError::StoreQuery {
                operation: "installations.upsert".into(),
                message: format!("invalid installation kind/role `{kind}`: {e}"),
            })?;
        let name = display_name.unwrap_or("device");

        if store::device_installations::get_device(&self.store, device_id)
            .await?
            .is_none()
        {
            // Postgres CHECK: clients need account_id; hosts need public_key.
            if role.is_account_client() {
                let account_id = account_id.ok_or_else(|| BackendError::StoreQuery {
                    operation: "installations.upsert".into(),
                    message: "account_id required for client installation kinds".into(),
                })?;
                store::device_installations::insert_client_for_account(
                    &self.store,
                    device_id,
                    name,
                    role,
                    account_id,
                    at_ms,
                )
                .await?;
            } else if role == minos_domain::DeviceRole::AgentHost {
                let public_key = public_key.ok_or_else(|| BackendError::StoreQuery {
                    operation: "installations.upsert".into(),
                    message: "public_key required for host installations".into(),
                })?;
                store::device_installations::insert_host_with_public_key(
                    &self.store,
                    device_id,
                    name,
                    public_key,
                    at_ms,
                )
                .await?;
            } else {
                return Err(BackendError::StoreQuery {
                    operation: "installations.upsert".into(),
                    message: format!("unsupported installation role: {role}"),
                });
            }
        } else {
            if let Some(pk) = public_key {
                let _ = store::device_installations::set_public_key_if_absent(
                    &self.store,
                    &device_id,
                    pk,
                )
                .await;
            }
            if let Some(aid) = account_id {
                if role.is_account_client() {
                    let _ =
                        store::device_installations::set_account_id(&self.store, &device_id, aid)
                            .await;
                }
            }
            if let Some(dn) = display_name {
                let _ = store::device_installations::set_display_name(&self.store, &device_id, dn)
                    .await;
            }
        }

        match store::device_installations::get_device(&self.store, device_id).await? {
            Some(row) => Ok(convert_installation_row(row)),
            None => Err(BackendError::StoreQuery {
                operation: "installations.upsert".into(),
                message: "device not found after upsert".into(),
            }),
        }
    }

    async fn find(&self, installation_id: &str) -> Result<Option<InstallationRow>, BackendError> {
        let device_id = Uuid::parse_str(installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "installation_id".into(),
                message: e.to_string(),
            })?;
        Ok(
            store::device_installations::get_device(&self.store, device_id)
                .await?
                .map(convert_installation_row),
        )
    }

    async fn touch_last_seen(&self, installation_id: &str, at_ms: i64) -> Result<(), BackendError> {
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE device_installations SET last_seen_at_ms = ? WHERE installation_id = ?",
                )
                .bind(at_ms)
                .bind(installation_id)
                .execute(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "installations.touch_last_seen".into(),
                    message: e.to_string(),
                })?;
            }
            StorePoolRef::Postgres(pool) => {
                sqlx::query(
                    "UPDATE device_installations SET last_seen_at_ms = $1 WHERE installation_id = $2",
                )
                .bind(at_ms)
                .bind(installation_id)
                .execute(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "installations.touch_last_seen".into(),
                    message: e.to_string(),
                })?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — RefreshTokensRepository
// ---------------------------------------------------------------------------

fn convert_refresh_token_row(row: store::refresh_tokens::RefreshTokenRow) -> RefreshTokenRow {
    RefreshTokenRow {
        token_hash: row.token_hash,
        account_id: row.account_id,
        installation_id: row.device_id,
        issued_at_ms: row.issued_at,
        expires_at_ms: row.expires_at,
        revoked_at_ms: row.revoked_at,
        rotated_to_hash: None,
    }
}

struct StoreBackedRefreshTokensRepository {
    store: StoreHandle,
}

#[async_trait]
impl RefreshTokensRepository for StoreBackedRefreshTokensRepository {
    async fn insert(
        &self,
        plaintext: &str,
        account_id: &str,
        installation_id: &str,
        _ttl_ms: i64,
        _at_ms: i64,
    ) -> Result<RefreshTokenRow, BackendError> {
        let row =
            store::refresh_tokens::insert(&self.store, plaintext, account_id, installation_id)
                .await?;
        Ok(convert_refresh_token_row(row))
    }

    async fn rotate(
        &self,
        old_plaintext: &str,
        new_plaintext: &str,
        _ttl_ms: i64,
        _at_ms: i64,
    ) -> Result<RefreshTokenRow, BackendError> {
        // Look up the old token to get account_id and device_id.
        let old = store::refresh_tokens::find_active(&self.store, old_plaintext)
            .await?
            .ok_or(BackendError::StoreQuery {
                operation: "refresh_tokens.rotate".into(),
                message: "old token not found or expired".into(),
            })?;
        match store::refresh_tokens::rotate(
            &self.store,
            old_plaintext,
            new_plaintext,
            &old.account_id,
            &old.device_id,
        )
        .await?
        {
            Some(row) => Ok(convert_refresh_token_row(row)),
            None => Err(BackendError::StoreQuery {
                operation: "refresh_tokens.rotate".into(),
                message: "old token already revoked".into(),
            }),
        }
    }

    async fn find_active(&self, plaintext: &str) -> Result<Option<RefreshTokenRow>, BackendError> {
        Ok(store::refresh_tokens::find_active(&self.store, plaintext)
            .await?
            .map(convert_refresh_token_row))
    }

    async fn revoke_all_for_account(
        &self,
        account_id: &str,
        _at_ms: i64,
    ) -> Result<u64, BackendError> {
        store::refresh_tokens::revoke_all_for_account(&self.store, account_id).await
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — HostInstallationTokensRepository
// ---------------------------------------------------------------------------

fn convert_host_token_row(
    row: store::host_installation_tokens::HostInstallationTokenRow,
) -> HostTokenRow {
    HostTokenRow {
        token_hash: row.token_hash,
        host_installation_id: row.host_installation_id.to_string(),
        issued_at_ms: row.issued_at_ms,
        last_used_at_ms: row.last_used_at_ms,
        revoked_at_ms: row.revoked_at_ms,
    }
}

struct StoreBackedHostInstallationTokensRepository {
    store: StoreHandle,
}

#[async_trait]
impl HostInstallationTokensRepository for StoreBackedHostInstallationTokensRepository {
    async fn insert(&self, host_installation_id: &str, at_ms: i64) -> Result<String, BackendError> {
        let device_id = Uuid::parse_str(host_installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_installation_id".into(),
                message: e.to_string(),
            })?;
        // Generate a random 32-byte token and hash it.
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
        let plaintext = hex_encode(&bytes);
        let token_hash = Sha256::digest(plaintext.as_bytes());
        let token_hash_hex = hex_encode(&token_hash);

        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                store::host_installation_tokens::insert_token_with_executor(
                    pool,
                    &token_hash_hex,
                    device_id,
                    at_ms,
                )
                .await?;
            }
            StorePoolRef::Postgres(pool) => {
                store::host_installation_tokens::insert_token_with_postgres_executor(
                    pool,
                    &token_hash_hex,
                    device_id,
                    at_ms,
                )
                .await?;
            }
        }
        Ok(plaintext)
    }

    async fn find_active(&self, token_hash: &str) -> Result<Option<HostTokenRow>, BackendError> {
        // verify_active_token updates last_used_at_ms as a side effect,
        // which is acceptable for the host authentication flow.
        let now_ms = chrono::Utc::now().timestamp_millis();
        Ok(
            store::host_installation_tokens::verify_active_token(&self.store, token_hash, now_ms)
                .await?
                .map(convert_host_token_row),
        )
    }

    async fn revoke(&self, token_hash: &str, at_ms: i64) -> Result<bool, BackendError> {
        let affected = match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => sqlx::query(
                "UPDATE host_installation_tokens SET revoked_at_ms = ? \
                     WHERE token_hash = ? AND revoked_at_ms IS NULL",
            )
            .bind(at_ms)
            .bind(token_hash)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
            StorePoolRef::Postgres(pool) => sqlx::query(
                "UPDATE host_installation_tokens SET revoked_at_ms = $1 \
                     WHERE token_hash = $2 AND revoked_at_ms IS NULL",
            )
            .bind(at_ms)
            .bind(token_hash)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
        };
        affected
            .map_err(|e| BackendError::StoreQuery {
                operation: "host_installation_tokens.revoke".into(),
                message: e.to_string(),
            })
            .map(|n| n == 1)
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — PairingCodesRepository
// ---------------------------------------------------------------------------

fn convert_pairing_code_row(row: store::pairing_codes::PairingCodeRow) -> PairingCodeRow {
    PairingCodeRow {
        code_hash: row.code_hash,
        host_installation_id: row.host_installation_id.to_string(),
        account_id: row.account_id,
        linked_via_installation_id: row.linked_via_installation_id.map(|id| id.to_string()),
        status: row.status.as_str().to_string(),
        client_request_id: row.client_request_id,
        created_at_ms: row.created_at_ms,
        expires_at_ms: row.expires_at_ms,
        confirmed_at_ms: row.confirmed_at_ms,
        redeemed_at_ms: row.redeemed_at_ms,
    }
}

struct StoreBackedPairingCodesRepository {
    store: StoreHandle,
}

#[async_trait]
impl PairingCodesRepository for StoreBackedPairingCodesRepository {
    async fn insert(
        &self,
        code_hash: &str,
        host_installation_id: &str,
        ttl_ms: i64,
        client_request_id: Option<&str>,
        at_ms: i64,
    ) -> Result<PairingCodeRow, BackendError> {
        let device_id = Uuid::parse_str(host_installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_installation_id".into(),
                message: e.to_string(),
            })?;
        let expires_at_ms = at_ms + ttl_ms;
        store::pairing_codes::insert_code(&self.store, code_hash, device_id, at_ms, expires_at_ms)
            .await?;
        // Return the constructed row; client_request_id is set later during confirm.
        Ok(PairingCodeRow {
            code_hash: code_hash.to_string(),
            host_installation_id: host_installation_id.to_string(),
            account_id: None,
            linked_via_installation_id: None,
            status: "pending".to_string(),
            client_request_id: client_request_id.map(str::to_string),
            created_at_ms: at_ms,
            expires_at_ms,
            confirmed_at_ms: None,
            redeemed_at_ms: None,
        })
    }

    async fn find_by_code(&self, code_hash: &str) -> Result<Option<PairingCodeRow>, BackendError> {
        let row = match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                store::pairing_codes::get_code_with_executor(pool, code_hash).await
            }
            StorePoolRef::Postgres(pool) => {
                store::pairing_codes::get_code_with_postgres_executor(pool, code_hash).await
            }
        }?;
        Ok(row.map(convert_pairing_code_row))
    }

    async fn update_status(
        &self,
        code_hash: &str,
        new_status: &str,
        at_ms: i64,
    ) -> Result<bool, BackendError> {
        let result = match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE pairing_codes SET status = ?, \
                     confirmed_at_ms = CASE WHEN ? = 'confirmed' THEN COALESCE(confirmed_at_ms, ?) ELSE confirmed_at_ms END, \
                     redeemed_at_ms = CASE WHEN ? = 'redeemed' THEN COALESCE(redeemed_at_ms, ?) ELSE redeemed_at_ms END \
                     WHERE code_hash = ?",
                )
                .bind(new_status)
                .bind(new_status)
                .bind(at_ms)
                .bind(new_status)
                .bind(at_ms)
                .bind(code_hash)
                .execute(pool)
                .await
                .map(|r| r.rows_affected())
            }
            StorePoolRef::Postgres(pool) => {
                sqlx::query(
                    "UPDATE pairing_codes SET status = $1, \
                     confirmed_at_ms = CASE WHEN $1 = 'confirmed' THEN COALESCE(confirmed_at_ms, $2) ELSE confirmed_at_ms END, \
                     redeemed_at_ms = CASE WHEN $1 = 'redeemed' THEN COALESCE(redeemed_at_ms, $2) ELSE redeemed_at_ms END \
                     WHERE code_hash = $3",
                )
                .bind(new_status)
                .bind(at_ms)
                .bind(code_hash)
                .execute(pool)
                .await
                .map(|r| r.rows_affected())
            }
        }
        .map_err(|e| BackendError::StoreQuery {
            operation: "pairing_codes.update_status".into(),
            message: e.to_string(),
        })?;
        Ok(result == 1)
    }

    async fn expire_stale(&self, now_ms: i64) -> Result<u64, BackendError> {
        let result = match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => sqlx::query(
                "UPDATE pairing_codes SET status = 'expired' \
                     WHERE status = 'pending' AND expires_at_ms < ?",
            )
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
            StorePoolRef::Postgres(pool) => sqlx::query(
                "UPDATE pairing_codes SET status = 'expired' \
                     WHERE status = 'pending' AND expires_at_ms < $1",
            )
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
        }
        .map_err(|e| BackendError::StoreQuery {
            operation: "pairing_codes.expire_stale".into(),
            message: e.to_string(),
        })?;
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — HostLinksRepository
// ---------------------------------------------------------------------------

fn convert_pair_row_to_host_link(row: store::host_links::PairRow, acl_json: &str) -> HostLinkRow {
    HostLinkRow {
        pair_id: row.pair_id,
        account_id: row.mobile_account_id,
        host_installation_id: row.host_device_id.to_string(),
        linked_via_installation_id: row.paired_via_device_id.to_string(),
        link_display_name: None,
        acl_json: acl_json.to_string(),
        paired_at_ms: row.paired_at_ms,
    }
}

struct StoreBackedHostLinksRepository {
    store: StoreHandle,
}

#[async_trait]
impl HostLinksRepository for StoreBackedHostLinksRepository {
    async fn insert(
        &self,
        account_id: &str,
        host_installation_id: &str,
        linked_via: &str,
        _display_name: Option<&str>,
        acl_json: &str,
        at_ms: i64,
    ) -> Result<HostLinkRow, BackendError> {
        let host_id = Uuid::parse_str(host_installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_installation_id".into(),
                message: e.to_string(),
            })?;
        let via_id =
            Uuid::parse_str(linked_via)
                .map(DeviceId)
                .map_err(|e| BackendError::StoreDecode {
                    column: "linked_via".into(),
                    message: e.to_string(),
                })?;
        store::host_links::insert_pair(&self.store, host_id, account_id, via_id, at_ms).await?;
        Ok(HostLinkRow {
            pair_id: Uuid::new_v4().to_string(),
            account_id: account_id.to_string(),
            host_installation_id: host_installation_id.to_string(),
            linked_via_installation_id: linked_via.to_string(),
            link_display_name: None,
            acl_json: acl_json.to_string(),
            paired_at_ms: at_ms,
        })
    }

    async fn exists(
        &self,
        account_id: &str,
        host_installation_id: &str,
    ) -> Result<bool, BackendError> {
        let host_id = Uuid::parse_str(host_installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_installation_id".into(),
                message: e.to_string(),
            })?;
        store::host_links::exists(&self.store, host_id, account_id).await
    }

    async fn list_for_account(&self, account_id: &str) -> Result<Vec<HostLinkRow>, BackendError> {
        let rows = store::host_links::list_hosts_for_account(&self.store, account_id).await?;
        Ok(rows
            .into_iter()
            .map(|r| convert_pair_row_to_host_link(r, "{}"))
            .collect())
    }

    async fn list_for_host(
        &self,
        host_installation_id: &str,
    ) -> Result<Vec<HostLinkRow>, BackendError> {
        let host_id = Uuid::parse_str(host_installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_installation_id".into(),
                message: e.to_string(),
            })?;
        let rows = store::host_links::list_accounts_for_host(&self.store, host_id).await?;
        Ok(rows
            .into_iter()
            .map(|r| convert_pair_row_to_host_link(r, "{}"))
            .collect())
    }

    async fn pick_default_host(&self, account_id: &str) -> Result<Option<String>, BackendError> {
        let rows = store::host_links::list_hosts_for_account(&self.store, account_id).await?;
        Ok(rows.first().map(|r| r.host_device_id.to_string()))
    }

    async fn remove(
        &self,
        account_id: &str,
        host_installation_id: &str,
    ) -> Result<bool, BackendError> {
        let host_id = Uuid::parse_str(host_installation_id)
            .map(DeviceId)
            .map_err(|e| BackendError::StoreDecode {
                column: "host_installation_id".into(),
                message: e.to_string(),
            })?;
        let deleted = store::host_links::delete_pair(&self.store, host_id, account_id).await?;
        Ok(deleted == 1)
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — AgentsRepository
//
// The agents table schema differs between SQLite (user-created agents with
// owner_account_id, name, runtime_agent) and Postgres (system-defined agents
// with runtime_kind, display_name, enabled). The adapter handles both.
// ---------------------------------------------------------------------------

struct StoreBackedAgentsRepository {
    store: StoreHandle,
}

#[async_trait]
impl AgentsRepository for StoreBackedAgentsRepository {
    async fn list_enabled(&self) -> Result<Vec<AgentRow>, BackendError> {
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let rows = sqlx::query_as::<_, (String, String, String, Option<String>, i64)>(
                    "SELECT agent_id, runtime_agent, name, description, created_at_ms \
                     FROM agents ORDER BY created_at_ms ASC",
                )
                .fetch_all(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "agents.list_enabled".into(),
                    message: e.to_string(),
                })?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(agent_id, runtime_kind, display_name, description, created_at_ms)| {
                            AgentRow {
                                agent_id,
                                runtime_kind,
                                display_name,
                                description,
                                enabled: true,
                                created_at_ms,
                            }
                        },
                    )
                    .collect())
            }
            StorePoolRef::Postgres(pool) => {
                let rows = sqlx::query_as::<
                    _,
                    (String, String, String, Option<String>, bool, i64),
                >(
                    "SELECT agent_id, runtime_kind, display_name, description, enabled, created_at_ms \
                     FROM agents WHERE enabled = true ORDER BY created_at_ms ASC",
                )
                .fetch_all(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "agents.list_enabled".into(),
                    message: e.to_string(),
                })?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            agent_id,
                            runtime_kind,
                            display_name,
                            description,
                            enabled,
                            created_at_ms,
                        )| {
                            AgentRow {
                                agent_id,
                                runtime_kind,
                                display_name,
                                description,
                                enabled,
                                created_at_ms,
                            }
                        },
                    )
                    .collect())
            }
        }
    }

    async fn find(&self, agent_id: &str) -> Result<Option<AgentRow>, BackendError> {
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, Option<String>, i64)>(
                    "SELECT agent_id, runtime_agent, name, description, created_at_ms \
                     FROM agents WHERE agent_id = ?",
                )
                .bind(agent_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "agents.find".into(),
                    message: e.to_string(),
                })?;
                Ok(row.map(
                    |(agent_id, runtime_kind, display_name, description, created_at_ms)| AgentRow {
                        agent_id,
                        runtime_kind,
                        display_name,
                        description,
                        enabled: true,
                        created_at_ms,
                    },
                ))
            }
            StorePoolRef::Postgres(pool) => {
                let row = sqlx::query_as::<
                    _,
                    (String, String, String, Option<String>, bool, i64),
                >(
                    "SELECT agent_id, runtime_kind, display_name, description, enabled, created_at_ms \
                     FROM agents WHERE agent_id = $1",
                )
                .bind(agent_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "agents.find".into(),
                    message: e.to_string(),
                })?;
                Ok(row.map(
                    |(
                        agent_id,
                        runtime_kind,
                        display_name,
                        description,
                        enabled,
                        created_at_ms,
                    )| {
                        AgentRow {
                            agent_id,
                            runtime_kind,
                            display_name,
                            description,
                            enabled,
                            created_at_ms,
                        }
                    },
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — ProjectsRepository
// ---------------------------------------------------------------------------

struct StoreBackedProjectsRepository {
    store: StoreHandle,
}

#[async_trait]
impl ProjectsRepository for StoreBackedProjectsRepository {
    async fn create(
        &self,
        account_id: &str,
        name: &str,
        workspace_root: &str,
        at_ms: i64,
    ) -> Result<ProjectRow, BackendError> {
        let project_id = Uuid::new_v4().to_string();
        store::projects::create(
            &self.store,
            &project_id,
            account_id,
            name,
            workspace_root,
            None,
            at_ms,
        )
        .await?;
        Ok(ProjectRow {
            project_id,
            account_id: account_id.to_string(),
            name: name.to_string(),
            workspace_root: workspace_root.to_string(),
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
            archived_at_ms: None,
        })
    }

    async fn find(&self, project_id: &str) -> Result<Option<ProjectRow>, BackendError> {
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let row = sqlx::query_as::<
                    _,
                    (String, String, String, String, i64, i64, Option<i64>),
                >(
                    "SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms \
                     FROM projects WHERE project_id = ?",
                )
                .bind(project_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "projects.find".into(),
                    message: e.to_string(),
                })?;
                Ok(row.map(
                    |(
                        project_id,
                        account_id,
                        name,
                        workspace_root,
                        created_at_ms,
                        updated_at_ms,
                        archived_at_ms,
                    )| {
                        ProjectRow {
                            project_id,
                            account_id,
                            name,
                            workspace_root,
                            created_at_ms,
                            updated_at_ms,
                            archived_at_ms,
                        }
                    },
                ))
            }
            StorePoolRef::Postgres(pool) => {
                let row = sqlx::query_as::<
                    _,
                    (String, String, String, String, i64, i64, Option<i64>),
                >(
                    "SELECT project_id, account_id, name, workspace_root, created_at_ms, updated_at_ms, archived_at_ms \
                     FROM projects WHERE project_id = $1",
                )
                .bind(project_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "projects.find".into(),
                    message: e.to_string(),
                })?;
                Ok(row.map(
                    |(
                        project_id,
                        account_id,
                        name,
                        workspace_root,
                        created_at_ms,
                        updated_at_ms,
                        archived_at_ms,
                    )| {
                        ProjectRow {
                            project_id,
                            account_id,
                            name,
                            workspace_root,
                            created_at_ms,
                            updated_at_ms,
                            archived_at_ms,
                        }
                    },
                ))
            }
        }
    }

    async fn list_for_account(
        &self,
        account_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<ProjectRow>, BackendError> {
        let effective_limit = i64::from(limit.min(200));
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let rows = match cursor {
                    Some(cursor_id) => {
                        sqlx::query_as::<
                            _,
                            (String, String, String, String, i64, i64, Option<i64>),
                        >(
                            "SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms \
                             FROM projects WHERE account_id = ? AND project_id > ? \
                             ORDER BY project_id ASC LIMIT ?",
                        )
                        .bind(account_id)
                        .bind(cursor_id)
                        .bind(effective_limit)
                        .fetch_all(pool)
                        .await
                    }
                    None => {
                        sqlx::query_as::<
                            _,
                            (String, String, String, String, i64, i64, Option<i64>),
                        >(
                            "SELECT project_id, account_id, name, workspace_slug, created_at_ms, updated_at_ms, archived_at_ms \
                             FROM projects WHERE account_id = ? \
                             ORDER BY project_id ASC LIMIT ?",
                        )
                        .bind(account_id)
                        .bind(effective_limit)
                        .fetch_all(pool)
                        .await
                    }
                }
                .map_err(|e| BackendError::StoreQuery {
                    operation: "projects.list_for_account".into(),
                    message: e.to_string(),
                })?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            project_id,
                            account_id,
                            name,
                            workspace_root,
                            created_at_ms,
                            updated_at_ms,
                            archived_at_ms,
                        )| {
                            ProjectRow {
                                project_id,
                                account_id,
                                name,
                                workspace_root,
                                created_at_ms,
                                updated_at_ms,
                                archived_at_ms,
                            }
                        },
                    )
                    .collect())
            }
            StorePoolRef::Postgres(pool) => {
                let rows = match cursor {
                    Some(cursor_id) => {
                        sqlx::query_as::<
                            _,
                            (String, String, String, String, i64, i64, Option<i64>),
                        >(
                            "SELECT project_id, account_id, name, workspace_root, created_at_ms, updated_at_ms, archived_at_ms \
                             FROM projects WHERE account_id = $1 AND project_id > $2 \
                             ORDER BY project_id ASC LIMIT $3",
                        )
                        .bind(account_id)
                        .bind(cursor_id)
                        .bind(effective_limit)
                        .fetch_all(pool)
                        .await
                    }
                    None => {
                        sqlx::query_as::<
                            _,
                            (String, String, String, String, i64, i64, Option<i64>),
                        >(
                            "SELECT project_id, account_id, name, workspace_root, created_at_ms, updated_at_ms, archived_at_ms \
                             FROM projects WHERE account_id = $1 \
                             ORDER BY project_id ASC LIMIT $2",
                        )
                        .bind(account_id)
                        .bind(effective_limit)
                        .fetch_all(pool)
                        .await
                    }
                }
                .map_err(|e| BackendError::StoreQuery {
                    operation: "projects.list_for_account".into(),
                    message: e.to_string(),
                })?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            project_id,
                            account_id,
                            name,
                            workspace_root,
                            created_at_ms,
                            updated_at_ms,
                            archived_at_ms,
                        )| {
                            ProjectRow {
                                project_id,
                                account_id,
                                name,
                                workspace_root,
                                created_at_ms,
                                updated_at_ms,
                                archived_at_ms,
                            }
                        },
                    )
                    .collect())
            }
        }
    }

    async fn update(
        &self,
        project_id: &str,
        name: Option<&str>,
        workspace_root: Option<&str>,
        at_ms: i64,
    ) -> Result<ProjectRow, BackendError> {
        // Fetch existing first to fill in unchanged fields.
        let existing = self
            .find(project_id)
            .await?
            .ok_or(BackendError::StoreQuery {
                operation: "projects.update".into(),
                message: "project not found".into(),
            })?;
        let new_name = name.unwrap_or(&existing.name);
        let ws_col = if self.store.is_sqlite() {
            "workspace_slug"
        } else {
            "workspace_root"
        };
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                sqlx::query(&format!(
                    "UPDATE projects SET name = ?, {ws_col} = ?, updated_at_ms = ? \
                     WHERE project_id = ? AND account_id = ?"
                ))
                .bind(new_name)
                .bind(workspace_root.unwrap_or(&existing.workspace_root))
                .bind(at_ms)
                .bind(project_id)
                .bind(&existing.account_id)
                .execute(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "projects.update".into(),
                    message: e.to_string(),
                })?;
            }
            StorePoolRef::Postgres(pool) => {
                sqlx::query(&format!(
                    "UPDATE projects SET name = $1, {ws_col} = $2, updated_at_ms = $3 \
                     WHERE project_id = $4 AND account_id = $5"
                ))
                .bind(new_name)
                .bind(workspace_root.unwrap_or(&existing.workspace_root))
                .bind(at_ms)
                .bind(project_id)
                .bind(&existing.account_id)
                .execute(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "projects.update".into(),
                    message: e.to_string(),
                })?;
            }
        }
        // Return updated row.
        Ok(ProjectRow {
            project_id: project_id.to_string(),
            account_id: existing.account_id,
            name: new_name.to_string(),
            workspace_root: workspace_root
                .unwrap_or(&existing.workspace_root)
                .to_string(),
            created_at_ms: existing.created_at_ms,
            updated_at_ms: at_ms,
            archived_at_ms: existing.archived_at_ms,
        })
    }

    async fn archive(&self, project_id: &str, at_ms: i64) -> Result<bool, BackendError> {
        let result = match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => sqlx::query(
                "UPDATE projects SET archived_at_ms = ? \
                     WHERE project_id = ? AND archived_at_ms IS NULL",
            )
            .bind(at_ms)
            .bind(project_id)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
            StorePoolRef::Postgres(pool) => sqlx::query(
                "UPDATE projects SET archived_at_ms = $1 \
                     WHERE project_id = $2 AND archived_at_ms IS NULL",
            )
            .bind(at_ms)
            .bind(project_id)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
        }
        .map_err(|e| BackendError::StoreQuery {
            operation: "projects.archive".into(),
            message: e.to_string(),
        })?;
        Ok(result == 1)
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — ConversationsRepository
// ---------------------------------------------------------------------------

fn convert_social_conversation(row: store::social::ConversationRow) -> ConversationRow {
    ConversationRow {
        conversation_id: row.conversation_id,
        kind: row.kind,
        title: row.title,
        project_id: None,
        created_by_account_id: row.created_by_account_id,
        direct_account_low: row.direct_account_low,
        direct_account_high: row.direct_account_high,
        created_at_ms: row.created_at_ms,
        updated_at_ms: row.updated_at_ms,
    }
}

struct StoreBackedConversationsRepository {
    store: StoreHandle,
}

#[async_trait]
impl ConversationsRepository for StoreBackedConversationsRepository {
    async fn create(
        &self,
        kind: &str,
        created_by_account_id: &str,
        title: Option<&str>,
        _project_id: Option<&str>,
        at_ms: i64,
    ) -> Result<ConversationRow, BackendError> {
        let row = match kind {
            "direct" => {
                // For direct conversations, we need two members. Use the creator
                // as both sides (caller should use ensure_direct_conversation for
                // real direct convos). This is a fallback for the generic create.
                store::social::ensure_direct_conversation(
                    &self.store,
                    created_by_account_id,
                    created_by_account_id,
                    created_by_account_id,
                    at_ms,
                )
                .await?
            }
            _ => {
                store::social::create_group_conversation(
                    &self.store,
                    created_by_account_id,
                    title.unwrap_or(""),
                    &[],
                    at_ms,
                )
                .await?
            }
        };
        Ok(convert_social_conversation(row))
    }

    async fn find(&self, conversation_id: &str) -> Result<Option<ConversationRow>, BackendError> {
        Ok(
            store::social::get_conversation(&self.store, conversation_id)
                .await?
                .map(convert_social_conversation),
        )
    }

    async fn find_direct(
        &self,
        account_low: &str,
        account_high: &str,
    ) -> Result<Option<ConversationRow>, BackendError> {
        // ensure_direct_conversation creates if not found; we just want to find.
        // Use the internal find_direct_conversation via raw query.
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                sqlx::query_as::<_, store::social::ConversationRow>(
                    "SELECT conversation_id, kind, title, created_by_account_id, \
                     direct_account_low, direct_account_high, created_at_ms, updated_at_ms \
                     FROM conversations WHERE kind = 'direct' \
                     AND direct_account_low = ? AND direct_account_high = ?",
                )
                .bind(account_low)
                .bind(account_high)
                .fetch_optional(pool)
                .await
            }
            StorePoolRef::Postgres(pool) => {
                sqlx::query_as::<_, store::social::ConversationRow>(
                    "SELECT conversation_id, kind, title, created_by_account_id, \
                     direct_account_low, direct_account_high, created_at_ms, updated_at_ms \
                     FROM conversations WHERE kind = 'direct' \
                     AND direct_account_low = $1 AND direct_account_high = $2",
                )
                .bind(account_low)
                .bind(account_high)
                .fetch_optional(pool)
                .await
            }
        }
        .map_err(|e| BackendError::StoreQuery {
            operation: "conversations.find_direct".into(),
            message: e.to_string(),
        })
        .map(|opt| opt.map(convert_social_conversation))
    }

    async fn list_for_account(
        &self,
        account_id: &str,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<Vec<ConversationRow>, BackendError> {
        let digests = store::social::list_conversations_for(&self.store, account_id).await?;
        Ok(digests
            .into_iter()
            .map(|d| ConversationRow {
                conversation_id: d.conversation_id,
                kind: d.kind,
                title: d.title,
                project_id: None,
                created_by_account_id: d.created_by_account_id,
                direct_account_low: d.direct_account_low,
                direct_account_high: d.direct_account_high,
                created_at_ms: d.created_at_ms,
                updated_at_ms: d.updated_at_ms,
            })
            .collect())
    }

    async fn is_member(
        &self,
        conversation_id: &str,
        account_id: &str,
    ) -> Result<bool, BackendError> {
        store::social::is_conversation_member(&self.store, conversation_id, account_id).await
    }

    async fn project_id(&self, _conversation_id: &str) -> Result<Option<String>, BackendError> {
        // The social store doesn't track project_id on conversations.
        // This field is used by the agent session pipeline.
        Ok(None)
    }

    async fn update_at(&self, conversation_id: &str, at_ms: i64) -> Result<(), BackendError> {
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                sqlx::query("UPDATE conversations SET updated_at_ms = ? WHERE conversation_id = ?")
                    .bind(at_ms)
                    .bind(conversation_id)
                    .execute(pool)
                    .await
                    .map_err(|e| BackendError::StoreQuery {
                        operation: "conversations.update_at".into(),
                        message: e.to_string(),
                    })?;
            }
            StorePoolRef::Postgres(pool) => {
                sqlx::query(
                    "UPDATE conversations SET updated_at_ms = $1 WHERE conversation_id = $2",
                )
                .bind(at_ms)
                .bind(conversation_id)
                .execute(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "conversations.update_at".into(),
                    message: e.to_string(),
                })?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — ConversationMessagesRepository
// ---------------------------------------------------------------------------

fn convert_chat_message(row: store::social::ChatMessageRow) -> MessageRow {
    MessageRow {
        message_id: row.message_id,
        conversation_id: row.conversation_id,
        sender_kind: row.sender_type,
        sender_account_id: Some(row.sender_account_id),
        sender_agent_id: row.sender_agent_id,
        body_json: serde_json::json!({ "text": row.text }).to_string(),
        reply_to_message_id: row.reply_to_message_id,
        agent_session_id: None,
        created_at_ms: row.created_at_ms,
        recalled_at_ms: row.recalled_at_ms,
    }
}

struct StoreBackedConversationMessagesRepository {
    store: StoreHandle,
}

#[async_trait]
impl ConversationMessagesRepository for StoreBackedConversationMessagesRepository {
    async fn insert(
        &self,
        conversation_id: &str,
        sender_kind: &str,
        sender_account_id: Option<&str>,
        sender_agent_id: Option<&str>,
        body_json: &str,
        reply_to_message_id: Option<&str>,
        _agent_session_id: Option<&str>,
        at_ms: i64,
    ) -> Result<MessageRow, BackendError> {
        // Extract text from body_json if possible.
        let text = serde_json::from_str::<serde_json::Value>(body_json)
            .ok()
            .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .unwrap_or_else(|| body_json.to_string());

        if sender_kind == "agent" {
            let agent_id = sender_agent_id.unwrap_or("unknown");
            let row = store::social::insert_agent_message(
                &self.store,
                conversation_id,
                agent_id,
                &text,
                at_ms,
                reply_to_message_id,
                &[],
            )
            .await?;
            Ok(MessageRow {
                message_id: row.message_id,
                conversation_id: row.conversation_id,
                sender_kind: row.sender_type,
                sender_account_id: Some(row.sender_account_id),
                sender_agent_id: row.sender_agent_id,
                body_json: serde_json::json!({ "text": row.text }).to_string(),
                reply_to_message_id: row.reply_to_message_id,
                agent_session_id: None,
                created_at_ms: row.created_at_ms,
                recalled_at_ms: row.recalled_at_ms,
            })
        } else {
            let account_id = sender_account_id.unwrap_or("unknown");
            let row = store::social::insert_message(
                &self.store,
                conversation_id,
                account_id,
                &text,
                at_ms,
                reply_to_message_id,
                &[],
            )
            .await?;
            Ok(convert_chat_message(row))
        }
    }

    async fn find(&self, message_id: &str) -> Result<Option<MessageRow>, BackendError> {
        Ok(store::social::get_message(&self.store, message_id)
            .await?
            .map(convert_chat_message))
    }

    async fn list_for_conversation(
        &self,
        conversation_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<MessageRow>, BackendError> {
        let before_ts = cursor.and_then(|c| c.parse::<i64>().ok());
        let rows =
            store::social::list_messages(&self.store, conversation_id, before_ts, limit).await?;
        Ok(rows.into_iter().map(convert_chat_message).collect())
    }

    async fn recall(&self, message_id: &str, at_ms: i64) -> Result<bool, BackendError> {
        // Look up the message to get conversation_id and sender_account_id.
        let msg = store::social::get_message(&self.store, message_id)
            .await?
            .ok_or(BackendError::StoreQuery {
                operation: "conversation_messages.recall".into(),
                message: "message not found".into(),
            })?;
        let result = store::social::recall_message(
            &self.store,
            &msg.conversation_id,
            message_id,
            &msg.sender_account_id,
            at_ms,
        )
        .await?;
        Ok(result.is_some())
    }

    async fn insert_mentions(
        &self,
        message_id: &str,
        account_ids: &[&str],
    ) -> Result<(), BackendError> {
        // The social store's insert_message handles mentions inline.
        // For standalone mention insertion, use raw SQL.
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                for account_id in account_ids {
                    sqlx::query(
                        "INSERT OR IGNORE INTO chat_message_mentions \
                         (message_id, mentioned_account_id) VALUES (?, ?)",
                    )
                    .bind(message_id)
                    .bind(account_id)
                    .execute(pool)
                    .await
                    .map_err(|e| BackendError::StoreQuery {
                        operation: "conversation_messages.insert_mentions".into(),
                        message: e.to_string(),
                    })?;
                }
            }
            StorePoolRef::Postgres(pool) => {
                for account_id in account_ids {
                    sqlx::query(
                        "INSERT INTO chat_message_mentions \
                         (message_id, mentioned_account_id) VALUES ($1, $2) \
                         ON CONFLICT DO NOTHING",
                    )
                    .bind(message_id)
                    .bind(account_id)
                    .execute(pool)
                    .await
                    .map_err(|e| BackendError::StoreQuery {
                        operation: "conversation_messages.insert_mentions".into(),
                        message: e.to_string(),
                    })?;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — DurableEventStore
//
// record() opens its own transaction for topic-level seq serialization.
// On Postgres, pg_advisory_xact_lock provides topic-level exclusive access.
// On SQLite, the WAL-mode write lock serializes naturally.
// ---------------------------------------------------------------------------

fn convert_durable_event_row(row: store::durable_event_log::DurableEventRow) -> DurableEventRow {
    DurableEventRow {
        event_id: row.event_id,
        topic: row.topic,
        topic_kind: row.topic_kind,
        topic_seq: row.topic_seq,
        partition_key: row.partition_key,
        payload_json: row.payload_json.to_string(),
        created_at_ms: row.created_at_ms,
    }
}

struct StoreBackedDurableEventStore {
    store: StoreHandle,
}

#[async_trait]
impl DurableEventStore for StoreBackedDurableEventStore {
    async fn record(
        &self,
        topic: &str,
        topic_kind: &str,
        partition_key: &str,
        payload_json: &str,
        at_ms: i64,
    ) -> Result<TopicCursor, BackendError> {
        let event_id = Uuid::new_v4().to_string();
        let mut tx = self.store.begin().await?;

        let topic_seq = match &mut tx {
            DbTx::Sqlite(tx) => {
                let next_seq = sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(MAX(topic_seq), 0) + 1 \
                     FROM durable_event_log \
                     WHERE topic_kind = ? AND topic = ?",
                )
                .bind(topic_kind)
                .bind(topic)
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "durable_event_store.record.next_seq".into(),
                    message: e.to_string(),
                })?;

                sqlx::query(
                    "INSERT INTO durable_event_log \
                     (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&event_id)
                .bind(topic)
                .bind(topic_kind)
                .bind(next_seq)
                .bind(partition_key)
                .bind(payload_json)
                .bind(at_ms)
                .execute(&mut **tx)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "durable_event_store.record.insert".into(),
                    message: e.to_string(),
                })?;
                next_seq
            }
            DbTx::Postgres(tx) => {
                // Advisory lock for topic-level serialization.
                sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
                    .bind(topic)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| BackendError::StoreQuery {
                        operation: "durable_event_store.record.lock".into(),
                        message: e.to_string(),
                    })?;

                let next_seq = sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(MAX(topic_seq), 0) + 1 \
                     FROM durable_event_log \
                     WHERE topic_kind = $1 AND topic = $2",
                )
                .bind(topic_kind)
                .bind(topic)
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "durable_event_store.record.next_seq".into(),
                    message: e.to_string(),
                })?;

                sqlx::query(
                    "INSERT INTO durable_event_log \
                     (event_id, topic, topic_kind, topic_seq, partition_key, payload_json, created_at_ms) \
                     VALUES ($1, $2, $3, $4, $5, CAST($6 AS JSONB), $7)",
                )
                .bind(&event_id)
                .bind(topic)
                .bind(topic_kind)
                .bind(next_seq)
                .bind(partition_key)
                .bind(payload_json)
                .bind(at_ms)
                .execute(&mut **tx)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "durable_event_store.record.insert".into(),
                    message: e.to_string(),
                })?;
                next_seq
            }
        };

        tx.commit().await?;

        Ok(TopicCursor {
            topic: topic.to_string(),
            topic_seq,
        })
    }

    async fn read_after(
        &self,
        topic: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<DurableEventRow>, BackendError> {
        // Parse topic to extract kind: "account:abc123" -> kind="account", topic="account:abc123".
        let topic_kind = topic.split_once(':').map(|(k, _)| k).unwrap_or(topic);
        let rows = store::durable_event_log::read_topic_after(
            &self.store,
            topic_kind,
            topic,
            after_seq,
            limit,
        )
        .await?;
        Ok(rows.into_iter().map(convert_durable_event_row).collect())
    }

    async fn retention_floor(&self, topic: &str) -> Result<i64, BackendError> {
        let topic_kind = topic.split_once(':').map(|(k, _)| k).unwrap_or(topic);
        store::durable_event_log::retention_floor(&self.store, topic_kind, topic).await
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — OutboxRepository
// ---------------------------------------------------------------------------

fn convert_outbox_row(row: store::outbox_events::OutboxEventRow) -> OutboxRow {
    let status_str = match row.status {
        store::outbox_events::OutboxStatus::Pending => "pending",
        store::outbox_events::OutboxStatus::Claimed => "claimed",
        store::outbox_events::OutboxStatus::Acked => "acked",
        store::outbox_events::OutboxStatus::Dead => "dead",
    };
    OutboxRow {
        outbox_id: row.outbox_id,
        topic_kind: row.topic_kind,
        event_id: row.event_id,
        status: status_str.to_string(),
        available_at_ms: row.available_at_ms,
        attempts: i32::try_from(row.attempts).unwrap_or(i32::MAX),
        claimed_by: row.claimed_by,
        claimed_at_ms: row.claimed_at_ms,
        ack_at_ms: row.ack_at_ms,
        dead_at_ms: row.dead_at_ms,
        last_error_json: row.last_error_json.map(|v| v.to_string()),
    }
}

struct StoreBackedOutboxRepository {
    store: StoreHandle,
}

#[async_trait]
impl OutboxRepository for StoreBackedOutboxRepository {
    async fn enqueue(
        &self,
        topic_kind: &str,
        event_id: &str,
        available_at_ms: i64,
    ) -> Result<String, BackendError> {
        let outbox_id = Uuid::new_v4().to_string();
        store::outbox_events::enqueue(
            &self.store,
            &outbox_id,
            topic_kind,
            event_id,
            available_at_ms,
        )
        .await?;
        Ok(outbox_id)
    }

    async fn claim(&self, worker_id: &str, batch: u32) -> Result<Vec<OutboxRow>, BackendError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let rows =
            store::outbox_events::claim_available(&self.store, worker_id, now_ms, batch).await?;
        Ok(rows.into_iter().map(convert_outbox_row).collect())
    }

    async fn ack(&self, outbox_id: &str, at_ms: i64) -> Result<bool, BackendError> {
        store::outbox_events::ack(&self.store, outbox_id, at_ms).await
    }

    async fn retry(
        &self,
        outbox_id: &str,
        available_at_ms: i64,
        last_error: &str,
    ) -> Result<bool, BackendError> {
        let error_json = serde_json::json!({ "message": last_error });
        store::outbox_events::retry(&self.store, outbox_id, available_at_ms, &error_json).await
    }

    async fn dead_letter(
        &self,
        outbox_id: &str,
        at_ms: i64,
        last_error: &str,
    ) -> Result<bool, BackendError> {
        let error_json = serde_json::json!({ "message": last_error });
        store::outbox_events::dead_letter(&self.store, outbox_id, at_ms, &error_json).await
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — AuditRepository
//
// The audit_events table exists only in the Postgres schema. On SQLite,
// insert/list operations silently succeed with no persistence (audit is
// non-critical for local development).
// ---------------------------------------------------------------------------

struct StoreBackedAuditRepository {
    store: StoreHandle,
}

#[async_trait]
impl AuditRepository for StoreBackedAuditRepository {
    async fn insert(
        &self,
        actor_kind: &str,
        account_id: Option<&str>,
        installation_id: Option<&str>,
        event_type: &str,
        metadata: Option<&str>,
        at_ms: i64,
    ) -> Result<AuditRow, BackendError> {
        let audit_id = Uuid::new_v4().to_string();
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(_) => {
                // No audit table in SQLite; return the constructed row.
            }
            StorePoolRef::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO audit_events \
                     (audit_id, actor_kind, account_id, installation_id, event_type, metadata, at_ms) \
                     VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7)",
                )
                .bind(&audit_id)
                .bind(actor_kind)
                .bind(account_id)
                .bind(installation_id)
                .bind(event_type)
                .bind(metadata)
                .bind(at_ms)
                .execute(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "audit.insert".into(),
                    message: e.to_string(),
                })?;
            }
        }
        Ok(AuditRow {
            audit_id,
            actor_kind: actor_kind.to_string(),
            account_id: account_id.map(str::to_string),
            installation_id: installation_id.map(str::to_string),
            event_type: event_type.to_string(),
            metadata: metadata.map(str::to_string),
            at_ms,
        })
    }

    async fn list_since(
        &self,
        account_id: &str,
        since_ms: i64,
        limit: u32,
    ) -> Result<Vec<AuditRow>, BackendError> {
        match self.store.as_store_pool() {
            StorePoolRef::Sqlite(_) => Ok(Vec::new()),
            StorePoolRef::Postgres(pool) => {
                let rows = sqlx::query_as::<
                    _,
                    (
                        String,
                        String,
                        Option<String>,
                        Option<String>,
                        String,
                        Option<String>,
                        i64,
                    ),
                >(
                    "SELECT audit_id, actor_kind, account_id, installation_id, \
                     event_type, metadata::text, at_ms \
                     FROM audit_events \
                     WHERE account_id = $1 AND at_ms >= $2 \
                     ORDER BY at_ms DESC LIMIT $3",
                )
                .bind(account_id)
                .bind(since_ms)
                .bind(i64::from(limit))
                .fetch_all(pool)
                .await
                .map_err(|e| BackendError::StoreQuery {
                    operation: "audit.list_since".into(),
                    message: e.to_string(),
                })?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(audit_id, actor_kind, acc_id, inst_id, event_type, metadata, at_ms)| {
                            AuditRow {
                                audit_id,
                                actor_kind,
                                account_id: acc_id,
                                installation_id: inst_id,
                                event_type,
                                metadata,
                                at_ms,
                            }
                        },
                    )
                    .collect())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Store-backed implementations — PushTokensRepository
// ---------------------------------------------------------------------------

struct StoreBackedPushTokensRepository {
    store: StoreHandle,
}

#[async_trait]
impl PushTokensRepository for StoreBackedPushTokensRepository {
    async fn upsert(
        &self,
        account_id: &str,
        installation_id: &str,
        kind: &str,
        token: &str,
        locale: Option<&str>,
        at_ms: i64,
    ) -> Result<PushTokenRow, BackendError> {
        let row = store::push_tokens::upsert(
            &self.store,
            account_id,
            installation_id,
            kind,
            token,
            locale,
            at_ms,
        )
        .await?;
        Ok(PushTokenRow {
            token_hash: row.token_hash,
            account_id: row.account_id,
            installation_id: row.installation_id,
            kind: row.kind,
            locale: row.locale,
            created_at_ms: row.created_at_ms,
            last_used_at_ms: row.last_used_at_ms,
            revoked_at_ms: row.revoked_at_ms,
        })
    }

    async fn revoke(&self, token_hash: &str, at_ms: i64) -> Result<bool, BackendError> {
        store::push_tokens::revoke(&self.store, token_hash, at_ms).await
    }

    async fn list_for_account(&self, account_id: &str) -> Result<Vec<PushTokenRow>, BackendError> {
        let rows = store::push_tokens::list_for_account(&self.store, account_id).await?;
        Ok(rows
            .into_iter()
            .map(|r| PushTokenRow {
                token_hash: r.token_hash,
                account_id: r.account_id,
                installation_id: r.installation_id,
                kind: r.kind,
                locale: r.locale,
                created_at_ms: r.created_at_ms,
                last_used_at_ms: r.last_used_at_ms,
                revoked_at_ms: r.revoked_at_ms,
            })
            .collect())
    }
}

#[allow(dead_code)]
struct StubPushTokensRepository;

#[async_trait]
impl PushTokensRepository for StubPushTokensRepository {
    async fn upsert(
        &self,
        _account_id: &str,
        _installation_id: &str,
        _kind: &str,
        _token: &str,
        _locale: Option<&str>,
        _at_ms: i64,
    ) -> Result<PushTokenRow, BackendError> {
        Err(BackendError::StoreQuery {
            operation: "push_tokens.upsert".into(),
            message: "stub: not yet implemented".into(),
        })
    }

    async fn revoke(&self, _token_hash: &str, _at_ms: i64) -> Result<bool, BackendError> {
        Err(BackendError::StoreQuery {
            operation: "push_tokens.revoke".into(),
            message: "stub: not yet implemented".into(),
        })
    }

    async fn list_for_account(&self, _account_id: &str) -> Result<Vec<PushTokenRow>, BackendError> {
        Err(BackendError::StoreQuery {
            operation: "push_tokens.list_for_account".into(),
            message: "stub: not yet implemented".into(),
        })
    }
}
