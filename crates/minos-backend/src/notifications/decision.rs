//! Decision engine for push notifications.
//!
//! Given a durable event, user preferences, and current time, decides
//! whether to send a push notification and which payload to use.

use serde::{Deserialize, Serialize};

use crate::notifications::preferences::NotificationPreferences;
use crate::notifications::channels::PushPayload;
use crate::realtime::event::{DurableEvent, SenderRef};

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
    /// Event type is not notifiable.
    NotNotifiable,
    /// Self-sender should not receive their own message.
    SelfSender,
    /// User has disabled this notification category.
    PreferenceDisabled,
    /// Currently in quiet hours.
    QuietHours,
    /// Any installation for this account is currently online.
    UserOnline,
}

/// Default cooldown for conversation messages (30 seconds).
const MESSAGE_COOLDOWN_MS: i64 = 30_000;
/// Shorter cooldown for approval requests (5 seconds).
const APPROVAL_COOLDOWN_MS: i64 = 5_000;
/// Agent session ended cooldown (60 seconds).
const SESSION_ENDED_COOLDOWN_MS: i64 = 60_000;

/// Core decision function. Evaluates an event against preferences and
/// returns whether a push notification should be sent.
///
/// Note: The `presence` check (whether any installation is online) is
/// handled at the dispatch layer, not here, because it requires access
/// to the session registry which is not available to the pure decision
/// function. Callers should check presence before or after calling this.
pub fn decide(
    event: &DurableEvent,
    prefs: &NotificationPreferences,
    now_ms: i64,
) -> Decision {
    let current_minute = ((now_ms / 60_000) % 1440) as i16;

    // Check quiet hours first — applies to all event types.
    if prefs.is_quiet_hours(current_minute) {
        // Exception: approval requests bypass quiet hours (high priority).
        if !matches!(event, DurableEvent::ApprovalRequested { .. }) {
            return Decision::Skip {
                reason: DecisionReason::QuietHours,
            };
        }
    }

    match event {
        DurableEvent::ConversationMessageAppended {
            conversation_id,
            message_id,
            sender,
            ..
        } => {
            // Self-sender check
            if matches!(sender, SenderRef::User { .. }) {
                // The target account is resolved by the caller; we can't check
                // self-sender here without knowing the target. The dispatch layer
                // filters self-sender when resolving targets.
            }

            if !prefs.direct_message_enabled && !prefs.group_mention_enabled {
                return Decision::Skip {
                    reason: DecisionReason::PreferenceDisabled,
                };
            }

            Decision::Send {
                payload: PushPayload {
                    title: "New message".into(),
                    body: format!("You have a new message"),
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
            if !prefs.approval_required_enabled {
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
            session_id,
            status,
            ..
        } => {
            if !prefs.agent_session_ended_enabled {
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

        _ => Decision::Skip {
            reason: DecisionReason::NotNotifiable,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::event::SenderRef;

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

    #[test]
    fn conversation_message_sends_when_prefs_enabled() {
        let event = DurableEvent::ConversationMessageAppended {
            conversation_id: "conv1".into(),
            message_id: "msg1".into(),
            sender: SenderRef::User {
                account_id: "other".into(),
            },
            at_ms: 1000,
        };
        let prefs = default_prefs();
        let result = decide(&event, &prefs, 1000);
        assert!(matches!(result, Decision::Send { .. }));
    }

    #[test]
    fn conversation_message_skipped_when_disabled() {
        let event = DurableEvent::ConversationMessageAppended {
            conversation_id: "conv1".into(),
            message_id: "msg1".into(),
            sender: SenderRef::User {
                account_id: "other".into(),
            },
            at_ms: 1000,
        };
        let mut prefs = default_prefs();
        prefs.direct_message_enabled = false;
        prefs.group_mention_enabled = false;
        let result = decide(&event, &prefs, 1000);
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
        // Set quiet hours that cover the current time
        prefs.quiet_hours_start_minute = Some(0);
        prefs.quiet_hours_end_minute = Some(1439);
        let result = decide(&event, &prefs, 1000);
        assert!(matches!(result, Decision::Send { .. }));
    }

    #[test]
    fn message_skipped_during_quiet_hours() {
        let event = DurableEvent::ConversationMessageAppended {
            conversation_id: "conv1".into(),
            message_id: "msg1".into(),
            sender: SenderRef::User {
                account_id: "other".into(),
            },
            at_ms: 1000,
        };
        let mut prefs = default_prefs();
        prefs.quiet_hours_start_minute = Some(0);
        prefs.quiet_hours_end_minute = Some(1439);
        let result = decide(&event, &prefs, 1000);
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::QuietHours
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
        let result = decide(&event, &prefs, 1000);
        assert!(matches!(
            result,
            Decision::Skip {
                reason: DecisionReason::NotNotifiable
            }
        ));
    }
}
