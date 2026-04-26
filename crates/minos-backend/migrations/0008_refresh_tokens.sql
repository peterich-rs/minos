CREATE TABLE refresh_tokens (
    token_hash     TEXT PRIMARY KEY,
    account_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    device_id      TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    issued_at      INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    revoked_at     INTEGER
) STRICT;

CREATE INDEX idx_refresh_tokens_account ON refresh_tokens(account_id) WHERE revoked_at IS NULL;
CREATE INDEX idx_refresh_tokens_device ON refresh_tokens(device_id) WHERE revoked_at IS NULL;
