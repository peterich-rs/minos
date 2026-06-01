-- Notification preferences and cooldown tables (SQLite).

CREATE TABLE notification_preferences (
    account_id                  TEXT PRIMARY KEY REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_message_enabled      INTEGER NOT NULL DEFAULT 1,
    group_mention_enabled       INTEGER NOT NULL DEFAULT 1,
    approval_required_enabled   INTEGER NOT NULL DEFAULT 1,
    agent_session_ended_enabled INTEGER NOT NULL DEFAULT 0,
    quiet_hours_start_minute    INTEGER,
    quiet_hours_end_minute      INTEGER,
    quiet_hours_timezone        TEXT,
    updated_at_ms               INTEGER NOT NULL
) STRICT;

CREATE TABLE notification_cooldowns (
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    cooldown_key     TEXT NOT NULL,
    last_sent_at_ms  INTEGER NOT NULL,
    PRIMARY KEY (account_id, cooldown_key)
) STRICT;

CREATE INDEX idx_notif_cooldowns_last_sent
    ON notification_cooldowns(last_sent_at_ms);
