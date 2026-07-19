//! Notification preferences domain type + quiet-hours logic.

use serde::{Deserialize, Serialize};

use crate::store::notification_preferences::NotificationPreferencesRow;

/// Domain-level notification preferences. Decouples the notification
/// decision engine from the raw database row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPreferences {
    pub account_id: String,
    pub direct_message_enabled: bool,
    pub group_mention_enabled: bool,
    pub approval_required_enabled: bool,
    pub agent_session_ended_enabled: bool,
    pub quiet_hours_start_minute: Option<i16>,
    pub quiet_hours_end_minute: Option<i16>,
    pub quiet_hours_timezone: Option<String>,
}

impl NotificationPreferences {
    #[must_use]
    pub fn from_row(row: &NotificationPreferencesRow) -> Self {
        Self {
            account_id: row.account_id.clone(),
            direct_message_enabled: row.direct_message_enabled,
            group_mention_enabled: row.group_mention_enabled,
            approval_required_enabled: row.approval_required_enabled,
            agent_session_ended_enabled: row.agent_session_ended_enabled,
            quiet_hours_start_minute: row.quiet_hours_start_minute,
            quiet_hours_end_minute: row.quiet_hours_end_minute,
            quiet_hours_timezone: row.quiet_hours_timezone.clone(),
        }
    }

    /// Check whether the current time falls within quiet hours.
    ///
    /// `current_minute_of_day` is 0..1439 (hour * 60 + minute).
    /// Returns `true` if quiet hours are active right now.
    #[must_use]
    pub fn is_quiet_hours(&self, current_minute_of_day: i16) -> bool {
        match (self.quiet_hours_start_minute, self.quiet_hours_end_minute) {
            (Some(start), Some(end)) => {
                if start <= end {
                    // Simple range: e.g. 22:00..06:00 wraps, 09:00..17:00 doesn't
                    current_minute_of_day >= start && current_minute_of_day < end
                } else {
                    // Wrapping range: e.g. 1320 (22:00) .. 360 (06:00)
                    current_minute_of_day >= start || current_minute_of_day < end
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs_with_quiet(start: i16, end: i16) -> NotificationPreferences {
        NotificationPreferences {
            account_id: "test".into(),
            direct_message_enabled: true,
            group_mention_enabled: true,
            approval_required_enabled: true,
            agent_session_ended_enabled: false,
            quiet_hours_start_minute: Some(start),
            quiet_hours_end_minute: Some(end),
            quiet_hours_timezone: None,
        }
    }

    #[test]
    fn quiet_hours_simple_range() {
        // 09:00..17:00
        let prefs = prefs_with_quiet(540, 1020);
        assert!(!prefs.is_quiet_hours(480)); // 08:00
        assert!(prefs.is_quiet_hours(540)); // 09:00
        assert!(prefs.is_quiet_hours(600)); // 10:00
        assert!(prefs.is_quiet_hours(1019)); // 16:59
        assert!(!prefs.is_quiet_hours(1020)); // 17:00
    }

    #[test]
    fn quiet_hours_wrapping_range() {
        // 22:00..06:00 (wraps midnight)
        let prefs = prefs_with_quiet(1320, 360);
        assert!(prefs.is_quiet_hours(1320)); // 22:00
        assert!(prefs.is_quiet_hours(0)); // midnight
        assert!(prefs.is_quiet_hours(359)); // 05:59
        assert!(!prefs.is_quiet_hours(360)); // 06:00
        assert!(!prefs.is_quiet_hours(600)); // 10:00
    }

    #[test]
    fn no_quiet_hours_means_never_quiet() {
        let prefs = NotificationPreferences {
            account_id: "test".into(),
            direct_message_enabled: true,
            group_mention_enabled: true,
            approval_required_enabled: true,
            agent_session_ended_enabled: false,
            quiet_hours_start_minute: None,
            quiet_hours_end_minute: None,
            quiet_hours_timezone: None,
        };
        assert!(!prefs.is_quiet_hours(0));
        assert!(!prefs.is_quiet_hours(1439));
    }
}
