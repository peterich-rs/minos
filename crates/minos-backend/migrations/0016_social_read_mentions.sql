CREATE TABLE conversation_reads (
    conversation_id    TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id         TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    last_read_at_ms    INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
) STRICT;

CREATE INDEX idx_conversation_reads_account
ON conversation_reads(account_id, updated_at_ms DESC);

CREATE TABLE chat_message_mentions (
    message_id             TEXT NOT NULL REFERENCES chat_messages(message_id) ON DELETE CASCADE,
    mentioned_account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, mentioned_account_id)
) STRICT;

CREATE INDEX idx_chat_message_mentions_account
ON chat_message_mentions(mentioned_account_id, message_id);