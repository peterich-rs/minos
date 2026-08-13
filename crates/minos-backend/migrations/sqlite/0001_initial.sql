-- Canonical SQLite schema (latest-only).
-- Logical SSOT shared with postgres/0001_initial.sql (dialect types only differ).
-- Wipe local DBs on upgrade. Storage parity: docs/architecture-backend.md.

-- Human accounts are IdP-bound via supabase_sub (no local password).
CREATE TABLE accounts (
    account_id        TEXT PRIMARY KEY,
    email             TEXT NOT NULL UNIQUE COLLATE NOCASE,
    minos_id          TEXT,
    display_name      TEXT,
    -- Supabase Auth subject (JWT `sub`). Required for new users via
    -- POST /v1/auth/supabase exchange. NULL only for rare unbound fixtures.
    supabase_sub      TEXT,
    created_at_ms     INTEGER NOT NULL,
    last_login_at_ms  INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_accounts_email ON accounts(email);
CREATE UNIQUE INDEX idx_accounts_minos_id ON accounts(minos_id COLLATE BINARY);
CREATE UNIQUE INDEX idx_accounts_supabase_sub
    ON accounts(supabase_sub)
    WHERE supabase_sub IS NOT NULL;

-- Client/host installations. Wire DeviceRole maps:
--   mobile-client→mobile, browser-admin→browser, desktop-console→desktop, agent-host→host.
CREATE TABLE devices (
    device_id   TEXT PRIMARY KEY,
    kind              TEXT NOT NULL CHECK (kind IN ('mobile', 'browser', 'desktop', 'host')),
    platform          TEXT,
    public_key        TEXT,
    account_id        TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    display_name      TEXT,
    created_at_ms     INTEGER NOT NULL,
    last_seen_at_ms   INTEGER NOT NULL,
    -- Strict parity with Postgres: clients require account_id; hosts require public_key.
    CONSTRAINT device_kind_account_consistency CHECK (
        (kind IN ('mobile', 'browser', 'desktop') AND account_id IS NOT NULL AND public_key IS NULL) OR
        (kind = 'host' AND account_id IS NULL AND public_key IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_devices_account
    ON devices(account_id)
    WHERE account_id IS NOT NULL;

CREATE TABLE host_tokens (
    token_hash            TEXT PRIMARY KEY,
    host_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    -- Bound account; required for Desktop login-issued tokens. Host WS auth
    -- rejects a live token that cannot resolve to exactly one account.
    account_id            TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    issued_at_ms          INTEGER NOT NULL,
    last_used_at_ms       INTEGER,
    revoked_at_ms         INTEGER
) STRICT;

CREATE INDEX idx_host_tokens_host_active
    ON host_tokens(host_device_id, revoked_at_ms);

CREATE TABLE refresh_tokens (
    token_hash        TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    device_id   TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    issued_at_ms      INTEGER NOT NULL,
    expires_at_ms     INTEGER NOT NULL,
    revoked_at_ms     INTEGER,
    rotated_to_hash   TEXT REFERENCES refresh_tokens(token_hash) ON DELETE SET NULL
) STRICT;

CREATE INDEX idx_refresh_tokens_account
    ON refresh_tokens(account_id)
    WHERE revoked_at_ms IS NULL;
CREATE INDEX idx_refresh_tokens_device
    ON refresh_tokens(device_id)
    WHERE revoked_at_ms IS NULL;

CREATE TABLE host_links (
    pair_id                    TEXT NOT NULL PRIMARY KEY,
    account_id                 TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    -- Exclusive host ownership: one account per host installation (Host Link).
    host_device_id       TEXT NOT NULL UNIQUE REFERENCES devices(device_id) ON DELETE CASCADE,
    linked_via_device_id TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
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

CREATE TABLE agents (
    agent_id                   TEXT PRIMARY KEY,
    owner_account_id           TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name                       TEXT NOT NULL,
    display_name               TEXT NOT NULL DEFAULT '',
    description                TEXT NOT NULL DEFAULT '',
    avatar_url                 TEXT,
    source                     TEXT NOT NULL DEFAULT 'user'
                                 CHECK (source IN ('user', 'host_runtime', 'system')),
    status                     TEXT NOT NULL DEFAULT 'active'
                                 CHECK (status IN ('active', 'disabled')),
    runtime_agent              TEXT NOT NULL CHECK (runtime_agent IN ('codex', 'claude', 'gemini', 'opencode', 'grok')),
    model                      TEXT NOT NULL DEFAULT '',
    default_reasoning_effort   TEXT NOT NULL DEFAULT '',
    system_prompt              TEXT NOT NULL DEFAULT '',
    workspace_path             TEXT,
    created_at_ms              INTEGER NOT NULL,
    updated_at_ms              INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_agents_owner
    ON agents(owner_account_id, created_at_ms DESC);

CREATE UNIQUE INDEX idx_agents_host_runtime_unique
    ON agents(owner_account_id, runtime_agent)
    WHERE source = 'host_runtime';

-- Case-insensitive unique bot name per owner among active bots (mention resolve).
CREATE UNIQUE INDEX idx_agents_owner_name_active
    ON agents (owner_account_id, lower(name))
    WHERE status = 'active';

CREATE TABLE projects (
    project_id       TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    workspace_slug   TEXT NOT NULL,
    workspace_path   TEXT,
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    archived_at_ms   INTEGER,
    UNIQUE(account_id, workspace_slug)
) STRICT;

CREATE INDEX idx_projects_account_updated
    ON projects(account_id, updated_at_ms DESC);

CREATE TABLE project_members (
    project_id   TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    role         TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    joined_at_ms INTEGER NOT NULL,
    PRIMARY KEY (project_id, account_id)
) STRICT;

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
    -- Bumped on membership changes; clients treat lower versions as stale.
    membership_version     INTEGER NOT NULL DEFAULT 1,
    CHECK (
        (kind = 'direct' AND direct_account_low IS NOT NULL AND direct_account_high IS NOT NULL
                         AND direct_account_low < direct_account_high) OR
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
    role               TEXT NOT NULL DEFAULT 'member'
        CHECK (role IN ('owner', 'admin', 'member')),
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
    -- Human author account; NULL for agent-authored rows (bot is sender_agent_id).
    sender_account_id    TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    text                 TEXT NOT NULL,
    created_at_ms        INTEGER NOT NULL,
    message_seq          INTEGER NOT NULL,
    reply_to_message_id  TEXT REFERENCES chat_messages(message_id) ON DELETE SET NULL,
    recalled_at_ms       INTEGER,
    sender_type          TEXT NOT NULL DEFAULT 'user' CHECK (sender_type IN ('user', 'agent')),
    sender_agent_id      TEXT REFERENCES agents(agent_id) ON DELETE CASCADE,
    agent_session_id     TEXT,
    -- Request provenance for client_message_id fingerprint (idempotency conflict).
    message_source       TEXT NOT NULL DEFAULT 'client_live'
        CHECK (message_source IN ('client_live', 'host_projection', 'system')),
    UNIQUE (conversation_id, message_seq),
    CHECK (
        (sender_type = 'user' AND sender_account_id IS NOT NULL AND sender_agent_id IS NULL)
        OR
        (sender_type = 'agent' AND sender_agent_id IS NOT NULL)
    )
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

-- Polymorphic mention targets: human account or bot agent participant.
-- target_id is account_id when target_kind='account', agent_id when 'agent'.
-- No FK on target_id (cross-kind polymorphic); membership validated at write time.
CREATE TABLE chat_message_mentions (
    message_id   TEXT NOT NULL REFERENCES chat_messages(message_id) ON DELETE CASCADE,
    target_kind  TEXT NOT NULL CHECK (target_kind IN ('account', 'agent')),
    target_id    TEXT NOT NULL,
    -- Body appearance order within (message_id, target_kind); hydrate SSOT.
    ordinal      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (message_id, target_kind, target_id)
) STRICT;

CREATE INDEX idx_chat_message_mentions_target
    ON chat_message_mentions(target_kind, target_id, message_id);
CREATE INDEX idx_chat_message_mentions_message_ordinal
    ON chat_message_mentions(message_id, target_kind, ordinal);

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

CREATE TABLE reaction_client_ops (
    client_op_id     TEXT PRIMARY KEY,
    conversation_id  TEXT NOT NULL,
    message_id       TEXT NOT NULL,
    emoji            TEXT NOT NULL,
    action           TEXT NOT NULL CHECK (action IN ('add', 'remove')),
    account_id       TEXT NOT NULL,
    created_at_ms    INTEGER NOT NULL
) STRICT;

CREATE TABLE agent_sessions (
    session_id        TEXT PRIMARY KEY,
    conversation_id   TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    project_id        TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    host_device_id TEXT REFERENCES devices(device_id) ON DELETE SET NULL,
    agent_id          TEXT REFERENCES agents(agent_id) ON DELETE SET NULL,
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
    usage_json           TEXT,
    UNIQUE (agent_session_id, turn_seq)
) STRICT;

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
    owner_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
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

-- No FK to sessions: ingest may outpace session row creation (parity with Postgres).
CREATE TABLE raw_events (
    host_device_id   TEXT NOT NULL DEFAULT '',
    session_id       TEXT NOT NULL,
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
    session_id           TEXT NOT NULL,
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

CREATE TABLE host_commands (
    command_id                TEXT PRIMARY KEY,
    host_device_id      TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    agent_session_id          TEXT REFERENCES agent_sessions(session_id) ON DELETE SET NULL,
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
    ON host_commands(host_device_id, status, deadline_at_ms);

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

-- Sequence authority for durable topics. Retention may delete log payloads
-- but must never reset high_watermark; retention_floor tracks deleted upper bound.
CREATE TABLE topic_metadata (
    topic_kind         TEXT NOT NULL,
    topic              TEXT NOT NULL,
    high_watermark     INTEGER NOT NULL DEFAULT 0,
    retention_floor    INTEGER NOT NULL DEFAULT 0,
    updated_at_ms      INTEGER NOT NULL,
    PRIMARY KEY (topic_kind, topic),
    CHECK (high_watermark >= 0),
    CHECK (retention_floor >= 0),
    CHECK (retention_floor <= high_watermark)
) STRICT;

CREATE TABLE outbox_events (
    outbox_id         TEXT PRIMARY KEY,
    topic_kind        TEXT NOT NULL,
    event_id          TEXT NOT NULL,
    status            TEXT NOT NULL CHECK (status IN ('pending', 'claimed', 'acked', 'dead')),
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

CREATE TABLE audit_events (
    audit_id         TEXT PRIMARY KEY,
    actor_kind       TEXT NOT NULL,
    account_id       TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    device_id  TEXT REFERENCES devices(device_id) ON DELETE SET NULL,
    event_type       TEXT NOT NULL,
    metadata         TEXT,
    at_ms            INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_audit_at_ms ON audit_events(at_ms DESC);
CREATE INDEX idx_audit_account_at
    ON audit_events(account_id, at_ms DESC)
    WHERE account_id IS NOT NULL;

CREATE TABLE push_tokens (
    token_hash       TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
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

CREATE TABLE push_dispatch_log (
    event_id     TEXT NOT NULL,
    account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    sent_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (event_id, account_id)
) STRICT;

CREATE INDEX idx_push_dispatch_log_account
    ON push_dispatch_log(account_id);

-- Durable push work queue: claim/retry/backoff; push_dispatch_log remains success ledger.
CREATE TABLE push_dispatch_queue (
    queue_id              TEXT PRIMARY KEY,
    event_id              TEXT NOT NULL,
    account_id            TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    topic                 TEXT NOT NULL,
    topic_seq             INTEGER NOT NULL,
    payload_json          TEXT NOT NULL,
    status                TEXT NOT NULL
        CHECK (status IN ('pending', 'claimed', 'sent', 'skipped', 'dead')),
    attempts              INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ms    INTEGER NOT NULL,
    last_error            TEXT,
    provider_message_id   TEXT,
    created_at_ms         INTEGER NOT NULL,
    claimed_by            TEXT,
    claimed_at_ms         INTEGER,
    UNIQUE (event_id, account_id)
) STRICT;

CREATE INDEX idx_push_dispatch_queue_due
    ON push_dispatch_queue(status, next_attempt_at_ms);

-- Bot mailbox (domain: bot_message_deliveries). delivery_id column is dispatch_id
-- for store API continuity; UNIQUE (origin_message_id, agent_id) is the inbox key.
CREATE TABLE bot_message_deliveries (
    dispatch_id          TEXT PRIMARY KEY,
    origin_message_id    TEXT NOT NULL,
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
    updated_at_ms        INTEGER NOT NULL,
    lease_owner_host_id  TEXT,
    lease_expires_at_ms  INTEGER,
    automation_hop       INTEGER NOT NULL DEFAULT 0,
    UNIQUE (origin_message_id, agent_id)
) STRICT;

CREATE INDEX idx_bot_message_deliveries_due
    ON bot_message_deliveries(status, next_attempt_at_ms);
CREATE INDEX idx_bot_message_deliveries_conversation
    ON bot_message_deliveries(conversation_id);
CREATE INDEX idx_bot_message_deliveries_lease
    ON bot_message_deliveries(lease_owner_host_id, lease_expires_at_ms)
    WHERE lease_owner_host_id IS NOT NULL;

-- Durable CompletionWatch: restart-safe turn projection (in-memory is cache).
CREATE TABLE completion_watches (
    watch_key              TEXT PRIMARY KEY,
    dispatch_id            TEXT NOT NULL,
    origin_message_id      TEXT NOT NULL,
    conversation_id        TEXT NOT NULL,
    session_id             TEXT NOT NULL,
    agent_id               TEXT NOT NULL,
    raw_seq_floor          INTEGER NOT NULL,
    armed_at_ms            INTEGER NOT NULL,
    deadline_at_ms         INTEGER NOT NULL,
    status                 TEXT NOT NULL
        CHECK (status IN ('armed', 'projected', 'expired')),
    projected_message_id   TEXT,
    mention_account_id     TEXT,
    mention_minos_id       TEXT
) STRICT;

CREATE INDEX idx_completion_watches_session
    ON completion_watches(session_id, status);
CREATE INDEX idx_completion_watches_deadline
    ON completion_watches(status, deadline_at_ms);

CREATE TABLE media_blobs (
    blob_id             TEXT PRIMARY KEY,
    account_id          TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    object_key          TEXT NOT NULL UNIQUE,
    content_type        TEXT NOT NULL,
    byte_size           INTEGER NOT NULL,
    sha256_hex          TEXT,
    original_filename   TEXT,
    kind                TEXT NOT NULL
        CHECK (kind IN ('image', 'file', 'audio', 'video')),
    status              TEXT NOT NULL
        CHECK (status IN ('pending', 'ready', 'failed', 'deleted')),
    created_at_ms       INTEGER NOT NULL,
    ready_at_ms         INTEGER,
    deleted_at_ms       INTEGER
) STRICT;

CREATE INDEX idx_media_blobs_account_created
    ON media_blobs(account_id, created_at_ms DESC);
CREATE INDEX idx_media_blobs_status
    ON media_blobs(status, created_at_ms);

CREATE TABLE chat_message_attachments (
    message_id   TEXT NOT NULL REFERENCES chat_messages(message_id) ON DELETE CASCADE,
    blob_id      TEXT NOT NULL REFERENCES media_blobs(blob_id) ON DELETE RESTRICT,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (message_id, blob_id)
) STRICT;

CREATE INDEX idx_chat_message_attachments_blob
    ON chat_message_attachments(blob_id);

-- Immutable digital-body snapshot at mailbox schedule time + host capability.
CREATE TABLE bot_revisions (
    revision_id              TEXT PRIMARY KEY,
    agent_id                 TEXT NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    runtime_agent            TEXT NOT NULL,
    model                    TEXT NOT NULL DEFAULT '',
    default_reasoning_effort TEXT NOT NULL DEFAULT '',
    system_prompt            TEXT NOT NULL DEFAULT '',
    display_name             TEXT NOT NULL DEFAULT '',
    workspace_path           TEXT,
    created_at_ms            INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_bot_revisions_agent_created
    ON bot_revisions(agent_id, created_at_ms DESC);

CREATE TABLE bot_deployments (
    agent_id              TEXT NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    host_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    status                TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled')),
    updated_at_ms         INTEGER NOT NULL,
    PRIMARY KEY (agent_id, host_device_id)
) STRICT;

CREATE INDEX idx_bot_deployments_host
    ON bot_deployments(host_device_id, status);

