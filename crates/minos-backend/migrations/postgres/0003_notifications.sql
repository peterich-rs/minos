-- Notification preferences and cooldown tables.

CREATE TABLE notification_preferences (
    account_id                  TEXT PRIMARY KEY REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_message_enabled      BOOLEAN NOT NULL DEFAULT TRUE,
    group_mention_enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    approval_required_enabled   BOOLEAN NOT NULL DEFAULT TRUE,
    agent_session_ended_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    quiet_hours_start_minute    SMALLINT,
    quiet_hours_end_minute      SMALLINT,
    quiet_hours_timezone        TEXT,
    updated_at_ms               BIGINT NOT NULL
);

CREATE TABLE notification_cooldowns (
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    cooldown_key     TEXT NOT NULL,
    last_sent_at_ms  BIGINT NOT NULL,
    PRIMARY KEY (account_id, cooldown_key)
);

CREATE INDEX idx_notif_cooldowns_last_sent
    ON notification_cooldowns(last_sent_at_ms);
