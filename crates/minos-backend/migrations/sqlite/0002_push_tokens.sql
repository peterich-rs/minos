-- Push tokens table for notification delivery (SQLite).

CREATE TABLE push_tokens (
    token_hash       TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    installation_id  TEXT NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('apns', 'fcm')),
    locale           TEXT,
    created_at_ms    INTEGER NOT NULL,
    last_used_at_ms  INTEGER NOT NULL,
    revoked_at_ms    INTEGER
) STRICT;

CREATE INDEX idx_push_tokens_account
    ON push_tokens(account_id)
    WHERE revoked_at_ms IS NULL;
