-- Canonical SQLite schema (latest-only).
-- Incremental migration history has been collapsed; wipe local DBs on upgrade.

-- Human accounts are IdP-bound via supabase_sub (no local password).
CREATE TABLE accounts (
    account_id     TEXT PRIMARY KEY,
    email          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    minos_id       TEXT,
    display_name   TEXT,
    -- Supabase Auth subject (JWT `sub`). Required for new users via
    -- POST /v1/auth/supabase exchange. NULL only for rare unbound fixtures.
    supabase_sub   TEXT,
    created_at     INTEGER NOT NULL,
    last_login_at  INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_accounts_email ON accounts(email);
CREATE UNIQUE INDEX idx_accounts_minos_id ON accounts(minos_id COLLATE BINARY);
CREATE UNIQUE INDEX idx_accounts_supabase_sub
    ON accounts(supabase_sub)
    WHERE supabase_sub IS NOT NULL;

-- Client/host installations. `kind` mirrors Postgres installation_kind
-- (mobile|browser|desktop|host). Wire DeviceRole maps:
--   mobile-client→mobile, browser-admin→browser, desktop-console→desktop, agent-host→host.
-- secret_hash removed (device-secret rail retired; host uses host_installation_tokens).
CREATE TABLE device_installations (
    installation_id   TEXT PRIMARY KEY,
    kind              TEXT NOT NULL CHECK (kind IN ('mobile', 'browser', 'desktop', 'host')),
    platform          TEXT,
    public_key        TEXT,
    account_id        TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    display_name      TEXT,
    created_at_ms     INTEGER NOT NULL,
    last_seen_at_ms   INTEGER NOT NULL,
    -- Bootstrap-friendly consistency (host may lack public_key until TOFU;
    -- client may lack account_id until login/exchange bind).
    -- Steady-state: clients have account_id + null public_key; host has null account_id.
    CONSTRAINT installation_kind_account_consistency CHECK (
        (kind IN ('mobile', 'browser', 'desktop') AND public_key IS NULL) OR
        (kind = 'host' AND account_id IS NULL)
    )
) STRICT;

CREATE INDEX idx_installations_account
    ON device_installations(account_id)
    WHERE account_id IS NOT NULL;

CREATE TABLE host_installation_tokens (
    token_hash            TEXT PRIMARY KEY,
    host_installation_id  TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    issued_at_ms          INTEGER NOT NULL,
    last_used_at_ms       INTEGER,
    revoked_at_ms         INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_host_installation_tokens_token_hash
    ON host_installation_tokens(token_hash);
CREATE INDEX idx_host_installation_tokens_host_active
    ON host_installation_tokens(host_installation_id, revoked_at_ms);

CREATE TABLE refresh_tokens (
    token_hash        TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    installation_id   TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    issued_at         INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL,
    revoked_at        INTEGER
) STRICT;

CREATE INDEX idx_refresh_tokens_account
    ON refresh_tokens(account_id)
    WHERE revoked_at IS NULL;
CREATE INDEX idx_refresh_tokens_device
    ON refresh_tokens(installation_id)
    WHERE revoked_at IS NULL;

CREATE TABLE host_links (
    pair_id                    TEXT NOT NULL PRIMARY KEY,
    account_id                 TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    -- Exclusive host ownership: one account per host installation (Host Link).
    host_installation_id       TEXT NOT NULL UNIQUE REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    linked_via_installation_id TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    link_display_name          TEXT,
    acl_json                   TEXT NOT NULL DEFAULT '{}',
    paired_at_ms               INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_host_links_account ON host_links(account_id);

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
    next_message_seq       INTEGER NOT NULL DEFAULT 1,
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
    last_read_seq      INTEGER NOT NULL DEFAULT 0,
    last_read_at_ms    INTEGER NOT NULL DEFAULT 0,
    updated_at_ms      INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
) STRICT;

CREATE INDEX idx_conversation_reads_account
    ON conversation_reads(account_id, updated_at_ms DESC);

CREATE TABLE conversation_deletions (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    deleted_at_ms    INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
) STRICT;

CREATE INDEX idx_conversation_deletions_account
    ON conversation_deletions(account_id, deleted_at_ms DESC);

CREATE TABLE agents (
    agent_id          TEXT PRIMARY KEY,
    owner_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    -- user | host_runtime | system — host_runtime is the stable Desktop/Host mapping
    source            TEXT NOT NULL DEFAULT 'user'
                        CHECK (source IN ('user', 'host_runtime', 'system')),
    runtime_agent     TEXT NOT NULL CHECK (runtime_agent IN ('codex', 'claude', 'gemini', 'opencode', 'grok')),
    model             TEXT NOT NULL DEFAULT '',
    workspace_path    TEXT,
    created_at_ms     INTEGER NOT NULL,
    updated_at_ms     INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_agents_owner
    ON agents(owner_account_id, created_at_ms DESC);

-- One host-runtime projection per (owner, runtime_agent).
CREATE UNIQUE INDEX idx_agents_host_runtime_unique
    ON agents(owner_account_id, runtime_agent)
    WHERE source = 'host_runtime';

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
    message_seq          INTEGER NOT NULL,
    reply_to_message_id  TEXT REFERENCES chat_messages(message_id) ON DELETE SET NULL,
    recalled_at_ms       INTEGER,
    sender_type          TEXT NOT NULL DEFAULT 'user' CHECK (sender_type IN ('user', 'agent')),
    sender_agent_id      TEXT REFERENCES agents(agent_id) ON DELETE CASCADE,
    agent_session_id     TEXT,
    UNIQUE (conversation_id, message_seq)
) STRICT;

CREATE INDEX idx_chat_messages_conversation_seq
    ON chat_messages(conversation_id, message_seq DESC);
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

CREATE TABLE message_reactions (
    reaction_id      TEXT PRIMARY KEY,
    message_id       TEXT NOT NULL REFERENCES chat_messages(message_id) ON DELETE CASCADE,
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    emoji            TEXT NOT NULL,
    actor_kind       TEXT NOT NULL CHECK (actor_kind IN ('user', 'agent')),
    actor_id         TEXT NOT NULL,
    display_name     TEXT NOT NULL,
    created_at_ms    INTEGER NOT NULL,
    UNIQUE (message_id, emoji, actor_kind, actor_id)
) STRICT;

CREATE INDEX idx_message_reactions_message
    ON message_reactions(message_id, emoji);
CREATE INDEX idx_message_reactions_conversation
    ON message_reactions(conversation_id, message_id);

-- Intent Outbox op idempotency for reaction toggle (B6/C5): same client_op_id
-- must not re-toggle message_reactions on HTTP retry.
CREATE TABLE reaction_client_ops (
    client_op_id     TEXT PRIMARY KEY,
    conversation_id  TEXT NOT NULL,
    message_id       TEXT NOT NULL,
    emoji            TEXT NOT NULL,
    action           TEXT NOT NULL CHECK (action IN ('add', 'remove')),
    account_id       TEXT NOT NULL,
    created_at_ms    INTEGER NOT NULL
) STRICT;

CREATE TABLE projects (
    project_id       TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    workspace_slug   TEXT NOT NULL,
    workspace_path   TEXT,
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    UNIQUE(account_id, workspace_slug)
) STRICT;

CREATE INDEX idx_projects_account_updated
    ON projects(account_id, updated_at_ms DESC);

CREATE TABLE agent_sessions (
    session_id        TEXT PRIMARY KEY,
    conversation_id   TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    project_id        TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    host_installation_id TEXT REFERENCES device_installations(installation_id) ON DELETE SET NULL,
    agent_id          TEXT,
    status            TEXT NOT NULL CHECK (status IN ('pending', 'running', 'stopping', 'stopped', 'ended', 'failed')),
    started_at_ms     INTEGER NOT NULL,
    ended_at_ms       INTEGER,
    idempotency_account_id TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    idempotency_key   TEXT
) STRICT;

CREATE INDEX idx_agent_sessions_conversation_status
    ON agent_sessions(conversation_id, status);
CREATE INDEX idx_agent_sessions_project_started
    ON agent_sessions(project_id, started_at_ms DESC)
    WHERE project_id IS NOT NULL;
CREATE UNIQUE INDEX idx_agent_sessions_idempotency
    ON agent_sessions(idempotency_account_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

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

CREATE TABLE approval_requests (
    request_id         TEXT PRIMARY KEY,
    agent_session_id   TEXT NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_id            TEXT REFERENCES agent_turns(turn_id) ON DELETE SET NULL,
    method             TEXT NOT NULL,
    params_json        TEXT NOT NULL,
    state              TEXT NOT NULL CHECK (state IN ('pending', 'decided', 'timeout', 'disconnected')),
    deadline_at_ms     INTEGER NOT NULL,
    created_at_ms      INTEGER NOT NULL,
    resolved_at_ms     INTEGER,
    resolution_json    TEXT,
    -- Client Intent Outbox id for respond idempotency (C5.3). NULL when absent.
    client_request_id  TEXT
) STRICT;

CREATE INDEX idx_approval_session_state
    ON approval_requests(agent_session_id, state);
CREATE INDEX idx_approval_deadline_state
    ON approval_requests(deadline_at_ms, state);
CREATE UNIQUE INDEX idx_approval_client_request_id
    ON approval_requests(client_request_id)
    WHERE client_request_id IS NOT NULL;

CREATE TABLE sessions (
    session_id        TEXT PRIMARY KEY,
    agent            TEXT NOT NULL CHECK (agent IN ('codex', 'claude', 'gemini', 'opencode', 'grok')),
    owner_device_id  TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    title            TEXT,
    first_ts_ms      INTEGER NOT NULL,
    last_ts_ms       INTEGER NOT NULL,
    ended_at_ms      INTEGER,
    end_reason       TEXT,
    message_count    INTEGER NOT NULL DEFAULT 0,
    project_id       TEXT REFERENCES projects(project_id) ON DELETE SET NULL
) STRICT;

CREATE INDEX idx_sessions_last_ts
    ON sessions(last_ts_ms DESC);
CREATE INDEX idx_sessions_owner
    ON sessions(owner_device_id, last_ts_ms DESC);
CREATE INDEX idx_sessions_project_last
    ON sessions(project_id, last_ts_ms DESC)
    WHERE project_id IS NOT NULL;

CREATE TABLE raw_events (
    host_device_id   TEXT NOT NULL DEFAULT '',
    session_id        TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    seq              INTEGER NOT NULL,
    event_id         TEXT NOT NULL DEFAULT '',
    kind             TEXT NOT NULL DEFAULT 'agent_event',
    agent            TEXT NOT NULL CHECK (agent IN ('codex', 'claude', 'gemini', 'opencode', 'grok')),
    payload_json     TEXT NOT NULL,
    ts_ms            INTEGER NOT NULL,
    checksum_sha256  TEXT NOT NULL DEFAULT '',
    byte_len         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (host_device_id, session_id, seq)
) STRICT;

CREATE INDEX idx_raw_events_thread_seq
    ON raw_events(session_id, seq);
CREATE UNIQUE INDEX idx_raw_events_event_id
    ON raw_events(event_id)
    WHERE event_id != '';

CREATE TABLE thread_sync_state (
    host_device_id       TEXT NOT NULL,
    session_id            TEXT NOT NULL,
    backend_acked_seq    INTEGER NOT NULL DEFAULT 0,
    local_from_seq       INTEGER,
    local_to_seq         INTEGER,
    missing_ranges_json  TEXT NOT NULL DEFAULT '[]',
    bytes                INTEGER NOT NULL DEFAULT 0,
    event_count          INTEGER NOT NULL DEFAULT 0,
    first_ts_ms          INTEGER NOT NULL DEFAULT 0,
    last_ts_ms           INTEGER NOT NULL DEFAULT 0,
    running              INTEGER NOT NULL DEFAULT 0,
    updated_at_ms        INTEGER NOT NULL,
    PRIMARY KEY (host_device_id, session_id)
) STRICT;

CREATE TABLE project_sessions (
    project_id    TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    session_id     TEXT NOT NULL,
    account_id    TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    linked_at_ms  INTEGER NOT NULL,
    PRIMARY KEY (project_id, session_id)
) STRICT;

CREATE INDEX idx_project_sessions_account_project
    ON project_sessions(account_id, project_id, linked_at_ms DESC);

CREATE TABLE pending_approvals (
    request_id      TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL,
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
    ON pending_approvals(session_id)
    WHERE resolved_at_ms IS NULL;

CREATE TABLE host_commands (
    command_id                TEXT PRIMARY KEY,
    host_installation_id      TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
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
    event_id        TEXT NOT NULL,
    topic           TEXT NOT NULL,
    topic_kind      TEXT NOT NULL,
    topic_seq       INTEGER NOT NULL,
    partition_key   TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (topic_kind, event_id)
) STRICT;

CREATE UNIQUE INDEX idx_durable_event_log_topic_seq
    ON durable_event_log(topic_kind, topic, topic_seq);
CREATE INDEX idx_durable_event_log_topic_created
    ON durable_event_log(topic, created_at_ms);

CREATE TABLE outbox_events (
    outbox_id         TEXT PRIMARY KEY,
    topic_kind        TEXT NOT NULL,
    event_id          TEXT NOT NULL,
    status            TEXT NOT NULL CHECK (status IN ('pending', 'claimed', 'acked', 'dead')),
    -- social_durable: chat/account/reaction fanout (publish → ack).
    -- host_command: host RPC delivery (publish → async host ack; expire → dead_letter).
    lane              TEXT NOT NULL DEFAULT 'social_durable'
        CHECK (lane IN ('social_durable', 'host_command')),
    available_at_ms   INTEGER NOT NULL,
    attempts          INTEGER NOT NULL DEFAULT 0,
    claimed_by        TEXT,
    claimed_at_ms     INTEGER,
    ack_at_ms         INTEGER,
    dead_at_ms        INTEGER,
    last_error_json   TEXT,
    FOREIGN KEY (topic_kind, event_id) REFERENCES durable_event_log(topic_kind, event_id)
) STRICT;

CREATE INDEX idx_outbox_events_lane_status_available
    ON outbox_events(lane, status, available_at_ms);
CREATE INDEX idx_outbox_events_status_available
    ON outbox_events(status, available_at_ms);
CREATE INDEX idx_outbox_events_event_id
    ON outbox_events(topic_kind, event_id);

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

-- Push idempotency: successful push once per (event_id, account_id).
CREATE TABLE push_dispatch_log (
    event_id     TEXT NOT NULL,
    account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    sent_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (event_id, account_id)
) STRICT;

CREATE INDEX idx_push_dispatch_log_account
    ON push_dispatch_log(account_id);

-- Agent dispatch queue: send_message enqueues; worker drains when host is live.
CREATE TABLE agent_dispatch_queue (
    dispatch_id          TEXT PRIMARY KEY,
    origin_message_id    TEXT NOT NULL UNIQUE,
    conversation_id      TEXT NOT NULL,
    account_id           TEXT NOT NULL,
    agent_id             TEXT NOT NULL,
    session_id           TEXT,
    forwarded_text       TEXT NOT NULL,
    mention_sender       INTEGER NOT NULL DEFAULT 0,
    sender_minos_id      TEXT,
    status               TEXT NOT NULL
        CHECK (status IN ('pending', 'inflight', 'succeeded', 'failed_terminal')),
    attempts             INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ms   INTEGER NOT NULL,
    last_error           TEXT,
    created_at_ms        INTEGER NOT NULL,
    updated_at_ms        INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_agent_dispatch_queue_due
    ON agent_dispatch_queue(status, next_attempt_at_ms);
CREATE INDEX idx_agent_dispatch_queue_conversation
    ON agent_dispatch_queue(conversation_id);
