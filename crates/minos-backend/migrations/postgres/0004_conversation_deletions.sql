CREATE TABLE conversation_deletions (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    deleted_at_ms    BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
);

CREATE INDEX idx_conversation_deletions_account
    ON conversation_deletions(account_id, deleted_at_ms DESC);
