-- Agents registered by an account. Each agent gets a unique agent_id
-- that acts like a user account_id within conversations.
-- Agents are owned by the account that created them.
CREATE TABLE agents (
    agent_id           TEXT PRIMARY KEY,
    owner_account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name               TEXT NOT NULL,
    description        TEXT NOT NULL DEFAULT '',
    runtime_agent      TEXT NOT NULL CHECK (runtime_agent IN ('codex', 'claude', 'gemini')),
    model              TEXT NOT NULL DEFAULT '',
    created_at_ms      INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_agents_owner ON agents(owner_account_id, created_at_ms DESC);

-- Agent membership in group conversations.
-- An agent can be added to a group by any member of that group.
CREATE TABLE conversation_agent_members (
    conversation_id    TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    agent_id           TEXT NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    added_by_account_id TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    joined_at_ms       INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, agent_id)
) STRICT;

CREATE INDEX idx_conversation_agent_members_agent
ON conversation_agent_members(agent_id, joined_at_ms DESC);

-- Add sender_type to chat_messages to distinguish user vs agent messages.
-- Default 'user' for backward compatibility with existing messages.
ALTER TABLE chat_messages ADD COLUMN sender_type TEXT NOT NULL DEFAULT 'user'
    CHECK (sender_type IN ('user', 'agent'));
