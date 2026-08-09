//! NotificationService trait + DefaultNotificationService implementation.
//!
//! Orchestrates push token registration, preference management, and
//! event-driven notification dispatch with presence + event_id idempotency.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::BackendError;
use crate::notifications::channels::{PushAttempt, PushChannel, PushKind, PushSendOutcome};
use crate::notifications::decision::{
    decide, AccountPresence, Decision, DecisionInput, DecisionReason,
};
use crate::notifications::preferences::NotificationPreferences;
use crate::realtime::event::{DurableEvent, DurableEventEnvelope, SenderRef};
use crate::session::SessionRegistry;
use crate::store::{
    agent_sessions, notification_preferences, push_dispatch_log, push_tokens, social, StoreHandle,
};

/// After the last mobile client WS disconnect, keep suppressing push for this
/// window so flaky reconnects do not flood the user.
///
/// Grace is based on registry-stamped disconnect time (real WS leave edge),
/// **not** throttled `device_installations.last_seen` (which can lag 30s+).
pub const DISCONNECT_GRACE_MS: i64 = 15_000;

// ── Presence port ──────────────────────────────────────────────────────

/// Live mobile WS + real disconnect timestamps for push grace.
pub trait PresencePort: Send + Sync {
    fn has_live_mobile_client(&self, account_id: &str) -> bool;
    /// Wall-clock ms when the account last lost its final live mobile WS.
    fn last_mobile_disconnect_at_ms(&self, account_id: &str) -> Option<i64>;
}

impl PresencePort for SessionRegistry {
    fn has_live_mobile_client(&self, account_id: &str) -> bool {
        self.mobile_client_session_count(account_id) > 0
    }

    fn last_mobile_disconnect_at_ms(&self, account_id: &str) -> Option<i64> {
        SessionRegistry::last_mobile_disconnect_at_ms(self, account_id)
    }
}

/// Always-offline presence for unit tests that do not inject a registry.
#[derive(Debug, Default)]
pub struct OfflinePresence;

impl PresencePort for OfflinePresence {
    fn has_live_mobile_client(&self, _account_id: &str) -> bool {
        false
    }

    fn last_mobile_disconnect_at_ms(&self, _account_id: &str) -> Option<i64> {
        None
    }
}

// ── DTOs ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterTokenInput {
    pub account_id: String,
    pub installation_id: String,
    pub kind: PushKind,
    pub token: String,
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnregisterTokenInput {
    pub account_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePreferencesInput {
    pub account_id: String,
    pub direct_message_enabled: Option<bool>,
    pub group_mention_enabled: Option<bool>,
    pub approval_required_enabled: Option<bool>,
    pub agent_session_ended_enabled: Option<bool>,
    pub quiet_hours_start_minute: Option<Option<i16>>,
    pub quiet_hours_end_minute: Option<Option<i16>>,
    pub quiet_hours_timezone: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushTokenDto {
    pub token_hash: String,
    pub installation_id: String,
    pub kind: PushKind,
    pub locale: Option<String>,
    pub created_at_ms: i64,
    pub last_used_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchOutcome {
    /// Notifications sent to at least one channel.
    Sent,
    /// Decision engine skipped the event (not notifiable, user online, cooldown, idempotent).
    Skipped,
    /// No push tokens registered for the target account(s).
    NoTokens,
}

/// Per-account outcome for the durable push queue worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountDispatchOutcome {
    /// At least one provider accepted the push; success ledger written.
    Sent,
    /// Intentional non-send (online, prefs, already pushed, no tokens, cooldown).
    /// Terminal for the queue row — do not retry.
    Skipped { reason: String },
    /// Provider/rate-limit failure — requeue with backoff.
    Transient { reason: String },
}

// ── Error ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("invalid push token format: {0}")]
    InvalidToken(String),
    #[error("token not found")]
    TokenNotFound,
    #[error(transparent)]
    Internal(#[from] BackendError),
}

// ── Trait ──────────────────────────────────────────────────────────────

#[async_trait]
pub trait NotificationService: Send + Sync {
    async fn register_token(&self, input: RegisterTokenInput) -> Result<(), NotificationError>;
    async fn unregister_token(&self, input: UnregisterTokenInput) -> Result<(), NotificationError>;
    async fn list_tokens(&self, account_id: &str) -> Result<Vec<PushTokenDto>, NotificationError>;
    async fn get_preferences(
        &self,
        account_id: &str,
    ) -> Result<NotificationPreferences, NotificationError>;
    async fn update_preferences(
        &self,
        input: UpdatePreferencesInput,
    ) -> Result<NotificationPreferences, NotificationError>;
    async fn dispatch_for_event(
        &self,
        event: &DurableEventEnvelope,
    ) -> Result<DispatchOutcome, NotificationError>;

    /// Dispatch push for a single account (durable push queue worker).
    async fn dispatch_for_account(
        &self,
        event: &DurableEventEnvelope,
        account_id: &str,
    ) -> Result<AccountDispatchOutcome, NotificationError>;
}

// ── Validation ─────────────────────────────────────────────────────────

/// Validate a push token for the given kind.
/// APNs: exactly 64 hex characters.
/// FCM: non-empty, max 4096 bytes.
fn validate_token(kind: PushKind, token: &str) -> Result<(), NotificationError> {
    match kind {
        PushKind::Apns => {
            if token.len() != 64 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(NotificationError::InvalidToken(
                    "APNs token must be exactly 64 hex characters".into(),
                ));
            }
        }
        PushKind::Fcm => {
            if token.is_empty() || token.len() > 4096 {
                return Err(NotificationError::InvalidToken(
                    "FCM token must be 1..4096 bytes".into(),
                ));
            }
        }
    }
    Ok(())
}

// ── Default implementation ─────────────────────────────────────────────

pub struct DefaultNotificationService {
    store: StoreHandle,
    channels: Vec<Arc<dyn PushChannel>>,
    presence: Arc<dyn PresencePort>,
    /// Default cooldown for conversation messages (30 seconds).
    #[allow(dead_code)]
    message_cooldown_ms: i64,
    /// Default cooldown for approval requests (5 seconds).
    #[allow(dead_code)]
    approval_cooldown_ms: i64,
}

impl DefaultNotificationService {
    #[must_use]
    pub fn new(
        store: StoreHandle,
        channels: Vec<Arc<dyn PushChannel>>,
        presence: Arc<dyn PresencePort>,
    ) -> Self {
        Self {
            store,
            channels,
            presence,
            message_cooldown_ms: 30_000,
            approval_cooldown_ms: 5_000,
        }
    }

    /// Find the matching push channel for a given kind.
    fn channel_for(&self, kind: PushKind) -> Option<&dyn PushChannel> {
        self.channels
            .iter()
            .find(|c| c.kind() == kind)
            .map(|c| c.as_ref())
    }

    /// Build presence for decide(): live WS + disconnect grace from registry.
    fn resolve_presence(&self, account_id: &str, now_ms: i64) -> AccountPresence {
        let online = self.presence.has_live_mobile_client(account_id);
        if online {
            return AccountPresence {
                online: true,
                within_disconnect_grace: false,
            };
        }

        let within_disconnect_grace = self
            .presence
            .last_mobile_disconnect_at_ms(account_id)
            .is_some_and(|at| now_ms >= at && now_ms.saturating_sub(at) < DISCONNECT_GRACE_MS);

        AccountPresence {
            online: false,
            within_disconnect_grace,
        }
    }
}

#[async_trait]
impl NotificationService for DefaultNotificationService {
    async fn register_token(&self, input: RegisterTokenInput) -> Result<(), NotificationError> {
        validate_token(input.kind, &input.token)?;
        let at_ms = chrono::Utc::now().timestamp_millis();
        push_tokens::upsert(
            &self.store,
            &input.account_id,
            &input.installation_id,
            input.kind.as_str(),
            &input.token,
            input.locale.as_deref(),
            at_ms,
        )
        .await?;
        Ok(())
    }

    async fn unregister_token(&self, input: UnregisterTokenInput) -> Result<(), NotificationError> {
        let token_hash = push_tokens::hash_token(&input.token);
        let at_ms = chrono::Utc::now().timestamp_millis();
        let revoked = push_tokens::revoke(&self.store, &token_hash, at_ms).await?;
        if !revoked {
            return Err(NotificationError::TokenNotFound);
        }
        Ok(())
    }

    async fn list_tokens(&self, account_id: &str) -> Result<Vec<PushTokenDto>, NotificationError> {
        let rows = push_tokens::list_for_account(&self.store, account_id).await?;
        Ok(rows
            .into_iter()
            .map(|r| PushTokenDto {
                token_hash: r.token_hash,
                installation_id: r.installation_id,
                kind: match r.kind.as_str() {
                    "apns" => PushKind::Apns,
                    _ => PushKind::Fcm,
                },
                locale: r.locale,
                created_at_ms: r.created_at_ms,
                last_used_at_ms: r.last_used_at_ms,
            })
            .collect())
    }

    async fn get_preferences(
        &self,
        account_id: &str,
    ) -> Result<NotificationPreferences, NotificationError> {
        let row = notification_preferences::get(&self.store, account_id).await?;
        Ok(NotificationPreferences::from_row(&row))
    }

    async fn update_preferences(
        &self,
        input: UpdatePreferencesInput,
    ) -> Result<NotificationPreferences, NotificationError> {
        let existing = notification_preferences::get(&self.store, &input.account_id).await?;
        let at_ms = chrono::Utc::now().timestamp_millis();

        let row = notification_preferences::upsert(
            &self.store,
            &input.account_id,
            input
                .direct_message_enabled
                .unwrap_or(existing.direct_message_enabled),
            input
                .group_mention_enabled
                .unwrap_or(existing.group_mention_enabled),
            input
                .approval_required_enabled
                .unwrap_or(existing.approval_required_enabled),
            input
                .agent_session_ended_enabled
                .unwrap_or(existing.agent_session_ended_enabled),
            input
                .quiet_hours_start_minute
                .unwrap_or(existing.quiet_hours_start_minute),
            input
                .quiet_hours_end_minute
                .unwrap_or(existing.quiet_hours_end_minute),
            input
                .quiet_hours_timezone
                .as_ref()
                .map(|v| v.as_deref())
                .unwrap_or(existing.quiet_hours_timezone.as_deref()),
            at_ms,
        )
        .await?;

        Ok(NotificationPreferences::from_row(&row))
    }

    async fn dispatch_for_event(
        &self,
        envelope: &DurableEventEnvelope,
    ) -> Result<DispatchOutcome, NotificationError> {
        let target_account_ids = resolve_target_accounts(&self.store, &envelope.payload).await?;
        if target_account_ids.is_empty() {
            return Ok(DispatchOutcome::Skipped);
        }

        let mut any_sent = false;
        let mut any_wanted_send_no_tokens = false;
        let mut any_other = false;

        for account_id in &target_account_ids {
            match self.dispatch_for_account(envelope, account_id).await? {
                AccountDispatchOutcome::Sent => any_sent = true,
                AccountDispatchOutcome::Skipped { reason } if reason == "no_tokens" => {
                    any_wanted_send_no_tokens = true;
                }
                AccountDispatchOutcome::Skipped { .. }
                | AccountDispatchOutcome::Transient { .. } => {
                    any_other = true;
                }
            }
        }

        if any_sent {
            Ok(DispatchOutcome::Sent)
        } else if any_wanted_send_no_tokens && !any_other {
            Ok(DispatchOutcome::NoTokens)
        } else {
            Ok(DispatchOutcome::Skipped)
        }
    }

    async fn dispatch_for_account(
        &self,
        envelope: &DurableEventEnvelope,
        account_id: &str,
    ) -> Result<AccountDispatchOutcome, NotificationError> {
        let now_ms = chrono::Utc::now().timestamp_millis();

        let already_pushed =
            push_dispatch_log::has_sent(&self.store, &envelope.event_id, account_id).await?;
        let presence = self.resolve_presence(account_id, now_ms);
        let prefs = notification_preferences::get(&self.store, account_id).await?;
        let prefs = NotificationPreferences::from_row(&prefs);

        let decision = decide(&DecisionInput {
            event: &envelope.payload,
            prefs: &prefs,
            now_ms,
            presence,
            already_pushed_event: already_pushed,
        });

        match decision {
            Decision::Send {
                payload,
                cooldown_key,
                cooldown_ms,
            } => {
                let allowed = crate::store::notification_cooldowns::is_allowed(
                    &self.store,
                    account_id,
                    &cooldown_key,
                    cooldown_ms,
                    now_ms,
                )
                .await?;

                if !allowed {
                    crate::telemetry::record_push_decision("skip", "cooldown");
                    return Ok(AccountDispatchOutcome::Skipped {
                        reason: "cooldown".into(),
                    });
                }

                let tokens = push_tokens::list_for_account(&self.store, account_id).await?;
                if tokens.is_empty() {
                    return Ok(AccountDispatchOutcome::Skipped {
                        reason: "no_tokens".into(),
                    });
                }

                let mut account_sent = false;
                let mut saw_rate_limited = false;
                let mut saw_provider_error = false;
                let mut only_not_wired = true;

                for token_row in &tokens {
                    let kind = match token_row.kind.as_str() {
                        "apns" => PushKind::Apns,
                        _ => PushKind::Fcm,
                    };
                    if let Some(channel) = self.channel_for(kind) {
                        let attempt = PushAttempt {
                            token_hash: token_row.token_hash.clone(),
                            account_id: account_id.to_owned(),
                            payload: payload.clone(),
                        };
                        match channel.send(attempt).await {
                            Ok(PushSendOutcome::Sent) => {
                                account_sent = true;
                                only_not_wired = false;
                                crate::telemetry::record_push_send(kind.as_str(), "sent");
                            }
                            Ok(PushSendOutcome::TokenExpired) => {
                                only_not_wired = false;
                                let _ =
                                    push_tokens::revoke(&self.store, &token_row.token_hash, now_ms)
                                        .await;
                                crate::telemetry::record_push_send(kind.as_str(), "token_expired");
                            }
                            Ok(PushSendOutcome::RateLimited) => {
                                only_not_wired = false;
                                saw_rate_limited = true;
                                crate::telemetry::record_push_send(kind.as_str(), "rate_limited");
                            }
                            Ok(PushSendOutcome::NotWired) => {
                                // Config present but provider not production-wired.
                                crate::telemetry::record_push_send(kind.as_str(), "not_wired");
                            }
                            Err(_) => {
                                only_not_wired = false;
                                saw_provider_error = true;
                                crate::telemetry::record_push_send(kind.as_str(), "error");
                            }
                        }
                    }
                }

                if account_sent {
                    push_dispatch_log::record_sent(
                        &self.store,
                        &envelope.event_id,
                        account_id,
                        now_ms,
                    )
                    .await?;
                    let _ = crate::store::notification_cooldowns::record_sent(
                        &self.store,
                        account_id,
                        &cooldown_key,
                        now_ms,
                    )
                    .await;
                    return Ok(AccountDispatchOutcome::Sent);
                }

                if only_not_wired {
                    // Dev/unwired: terminal skip so queue does not thrash forever.
                    return Ok(AccountDispatchOutcome::Skipped {
                        reason: "not_wired".into(),
                    });
                }
                if saw_rate_limited {
                    return Ok(AccountDispatchOutcome::Transient {
                        reason: "rate_limited".into(),
                    });
                }
                if saw_provider_error {
                    return Ok(AccountDispatchOutcome::Transient {
                        reason: "provider_error".into(),
                    });
                }
                Ok(AccountDispatchOutcome::Skipped {
                    reason: "no_successful_send".into(),
                })
            }
            Decision::Skip { reason } => {
                let reason_label = match reason {
                    DecisionReason::UserOnline => "user_online",
                    DecisionReason::AlreadyPushed => "already_pushed",
                    DecisionReason::QuietHours => "quiet_hours",
                    DecisionReason::PreferenceDisabled => "preference_disabled",
                    DecisionReason::NotNotifiable => "not_notifiable",
                };
                crate::telemetry::record_push_decision("skip", reason_label);
                Ok(AccountDispatchOutcome::Skipped {
                    reason: reason_label.into(),
                })
            }
        }
    }
}

/// Resolve account IDs that should receive a push for the event.
///
/// - Message account events: the account topic owner (not self-sender).
/// - Approval / session ended: conversation members of the agent session.
pub async fn resolve_target_accounts(
    store: &StoreHandle,
    event: &DurableEvent,
) -> Result<Vec<String>, NotificationError> {
    match event {
        DurableEvent::AccountConversationMessageAppended {
            account_id, sender, ..
        } => {
            if matches!(sender, SenderRef::User { account_id: sender_id } if sender_id == account_id)
            {
                Ok(Vec::new())
            } else {
                Ok(vec![account_id.clone()])
            }
        }
        DurableEvent::AccountConversationMessageRecalled { account_id, .. } => {
            Ok(vec![account_id.clone()])
        }
        DurableEvent::ConversationMessageAppended { .. }
        | DurableEvent::ConversationMessageRecalled { .. }
        | DurableEvent::ConversationMessageReactionUpdated { .. } => Ok(Vec::new()),
        DurableEvent::ApprovalRequested { session_id, .. }
        | DurableEvent::AgentSessionEnded { session_id, .. } => {
            Ok(resolve_session_notification_targets(store, session_id).await?)
        }
        _ => Ok(Vec::new()),
    }
}

/// Session owner / approvers = conversation members for the agent session.
async fn resolve_session_notification_targets(
    store: &StoreHandle,
    session_id: &str,
) -> Result<Vec<String>, BackendError> {
    let Some(session) = agent_sessions::get(store, session_id).await? else {
        tracing::debug!(
            target: "minos_backend::notifications",
            session_id,
            "no agent_session row for push target resolution"
        );
        return Ok(Vec::new());
    };
    let members = social::list_conversation_members(store, &session.conversation_id).await?;
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::channels::{PushChannel, PushPayload, PushSendOutcome};
    use crate::store::test_support::{insert_account, memory_pool, T0};
    use minos_domain::DeviceId;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct CountingChannel {
        sent: AtomicUsize,
        payloads: Mutex<Vec<PushPayload>>,
    }

    impl CountingChannel {
        fn new() -> Self {
            Self {
                sent: AtomicUsize::new(0),
                payloads: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl PushChannel for CountingChannel {
        fn kind(&self) -> PushKind {
            PushKind::Fcm
        }

        async fn send(
            &self,
            attempt: PushAttempt,
        ) -> Result<PushSendOutcome, crate::notifications::channels::PushSendError> {
            self.sent.fetch_add(1, Ordering::SeqCst);
            self.payloads.lock().unwrap().push(attempt.payload);
            Ok(PushSendOutcome::Sent)
        }
    }

    struct FixedPresence {
        online: bool,
        /// When set, simulates registry disconnect stamp for grace tests.
        last_disconnect_at_ms: Option<i64>,
    }

    impl PresencePort for FixedPresence {
        fn has_live_mobile_client(&self, _account_id: &str) -> bool {
            self.online
        }

        fn last_mobile_disconnect_at_ms(&self, _account_id: &str) -> Option<i64> {
            self.last_disconnect_at_ms
        }
    }

    async fn seed_account_with_token(
        store: &StoreHandle,
        email: &str,
    ) -> (String, String /* installation_id */) {
        let account_id = insert_account(store.sqlite_pool().unwrap(), email).await;
        let installation_id = DeviceId::new().to_string();
        // Minimal installation so FK on push_tokens / device tables hold if needed.
        sqlx::query(
            "INSERT INTO device_installations
                (installation_id, kind, display_name, public_key, created_at_ms, last_seen_at_ms, account_id)
             VALUES (?, 'mobile', 'phone', NULL, ?, ?, ?)",
        )
        .bind(&installation_id)
        .bind(T0)
        .bind(T0 - 60_000) // outside grace
        .bind(&account_id)
        .execute(store.sqlite_pool().unwrap())
        .await
        .unwrap();

        push_tokens::upsert(
            store,
            &account_id,
            &installation_id,
            "fcm",
            "test-fcm-token-value-xxxxxxxxxxxxxxxx",
            None,
            T0,
        )
        .await
        .unwrap();

        (account_id, installation_id)
    }

    fn account_message_envelope(account_id: &str, event_id: &str) -> DurableEventEnvelope {
        DurableEventEnvelope {
            topic: format!("account:{account_id}"),
            topic_seq: 1,
            event_id: event_id.into(),
            payload: DurableEvent::AccountConversationMessageAppended {
                account_id: account_id.into(),
                conversation_id: "conv-1".into(),
                message_id: "msg-1".into(),
                sender: SenderRef::User {
                    account_id: "sender-other".into(),
                },
                at_ms: T0,
                preview: "hello".into(),
                sender_display_name: "Test User".into(),
                mentioned: false,
                message_seq: Some(1),
            },
        }
    }

    #[tokio::test]
    async fn online_presence_skips_push() {
        let pool = memory_pool().await;
        let store = StoreHandle::from(pool);
        let (account_id, _) = seed_account_with_token(&store, "online@example.com").await;
        let channel = Arc::new(CountingChannel::new());
        let svc = DefaultNotificationService::new(
            store,
            vec![channel.clone()],
            Arc::new(FixedPresence {
                online: true,
                last_disconnect_at_ms: None,
            }),
        );
        let envelope = account_message_envelope(&account_id, "ev-online-1");
        let outcome = svc.dispatch_for_event(&envelope).await.unwrap();
        assert_eq!(outcome, DispatchOutcome::Skipped);
        assert_eq!(channel.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn offline_presence_sends_push() {
        let pool = memory_pool().await;
        let store = StoreHandle::from(pool);
        let (account_id, _) = seed_account_with_token(&store, "offline@example.com").await;
        let channel = Arc::new(CountingChannel::new());
        let svc = DefaultNotificationService::new(
            store,
            vec![channel.clone()],
            Arc::new(FixedPresence {
                online: false,
                last_disconnect_at_ms: None,
            }),
        );
        let envelope = account_message_envelope(&account_id, "ev-offline-1");
        let outcome = svc.dispatch_for_event(&envelope).await.unwrap();
        assert_eq!(outcome, DispatchOutcome::Sent);
        assert_eq!(channel.sent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn just_disconnected_still_skips_within_grace() {
        let pool = memory_pool().await;
        let store = StoreHandle::from(pool);
        let (account_id, _) = seed_account_with_token(&store, "grace@example.com").await;
        let channel = Arc::new(CountingChannel::new());
        // Disconnect 2s ago — well within DISCONNECT_GRACE_MS (15s).
        let now = chrono::Utc::now().timestamp_millis();
        let svc = DefaultNotificationService::new(
            store,
            vec![channel.clone()],
            Arc::new(FixedPresence {
                online: false,
                last_disconnect_at_ms: Some(now - 2_000),
            }),
        );
        let envelope = account_message_envelope(&account_id, "ev-grace-1");
        let outcome = svc.dispatch_for_event(&envelope).await.unwrap();
        assert_eq!(outcome, DispatchOutcome::Skipped);
        assert_eq!(channel.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn disconnect_grace_expires_then_sends() {
        let pool = memory_pool().await;
        let store = StoreHandle::from(pool);
        let (account_id, _) = seed_account_with_token(&store, "grace-done@example.com").await;
        let channel = Arc::new(CountingChannel::new());
        let now = chrono::Utc::now().timestamp_millis();
        let svc = DefaultNotificationService::new(
            store,
            vec![channel.clone()],
            Arc::new(FixedPresence {
                online: false,
                // Past grace window.
                last_disconnect_at_ms: Some(now - DISCONNECT_GRACE_MS - 5_000),
            }),
        );
        let envelope = account_message_envelope(&account_id, "ev-grace-2");
        let outcome = svc.dispatch_for_event(&envelope).await.unwrap();
        assert_eq!(outcome, DispatchOutcome::Sent);
        assert_eq!(channel.sent.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn registry_stamps_disconnect_when_last_mobile_leaves() {
        use minos_domain::DeviceRole;
        let reg = SessionRegistry::new();
        let (handle, _rx) = crate::session::SessionHandle::new(
            minos_domain::DeviceId::new(),
            DeviceRole::MobileClient,
        );
        handle.set_account_id("acct-grace".into());
        reg.insert(handle.clone());
        assert!(reg.mobile_client_session_count("acct-grace") > 0);
        assert!(reg.last_mobile_disconnect_at_ms("acct-grace").is_none());

        let removed = reg.remove(handle.device_id).expect("removed");
        assert_eq!(removed.device_id, handle.device_id);
        assert_eq!(reg.mobile_client_session_count("acct-grace"), 0);
        assert!(
            reg.last_mobile_disconnect_at_ms("acct-grace").is_some(),
            "last mobile leave must stamp disconnect for push grace"
        );
    }

    #[tokio::test]
    async fn event_id_idempotency_prevents_second_send() {
        let pool = memory_pool().await;
        let store = StoreHandle::from(pool);
        let (account_id, _) = seed_account_with_token(&store, "idem@example.com").await;
        let channel = Arc::new(CountingChannel::new());
        let svc = DefaultNotificationService::new(
            store.clone(),
            vec![channel.clone()],
            Arc::new(FixedPresence {
                online: false,
                last_disconnect_at_ms: None,
            }),
        );
        let envelope = account_message_envelope(&account_id, "ev-idem-1");
        assert_eq!(
            svc.dispatch_for_event(&envelope).await.unwrap(),
            DispatchOutcome::Sent
        );
        assert_eq!(channel.sent.load(Ordering::SeqCst), 1);

        // Re-dispatch same event_id must not send again.
        assert_eq!(
            svc.dispatch_for_event(&envelope).await.unwrap(),
            DispatchOutcome::Skipped
        );
        assert_eq!(channel.sent.load(Ordering::SeqCst), 1);
        assert!(
            push_dispatch_log::has_sent(&store, "ev-idem-1", &account_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn approval_targets_conversation_members() {
        let pool = memory_pool().await;
        let store = StoreHandle::from(pool);
        let owner = insert_account(store.sqlite_pool().unwrap(), "owner@example.com").await;
        let member = insert_account(store.sqlite_pool().unwrap(), "member@example.com").await;

        // Conversation + members
        sqlx::query(
            "INSERT INTO conversations (conversation_id, kind, title, created_at_ms, updated_at_ms, created_by_account_id)
             VALUES ('conv-appr', 'group', 'G', ?, ?, ?)",
        )
        .bind(T0)
        .bind(T0)
        .bind(&owner)
        .execute(store.sqlite_pool().unwrap())
        .await
        .unwrap();
        for (acc, joined) in [(&owner, T0), (&member, T0 + 1)] {
            sqlx::query(
                "INSERT INTO conversation_members (conversation_id, account_id, joined_at_ms)
                 VALUES ('conv-appr', ?, ?)",
            )
            .bind(acc)
            .bind(joined)
            .execute(store.sqlite_pool().unwrap())
            .await
            .unwrap();
        }
        agent_sessions::create(
            &store,
            "sess-appr",
            "conv-appr",
            None,
            None,
            None,
            "running",
            T0,
            None,
        )
        .await
        .unwrap();

        let event = DurableEvent::ApprovalRequested {
            request_id: "req-1".into(),
            session_id: "sess-appr".into(),
            method: "run_command".into(),
            deadline_at_ms: T0 + 60_000,
            at_ms: T0,
        };
        let targets = resolve_target_accounts(&store, &event).await.unwrap();
        assert!(!targets.is_empty(), "approval targets must be non-empty");
        assert!(targets.contains(&owner));
        assert!(targets.contains(&member));
    }

    #[test]
    fn account_conversation_event_targets_account_topic_owner() {
        // Sync helper for pure match on message events — exercised via async path
        // in integration tests above; keep smoke on resolve for self-skip.
        let event = DurableEvent::AccountConversationMessageAppended {
            account_id: "target-account".into(),
            conversation_id: "conv-1".into(),
            message_id: "msg-1".into(),
            sender: SenderRef::User {
                account_id: "target-account".into(),
            },
            at_ms: T0,
            preview: "hello".into(),
            sender_display_name: "Test User".into(),
            mentioned: false,
            message_seq: Some(1),
        };
        // Self-sender: empty targets (resolved without DB).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let pool = memory_pool().await;
            let store = StoreHandle::from(pool);
            assert!(resolve_target_accounts(&store, &event)
                .await
                .unwrap()
                .is_empty());
        });
    }
}
