//! Decision engine for push notifications.
//!
//! Pure function over [`DecisionInput`]: event, preferences, clock, presence,
//! and event-level idempotency. Side effects (send, cooldown, log) stay in
//! the dispatch layer.

use serde::{Deserialize, Serialize};

use crate::notifications::channels::PushPayload;
use crate::notifications::preferences::NotificationPreferences;
use crate::realtime::event::DurableEvent;

/// Live / recent presence for one account (injected by dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountPresence {
    /// Any live mobile client WebSocket for this account.
    pub online: bool,
    /// Offline but last disconnect within the configured grace window.
    /// Final policy: suppress push during grace, allow after.
    pub within_disconnect_grace: bool,
}

impl AccountPresence {
    /// Whether push should be suppressed for UX (online or grace).
    #[must_use]
    pub fn suppresses_push(self) -> bool {
        self.online || self.within_disconnect_grace
    }
}

/// Full input to the pure notification decision function.
#[derive(Debug, Clone, Copy)]
pub struct DecisionInput<'a> {
    pub event: &'a DurableEvent,
    pub prefs: &'a NotificationPreferences,
    pub now_ms: i64,
    pub presence: AccountPresence,
    /// True when `(event_id, account_id)` already has a successful push log row.
    pub already_pushed_event: bool,
}

/// The outcome of a notification decision.
#[derive(Debug, Clone)]
pub enum Decision {
    Send {
        payload: PushPayload,
        cooldown_key: String,
        cooldown_ms: i64,
    },
    Skip {
        reason: DecisionReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    /// Event type is not notifiable (or conversation-topic only — no account push).
    NotNotifiable,
    /// Self-sender should not receive their own message.
    SelfSender,
    /// User has disabled this notification category.
    PreferenceDisabled,
    /// Currently in quiet hours.
    QuietHours,
    /// Account has a live client WS (or is within disconnect grace).
    UserOnline,
    /// This event_id was already successfully pushed to the account.
    AlreadyPushed,
}

/// Default cooldown for conversation messages (30 seconds). UX rate limit only.
const MESSAGE_COOLDOWN_MS: i64 = 30_000;
/// Shorter cooldown for approval requests (5 seconds).
const APPROVAL_COOLDOWN_MS: i64 = 5_000;
/// Agent session ended cooldown (60 seconds).
const SESSION_ENDED_COOLDOWN_MS: i64 = 60_000;

/// Core decision function. Evaluates a complete [`DecisionInput`].
#[must_use]
pub fn decide(input: &DecisionInput<'_>) -> Decision {
    if input.already_pushed_event {
        return Decision::Skip {
            reason: DecisionReason::AlreadyPushed,
        };
    }

    let current_minute = ((input.now_ms / 60_000) % 1440) as i16;

    // Quiet hours first — applies to all event types except high-priority approvals.
    if input.prefs.is_quiet_hours(current_minute) {
        if !matches!(input.event, DurableEvent::ApprovalRequested { .. }) {
            return Decision::Skip {
                reason: DecisionReason::QuietHours,
            };
        }
    }

    // Presence: suppress when online or within disconnect grace.
    // Approvals still respect presence (user can see them in-app when online).
    if input.presence.suppresses_push() {
        return Decision::Skip {
            reason: DecisionReason::UserOnline,
        };
    }

    match input.event {
        // Account-scoped message fanout is the notifiable path for push.
        DurableEvent::AccountConversationMessageAppended {
            conversation_id,
            message_id,
            preview,
            ..
        } => {
            if !input.prefs.direct_message_enabled && !input.prefs.group_mention_enabled {
                return Decision::Skip {
                    reason: DecisionReason::PreferenceDisabled,
                };
            }

            // R3: push uses account digest preview — never full message body.
            let body = if preview.trim().is_empty() {
                "You have a new message".into()
            } else {
                truncate_body(preview, 120)
            };

            Decision::Send {
                payload: PushPayload {
                    title: "New message".into(),
                    body,
                    category: "message".into(),
                    data: serde_json::json!({
                        "conversation_id": conversation_id,
                        "message_id": message_id,
                    }),
                },
                cooldown_key: format!("msg:{conversation_id}"),
                cooldown_ms: MESSAGE_COOLDOWN_MS,
            }
        }

        DurableEvent::ApprovalRequested {
            request_id,
            session_id,
            method,
            ..
        } => {
            if !input.prefs.approval_required_enabled {
                return Decision::Skip {
                    reason: DecisionReason::PreferenceDisabled,
                };
            }

            Decision::Send {
                payload: PushPayload {
                    title: "Approval Required".into(),
                    body: format!("An agent needs your approval for: {method}"),
                    category: "approval".into(),
                    data: serde_json::json!({
                        "request_id": request_id,
                        "session_id": session_id,
                    }),
                },
                cooldown_key: format!("approval:{request_id}"),
                cooldown_ms: APPROVAL_COOLDOWN_MS,
            }
        }

        DurableEvent::AgentSessionEnded {
            session_id, status, ..
        } => {
            if !input.prefs.agent_session_ended_enabled {
                return Decision::Skip {
                    reason: DecisionReason::PreferenceDisabled,
                };
            }

            Decision::Send {
                payload: PushPayload {
                    title: "Agent Session Ended".into(),
                    body: format!("Agent session ended with status: {status}"),
                    category: "session_ended".into(),
                    data: serde_json::json!({
                        "session_id": session_id,
                    }),
                },
                cooldown_key: format!("session_end:{session_id}"),
                cooldown_ms: SESSION_ENDED_COOLDOWN_MS,
            }
        }

        // Conversation-topic events must not push (account-scoped twin carries targets).
        DurableEvent::ConversationMessageAppended { .. }
        | DurableEvent::ConversationMessageRecalled { .. }
        | DurableEvent::ConversationMessageReactionUpdated { .. } => Decision::Skip {
            reason: DecisionReason::NotNotifiable,
        },

        _ => Decision::Skip {
            reason: DecisionReason::NotNotifiable,
        },
    }
}

fn truncate_body(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::event::SenderRef;
    use minos_protocol::{ChatMessageSummary, SenderType, UserSummary};

    fn default_prefs() -> NotificationPreferences {
        NotificationPreferences {
            account_id: "acc1".into(),
            direct_message_enabled: true,
            group_mention_enabled: true,
            approval_required_enabled: true,
            agent_session_ended_enabled: false,
            quiet_hours_start_minute: None,
            quiet_hours_end_minute: None,
            quiet_hours_timezone: None,
        }
    }

    fn offline_presence() -> AccountPresence {
        AccountPresence {
            online: false,
            within_disconnect_grace: false,
        }
    }

    fn sample_message() -> ChatMessageSummary {
        ChatMessageSummary {
            message_id: "msg1".into(),
            conversation_id: "conv1".into(),
            sender: UserSummary {
                account_id: "other".into(),
                minos_id: "minos-other".into(),
                display_name: "Other".into(),
            },
            text: "hello".into(),
            created_at_ms: 1000,
            message_seq: 1,
            reply_to: None,
            recalled_at_ms: None,
            mentioned_account_ids: Vec::new(),
            sender_type: SenderType::User,
            reactions: vec![],
        }
    }

    fn account_message_event() -> DurableEvent {
        let msg = sample_message();
        DurableEvent::AccountConversationMessageAppended {
            account_id: "acc1".into(),
            conversation_id: "conv1".into(),
            message_id: "msg1".into(),
            sender: SenderRef::User {
                account_id: "other".into(),
            },
            at_ms: 1000,
            preview: msg.text.clone(),
            sender_display_name: msg.sender.display_name,
            mentioned: false,
            message_seq: Some(msg.message_seq),
        }
    }

    fn input_for<'a>(
        event: &'a DurableEvent,
        prefs: &'a NotificationPreferences,
        presence: AccountPresence,
        already_pushed: bool,
    ) -> DecisionInput<'a> {
        DecisionInput {
            event,
            prefs,
            now_ms: 1000,
            presence,
            already_pushed_event: already_pushed,
        }
    }

    #[test]
    fn account_message_sends_when_offline_and_prefs_enabled() {
        let event = account_message_event();
        let prefs = default_prefs();
        let result = decide(&input_for(&event, &prefs, offline_presence(), false));
        assert!(matches!(result, Decision::Send { .. }));
    }

    #[test]
    fn account_message_skipped_when_user_online() {
        let event = account_message_event();
        let prefs = default_prefs();
        let presence = AccountPresence {
            online: true,
            within_disconnect_grace: false,
        };
        let result = decide(&input_for(&event, &prefs, presence, false));
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::UserOnline
            }
        ));
    }

    #[test]
    fn account_message_skipped_within_disconnect_grace() {
        let event = account_message_event();
        let prefs = default_prefs();
        let presence = AccountPresence {
            online: false,
            within_disconnect_grace: true,
        };
        let result = decide(&input_for(&event, &prefs, presence, false));
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::UserOnline
            }
        ));
    }

    #[test]
    fn already_pushed_skips_idempotent() {
        let event = account_message_event();
        let prefs = default_prefs();
        let result = decide(&input_for(&event, &prefs, offline_presence(), true));
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::AlreadyPushed
            }
        ));
    }

    #[test]
    fn account_message_skipped_when_disabled() {
        let event = account_message_event();
        let mut prefs = default_prefs();
        prefs.direct_message_enabled = false;
        prefs.group_mention_enabled = false;
        let result = decide(&input_for(&event, &prefs, offline_presence(), false));
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::PreferenceDisabled
            }
        ));
    }

    #[test]
    fn approval_request_bypasses_quiet_hours() {
        let event = DurableEvent::ApprovalRequested {
            request_id: "req1".into(),
            session_id: "sess1".into(),
            method: "run_command".into(),
            deadline_at_ms: 5000,
            at_ms: 1000,
        };
        let mut prefs = default_prefs();
        prefs.quiet_hours_start_minute = Some(0);
        prefs.quiet_hours_end_minute = Some(1439);
        let result = decide(&input_for(&event, &prefs, offline_presence(), false));
        assert!(matches!(result, Decision::Send { .. }));
    }

    #[test]
    fn message_skipped_during_quiet_hours() {
        let event = account_message_event();
        let mut prefs = default_prefs();
        prefs.quiet_hours_start_minute = Some(0);
        prefs.quiet_hours_end_minute = Some(1439);
        let result = decide(&input_for(&event, &prefs, offline_presence(), false));
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::QuietHours
            }
        ));
    }

    #[test]
    fn conversation_topic_message_not_notifiable() {
        let event = DurableEvent::ConversationMessageAppended {
            conversation_id: "conv1".into(),
            message_id: "msg1".into(),
            sender: SenderRef::User {
                account_id: "other".into(),
            },
            at_ms: 1000,
            message: None,
        };
        let prefs = default_prefs();
        let result = decide(&input_for(&event, &prefs, offline_presence(), false));
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::NotNotifiable
            }
        ));
    }

    #[test]
    fn non_notifiable_event_skipped() {
        let event = DurableEvent::AccountRegistered {
            account_id: "acc1".into(),
            at_ms: 1000,
        };
        let prefs = default_prefs();
        let result = decide(&input_for(&event, &prefs, offline_presence(), false));
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::NotNotifiable
            }
        ));
    }
}
