ALTER TABLE accounts ADD COLUMN minos_id TEXT;
ALTER TABLE accounts ADD COLUMN display_name TEXT;

UPDATE accounts
SET minos_id = lower(hex(randomblob(6)))
WHERE minos_id IS NULL;

CREATE UNIQUE INDEX idx_accounts_minos_id ON accounts(minos_id COLLATE BINARY);

CREATE TABLE friend_requests (
    request_id        TEXT PRIMARY KEY,
    from_account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    to_account_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    status            TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected', 'canceled')),
    created_at_ms     INTEGER NOT NULL,
    resolved_at_ms    INTEGER,
    CHECK (from_account_id <> to_account_id)
) STRICT;

CREATE UNIQUE INDEX idx_friend_requests_pending_pair
ON friend_requests(from_account_id, to_account_id)
WHERE status = 'pending';

CREATE INDEX idx_friend_requests_to_status
ON friend_requests(to_account_id, status, created_at_ms DESC);

CREATE INDEX idx_friend_requests_from_status
ON friend_requests(from_account_id, status, created_at_ms DESC);

CREATE TABLE friendships (
    friendship_id      TEXT PRIMARY KEY,
    account_low_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    account_high_id    TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    created_at_ms      INTEGER NOT NULL,
    CHECK (account_low_id < account_high_id)
) STRICT;

CREATE UNIQUE INDEX idx_friendships_pair
ON friendships(account_low_id, account_high_id);

CREATE INDEX idx_friendships_low
ON friendships(account_low_id, created_at_ms DESC);

CREATE INDEX idx_friendships_high
ON friendships(account_high_id, created_at_ms DESC);

CREATE TABLE conversations (
    conversation_id        TEXT PRIMARY KEY,
    kind                   TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
    title                  TEXT,
    created_by_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_account_low     TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_account_high    TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    created_at_ms          INTEGER NOT NULL,
    updated_at_ms          INTEGER NOT NULL,
    CHECK (
        (kind = 'direct' AND direct_account_low IS NOT NULL AND direct_account_high IS NOT NULL) OR
        (kind = 'group' AND direct_account_low IS NULL AND direct_account_high IS NULL)
    )
) STRICT;

CREATE UNIQUE INDEX idx_conversations_direct_pair
ON conversations(direct_account_low, direct_account_high)
WHERE kind = 'direct';

CREATE INDEX idx_conversations_updated
ON conversations(updated_at_ms DESC);

CREATE TABLE conversation_members (
    conversation_id    TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id         TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    joined_at_ms       INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
) STRICT;

CREATE INDEX idx_conversation_members_account
ON conversation_members(account_id, joined_at_ms DESC);

CREATE TABLE chat_messages (
    message_id          TEXT PRIMARY KEY,
    conversation_id     TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    sender_account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    text                TEXT NOT NULL,
    created_at_ms       INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_chat_messages_conversation_created
ON chat_messages(conversation_id, created_at_ms DESC);
