//! NotificationService trait + DefaultNotificationService implementation.
//!
//! Orchestrates push token registration, preference management, and
//! event-driven notification dispatch.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::BackendError;
use crate::notifications::channels::{PushAttempt, PushChannel, PushKind, PushSendOutcome};
use crate::notifications::decision::{decide, Decision};
use crate::notifications::preferences::NotificationPreferences;
use crate::realtime::event::{DurableEvent, DurableEventEnvelope, SenderRef};
use crate::store::{notification_preferences, push_tokens, StoreHandle};

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
    /// Decision engine skipped the event (not notifiable, user online, cooldown).
    Skipped,
    /// No push tokens registered for the target account(s).
    NoTokens,
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
    /// Default cooldown for conversation messages (30 seconds).
    message_cooldown_ms: i64,
    /// Default cooldown for approval requests (5 seconds).
    approval_cooldown_ms: i64,
}

impl DefaultNotificationService {
    #[must_use]
    pub fn new(store: StoreHandle, channels: Vec<Arc<dyn PushChannel>>) -> Self {
        Self {
            store,
            channels,
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
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Resolve target account IDs from the event.
        let target_account_ids = resolve_target_accounts(&envelope.payload);
        if target_account_ids.is_empty() {
            return Ok(DispatchOutcome::Skipped);
        }

        let mut any_sent = false;
        let mut any_tokens = false;

        for account_id in &target_account_ids {
            let prefs = notification_preferences::get(&self.store, account_id).await?;
            let prefs = NotificationPreferences::from_row(&prefs);

            let decision = decide(&envelope.payload, &prefs, now_ms);
            match decision {
                Decision::Send {
                    payload,
                    cooldown_key,
                    cooldown_ms,
                } => {
                    // Check cooldown
                    let allowed = crate::store::notification_cooldowns::check_and_update(
                        &self.store,
                        account_id,
                        &cooldown_key,
                        cooldown_ms,
                        now_ms,
                    )
                    .await?;

                    if !allowed {
                        continue;
                    }

                    // Get tokens for this account
                    let tokens = push_tokens::list_for_account(&self.store, account_id).await?;
                    if tokens.is_empty() {
                        continue;
                    }
                    any_tokens = true;

                    // Send to each token
                    for token_row in &tokens {
                        let kind = match token_row.kind.as_str() {
                            "apns" => PushKind::Apns,
                            _ => PushKind::Fcm,
                        };
                        if let Some(channel) = self.channel_for(kind) {
                            let attempt = PushAttempt {
                                token_hash: token_row.token_hash.clone(),
                                account_id: account_id.clone(),
                                payload: payload.clone(),
                            };
                            match channel.send(attempt).await {
                                Ok(PushSendOutcome::Sent) => {
                                    any_sent = true;
                                    crate::telemetry::record_push_send(kind.as_str(), "sent");
                                }
                                Ok(PushSendOutcome::TokenExpired) => {
                                    // Revoke expired token
                                    let _ = push_tokens::revoke(
                                        &self.store,
                                        &token_row.token_hash,
                                        now_ms,
                                    )
                                    .await;
                                    crate::telemetry::record_push_send(
                                        kind.as_str(),
                                        "token_expired",
                                    );
                                }
                                Ok(PushSendOutcome::RateLimited) => {
                                    crate::telemetry::record_push_send(
                                        kind.as_str(),
                                        "rate_limited",
                                    );
                                }
                                Err(_) => {
                                    crate::telemetry::record_push_send(kind.as_str(), "error");
                                }
                            }
                        }
                    }
                }
                Decision::Skip { .. } => {
                    crate::telemetry::record_push_decision("skip", "decision_engine");
                }
            }
        }

        if any_sent {
            Ok(DispatchOutcome::Sent)
        } else if !any_tokens {
            Ok(DispatchOutcome::NoTokens)
        } else {
            Ok(DispatchOutcome::Skipped)
        }
    }
}

/// Resolve the set of account IDs that should receive a notification for
/// the given event. Returns empty for events that don't trigger push.
fn resolve_target_accounts(event: &DurableEvent) -> Vec<String> {
    match event {
        DurableEvent::ConversationMessageAppended { sender, .. } => {
            // The sender should NOT receive a push for their own message.
            match sender {
                SenderRef::User { account_id } => vec![account_id.clone()],
                _ => Vec::new(),
            }
        }
        DurableEvent::ApprovalRequested { .. } => {
            // The target account is the session owner; resolved externally
            // by the fanout job via the agent session lookup.
            Vec::new()
        }
        DurableEvent::AgentSessionEnded { .. } => {
            // Would need to look up the session owner; for now, empty.
            // In production, the fanout job resolves this from the session.
            Vec::new()
        }
        _ => Vec::new(),
    }
}
