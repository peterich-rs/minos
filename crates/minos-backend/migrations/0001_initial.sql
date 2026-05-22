-- Formal-development baseline schema.
--
-- The MVP incremental migrations have been collapsed into a single
-- bootstrapping schema so new environments start from the current model.

CREATE TABLE accounts (
    account_id     TEXT PRIMARY KEY,
    email          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    minos_id       TEXT,
    display_name   TEXT,
    password_hash  TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    last_login_at  INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_accounts_email ON accounts(email);
CREATE UNIQUE INDEX idx_accounts_minos_id ON accounts(minos_id COLLATE BINARY);

CREATE TABLE devices (
    device_id      TEXT PRIMARY KEY,
    display_name   TEXT NOT NULL,
    role           TEXT NOT NULL CHECK (role IN ('agent-host', 'mobile-client', 'browser-admin')),
    secret_hash    TEXT,
    public_key     TEXT,
    created_at     INTEGER NOT NULL,
    last_seen_at   INTEGER NOT NULL,
    account_id     TEXT REFERENCES accounts(account_id)
) STRICT;

CREATE INDEX idx_devices_account ON devices(account_id) WHERE account_id IS NOT NULL;

CREATE TABLE pairing_tokens (
    token_hash        TEXT PRIMARY KEY,
    issuer_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    created_at        INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL,
    consumed_at       INTEGER
) STRICT;

CREATE INDEX idx_pairing_tokens_expires
    ON pairing_tokens(expires_at)
    WHERE consumed_at IS NULL;

CREATE TABLE pairing_codes (
    code_hash                   TEXT PRIMARY KEY,
    host_installation_id        TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    account_id                  TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    linked_via_installation_id  TEXT REFERENCES devices(device_id) ON DELETE SET NULL,
    status                      TEXT NOT NULL CHECK (status IN ('pending', 'confirmed', 'redeemed', 'expired')),
    client_request_id           TEXT,
    created_at_ms               INTEGER NOT NULL,
    expires_at_ms               INTEGER NOT NULL,
    confirmed_at_ms             INTEGER,
    redeemed_at_ms              INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_pairing_codes_code_hash
    ON pairing_codes(code_hash);
CREATE INDEX idx_pairing_codes_host_status_created
    ON pairing_codes(host_installation_id, status, created_at_ms DESC);

CREATE TABLE host_installation_tokens (
    token_hash            TEXT PRIMARY KEY,
    host_installation_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    issued_at_ms          INTEGER NOT NULL,
    last_used_at_ms       INTEGER,
    revoked_at_ms         INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_host_installation_tokens_token_hash
    ON host_installation_tokens(token_hash);
CREATE INDEX idx_host_installation_tokens_host_active
    ON host_installation_tokens(host_installation_id, revoked_at_ms);

CREATE TABLE refresh_tokens (
    token_hash     TEXT PRIMARY KEY,
    account_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    device_id      TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    issued_at      INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    revoked_at     INTEGER
) STRICT;

CREATE INDEX idx_refresh_tokens_account
    ON refresh_tokens(account_id)
    WHERE revoked_at IS NULL;
CREATE INDEX idx_refresh_tokens_device
    ON refresh_tokens(device_id)
    WHERE revoked_at IS NULL;

CREATE TABLE account_host_pairings (
    pair_id               TEXT NOT NULL PRIMARY KEY,
    host_device_id        TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    mobile_account_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    paired_via_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    paired_at_ms          INTEGER NOT NULL,
    UNIQUE (host_device_id, mobile_account_id)
) STRICT;

CREATE INDEX idx_account_host_pairings_account
    ON account_host_pairings(mobile_account_id);
CREATE INDEX idx_account_host_pairings_host
    ON account_host_pairings(host_device_id);

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

CREATE TABLE conversation_reads (
    conversation_id    TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id         TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    last_read_at_ms    INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
) STRICT;

CREATE INDEX idx_conversation_reads_account
    ON conversation_reads(account_id, updated_at_ms DESC);

CREATE TABLE agents (
    agent_id          TEXT PRIMARY KEY,
    owner_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    runtime_agent     TEXT NOT NULL CHECK (runtime_agent IN ('codex', 'claude', 'gemini')),
    model             TEXT NOT NULL DEFAULT '',
    created_at_ms     INTEGER NOT NULL,
    updated_at_ms     INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_agents_owner
    ON agents(owner_account_id, created_at_ms DESC);

CREATE TABLE conversation_agent_members (
    conversation_id     TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    agent_id            TEXT NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    added_by_account_id TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    joined_at_ms        INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, agent_id)
) STRICT;

CREATE INDEX idx_conversation_agent_members_agent
    ON conversation_agent_members(agent_id, joined_at_ms DESC);

CREATE TABLE chat_messages (
    message_id           TEXT PRIMARY KEY,
    conversation_id      TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    sender_account_id    TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    text                 TEXT NOT NULL,
    created_at_ms        INTEGER NOT NULL,
    reply_to_message_id  TEXT REFERENCES chat_messages(message_id) ON DELETE SET NULL,
    recalled_at_ms       INTEGER,
    sender_type          TEXT NOT NULL DEFAULT 'user' CHECK (sender_type IN ('user', 'agent')),
    sender_agent_id      TEXT REFERENCES agents(agent_id) ON DELETE CASCADE,
    agent_session_id     TEXT
) STRICT;

CREATE INDEX idx_chat_messages_conversation_created
    ON chat_messages(conversation_id, created_at_ms DESC);
CREATE INDEX idx_chat_messages_reply_to
    ON chat_messages(reply_to_message_id)
    WHERE reply_to_message_id IS NOT NULL;
CREATE INDEX idx_chat_messages_agent_session
    ON chat_messages(agent_session_id)
    WHERE agent_session_id IS NOT NULL;

CREATE TABLE chat_message_mentions (
    message_id            TEXT NOT NULL REFERENCES chat_messages(message_id) ON DELETE CASCADE,
    mentioned_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, mentioned_account_id)
) STRICT;

CREATE INDEX idx_chat_message_mentions_account
    ON chat_message_mentions(mentioned_account_id, message_id);

CREATE TABLE projects (
    project_id      TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    workspace_slug  TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL,
    UNIQUE(account_id, workspace_slug)
) STRICT;

CREATE INDEX idx_projects_account_updated
    ON projects(account_id, updated_at_ms DESC);

CREATE TABLE agent_sessions (
    session_id        TEXT PRIMARY KEY,
    conversation_id   TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    project_id        TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    host_device_id    TEXT REFERENCES devices(device_id) ON DELETE SET NULL,
    agent_id          TEXT,
    status            TEXT NOT NULL CHECK (status IN ('pending', 'running', 'stopping', 'stopped', 'ended', 'failed')),
    started_at_ms     INTEGER NOT NULL,
    ended_at_ms       INTEGER
) STRICT;

CREATE INDEX idx_agent_sessions_conversation_status
    ON agent_sessions(conversation_id, status);
CREATE INDEX idx_agent_sessions_project_started
    ON agent_sessions(project_id, started_at_ms DESC)
    WHERE project_id IS NOT NULL;

CREATE TABLE agent_turns (
    turn_id              TEXT PRIMARY KEY,
    agent_session_id     TEXT NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_seq             INTEGER NOT NULL,
    role                 TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'system')),
    status               TEXT NOT NULL CHECK (status IN ('pending', 'streaming', 'completed', 'failed', 'canceled')),
    started_at_ms        INTEGER NOT NULL,
    finished_at_ms       INTEGER,
    summary_text         TEXT,
    usage_json           TEXT
) STRICT;

CREATE UNIQUE INDEX idx_agent_turns_session_seq
    ON agent_turns(agent_session_id, turn_seq);

CREATE TABLE agent_turn_events (
    turn_id         TEXT NOT NULL REFERENCES agent_turns(turn_id) ON DELETE CASCADE,
    event_seq       INTEGER NOT NULL,
    kind            TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (turn_id, event_seq)
) STRICT;

CREATE INDEX idx_agent_turn_events_turn_created
    ON agent_turn_events(turn_id, created_at_ms);

CREATE TABLE threads (
    thread_id        TEXT PRIMARY KEY,
    agent            TEXT NOT NULL CHECK (agent IN ('codex', 'claude', 'gemini')),
    owner_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    title            TEXT,
    first_ts_ms      INTEGER NOT NULL,
    last_ts_ms       INTEGER NOT NULL,
    ended_at_ms      INTEGER,
    end_reason       TEXT,
    message_count    INTEGER NOT NULL DEFAULT 0,
    project_id       TEXT REFERENCES projects(project_id) ON DELETE SET NULL
) STRICT;

CREATE INDEX idx_threads_last_ts
    ON threads(last_ts_ms DESC);
CREATE INDEX idx_threads_owner
    ON threads(owner_device_id, last_ts_ms DESC);
CREATE INDEX idx_threads_project_last
    ON threads(project_id, last_ts_ms DESC)
    WHERE project_id IS NOT NULL;

CREATE TABLE raw_events (
    thread_id     TEXT NOT NULL REFERENCES threads(thread_id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,
    agent         TEXT NOT NULL CHECK (agent IN ('codex', 'claude', 'gemini')),
    payload_json  TEXT NOT NULL,
    ts_ms         INTEGER NOT NULL,
    PRIMARY KEY (thread_id, seq)
) STRICT;

CREATE INDEX idx_raw_events_thread_seq
    ON raw_events(thread_id, seq);

CREATE TABLE project_threads (
    project_id    TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    thread_id     TEXT NOT NULL,
    account_id    TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    linked_at_ms  INTEGER NOT NULL,
    PRIMARY KEY (project_id, thread_id)
) STRICT;

CREATE INDEX idx_project_threads_account_project
    ON project_threads(account_id, project_id, linked_at_ms DESC);

CREATE TABLE pending_approvals (
    request_id      TEXT PRIMARY KEY,
    thread_id       TEXT NOT NULL,
    turn_id         TEXT NOT NULL,
    host_device_id  TEXT NOT NULL,
    method          TEXT NOT NULL,
    params_json     TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    timeout_at_ms   INTEGER NOT NULL,
    resolved_at_ms  INTEGER,
    resolution      TEXT CHECK (
        resolution IS NULL OR resolution IN ('user_decision', 'timeout', 'disconnected')
    )
) STRICT;

CREATE INDEX idx_pending_approvals_timeout
    ON pending_approvals(timeout_at_ms)
    WHERE resolved_at_ms IS NULL;
CREATE INDEX idx_pending_approvals_thread
    ON pending_approvals(thread_id)
    WHERE resolved_at_ms IS NULL;

CREATE TABLE host_commands (
    command_id                TEXT PRIMARY KEY,
    host_installation_id      TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    agent_session_id          TEXT,
    method                    TEXT NOT NULL,
    params_json               TEXT NOT NULL,
    requested_by_account_id   TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    status                    TEXT NOT NULL CHECK (status IN ('pending', 'acked', 'succeeded', 'failed')),
    response_json             TEXT,
    error_json                TEXT,
    deadline_at_ms            INTEGER NOT NULL,
    created_at_ms             INTEGER NOT NULL,
    ack_at_ms                 INTEGER,
    finished_at_ms            INTEGER
) STRICT;

CREATE INDEX idx_host_commands_host_status_deadline
    ON host_commands(host_installation_id, status, deadline_at_ms);

CREATE TABLE durable_event_log (
    event_id        TEXT PRIMARY KEY,
    topic           TEXT NOT NULL,
    topic_seq       INTEGER NOT NULL,
    partition_key   TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX idx_durable_event_log_topic_seq
    ON durable_event_log(topic, topic_seq);
CREATE INDEX idx_durable_event_log_topic_created
    ON durable_event_log(topic, created_at_ms);

CREATE TABLE outbox_events (
    outbox_id         TEXT PRIMARY KEY,
    event_id          TEXT NOT NULL REFERENCES durable_event_log(event_id),
    status            TEXT NOT NULL CHECK (status IN ('pending', 'claimed', 'acked', 'dead')),
    available_at_ms   INTEGER NOT NULL,
    attempts          INTEGER NOT NULL DEFAULT 0,
    claimed_by        TEXT,
    claimed_at_ms     INTEGER,
    ack_at_ms         INTEGER,
    dead_at_ms        INTEGER
) STRICT;

CREATE INDEX idx_outbox_events_status_available
    ON outbox_events(status, available_at_ms);
CREATE INDEX idx_outbox_events_event_id
    ON outbox_events(event_id);
