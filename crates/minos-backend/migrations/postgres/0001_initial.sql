-- Canonical Postgres schema (latest-only).
-- Incremental migration history has been collapsed; wipe local DBs on upgrade.

CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE accounts (
    account_id        TEXT PRIMARY KEY,
    email             CITEXT NOT NULL UNIQUE,
    minos_id          TEXT UNIQUE,
    display_name      TEXT,
    -- Supabase Auth subject (JWT `sub`). NULL for password-only accounts
    -- that have not yet been linked via OIDC exchange.
    supabase_sub      TEXT UNIQUE,
    created_at_ms     BIGINT NOT NULL,
    last_login_at_ms  BIGINT
);

CREATE TABLE account_credentials (
    account_id      TEXT PRIMARY KEY REFERENCES accounts(account_id) ON DELETE CASCADE,
    password_hash   TEXT NOT NULL,
    updated_at_ms   BIGINT NOT NULL
);

CREATE TYPE installation_kind AS ENUM ('mobile', 'browser', 'desktop', 'host');

CREATE TABLE device_installations (
    installation_id   TEXT PRIMARY KEY,
    kind              installation_kind NOT NULL,
    platform          TEXT,
    public_key        TEXT,
    account_id        TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    display_name      TEXT,
    created_at_ms     BIGINT NOT NULL,
    last_seen_at_ms   BIGINT NOT NULL,
    -- desktop behaves like mobile/browser: account_id required, public_key null.
    -- host: account_id null + public_key required (insert with key at TOFU register).
    CONSTRAINT installation_kind_account_consistency CHECK (
        (kind IN ('mobile', 'browser', 'desktop') AND account_id IS NOT NULL AND public_key IS NULL) OR
        (kind = 'host' AND account_id IS NULL AND public_key IS NOT NULL)
    )
);

CREATE INDEX idx_installations_account
    ON device_installations(account_id)
    WHERE account_id IS NOT NULL;

CREATE TABLE refresh_tokens (
    token_hash        TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    installation_id   TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    issued_at_ms      BIGINT NOT NULL,
    expires_at_ms     BIGINT NOT NULL,
    revoked_at_ms     BIGINT,
    rotated_to_hash   TEXT REFERENCES refresh_tokens(token_hash) ON DELETE SET NULL
);

CREATE INDEX idx_refresh_active
    ON refresh_tokens(account_id, installation_id)
    WHERE revoked_at_ms IS NULL;

CREATE TABLE host_installation_tokens (
    token_hash             TEXT PRIMARY KEY,
    host_installation_id   TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    issued_at_ms           BIGINT NOT NULL,
    last_used_at_ms        BIGINT,
    revoked_at_ms          BIGINT
);

CREATE INDEX idx_host_token_active
    ON host_installation_tokens(host_installation_id)
    WHERE revoked_at_ms IS NULL;

CREATE TYPE pairing_status AS ENUM ('pending', 'confirmed', 'redeemed', 'expired');

CREATE TABLE pairing_codes (
    code_hash                  TEXT PRIMARY KEY,
    host_installation_id       TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    account_id                 TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    linked_via_installation_id TEXT REFERENCES device_installations(installation_id) ON DELETE SET NULL,
    status                     pairing_status NOT NULL,
    client_request_id          TEXT,
    created_at_ms              BIGINT NOT NULL,
    expires_at_ms              BIGINT NOT NULL,
    confirmed_at_ms            BIGINT,
    redeemed_at_ms             BIGINT
);

CREATE INDEX idx_pairing_codes_host_status_created
    ON pairing_codes(host_installation_id, status, created_at_ms DESC);
CREATE INDEX idx_pairing_codes_expires
    ON pairing_codes(expires_at_ms)
    WHERE status IN ('pending', 'confirmed');

CREATE TABLE host_links (
    pair_id                    TEXT PRIMARY KEY,
    account_id                 TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    host_installation_id       TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    linked_via_installation_id TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    link_display_name          TEXT,
    acl_json                   JSONB NOT NULL DEFAULT '{}'::jsonb,
    paired_at_ms               BIGINT NOT NULL,
    UNIQUE (account_id, host_installation_id)
);

CREATE INDEX idx_host_links_account ON host_links(account_id);
CREATE INDEX idx_host_links_host ON host_links(host_installation_id);

CREATE TABLE agents (
    agent_id         TEXT PRIMARY KEY,
    runtime_kind     TEXT NOT NULL,
    display_name     TEXT NOT NULL,
    description      TEXT,
    enabled          BOOLEAN NOT NULL DEFAULT TRUE,
    workspace_path   TEXT,
    created_at_ms    BIGINT NOT NULL
);

INSERT INTO agents (agent_id, runtime_kind, display_name, created_at_ms)
VALUES
    ('agent_codex', 'codex', 'Codex', (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT),
    ('agent_claude', 'claude', 'Claude', (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT),
    ('agent_gemini', 'gemini', 'Gemini', (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT),
    ('agent_grok', 'grok', 'Grok', (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT)
ON CONFLICT (agent_id) DO NOTHING;

CREATE TABLE projects (
    project_id       TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    workspace_root   TEXT NOT NULL,
    workspace_path   TEXT,
    created_at_ms    BIGINT NOT NULL,
    updated_at_ms    BIGINT NOT NULL,
    archived_at_ms   BIGINT,
    UNIQUE (account_id, workspace_root)
);

CREATE TABLE project_members (
    project_id   TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    account_id   TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    role         TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    joined_at_ms BIGINT NOT NULL,
    PRIMARY KEY (project_id, account_id)
);

CREATE TABLE project_default_agents (
    project_id  TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    agent_id    TEXT NOT NULL REFERENCES agents(agent_id),
    priority    INT NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, agent_id)
);

CREATE TYPE conversation_kind AS ENUM ('direct', 'group');

CREATE TABLE conversations (
    conversation_id        TEXT PRIMARY KEY,
    kind                   conversation_kind NOT NULL,
    title                  TEXT,
    project_id             TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    created_by_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_account_low     TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    direct_account_high    TEXT REFERENCES accounts(account_id) ON DELETE CASCADE,
    created_at_ms          BIGINT NOT NULL,
    updated_at_ms          BIGINT NOT NULL,
    CONSTRAINT direct_pair_consistency CHECK (
        (kind = 'direct' AND direct_account_low IS NOT NULL AND direct_account_high IS NOT NULL
                         AND direct_account_low < direct_account_high) OR
        (kind = 'group' AND direct_account_low IS NULL AND direct_account_high IS NULL)
    )
);

CREATE UNIQUE INDEX idx_conversations_direct_pair
    ON conversations(direct_account_low, direct_account_high)
    WHERE kind = 'direct';
CREATE INDEX idx_conversations_updated_at ON conversations(updated_at_ms DESC);

CREATE TABLE conversation_members (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    joined_at_ms     BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
);

CREATE INDEX idx_conv_members_account
    ON conversation_members(account_id, joined_at_ms DESC);

CREATE TABLE conversation_messages (
    message_id           TEXT PRIMARY KEY,
    conversation_id      TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    sender_kind          TEXT NOT NULL CHECK (sender_kind IN ('user', 'agent', 'system')),
    sender_account_id    TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    sender_agent_id      TEXT REFERENCES agents(agent_id),
    body_json            JSONB NOT NULL,
    reply_to_message_id  TEXT REFERENCES conversation_messages(message_id) ON DELETE SET NULL,
    agent_session_id     TEXT,
    created_at_ms        BIGINT NOT NULL,
    recalled_at_ms       BIGINT,
    CONSTRAINT message_sender_consistency CHECK (
        (sender_kind = 'user' AND sender_account_id IS NOT NULL AND sender_agent_id IS NULL) OR
        (sender_kind = 'agent' AND sender_agent_id IS NOT NULL) OR
        (sender_kind = 'system' AND sender_account_id IS NULL AND sender_agent_id IS NULL)
    )
);

CREATE INDEX idx_conv_msgs_conv_created
    ON conversation_messages(conversation_id, created_at_ms DESC);
CREATE INDEX idx_conv_msgs_session
    ON conversation_messages(agent_session_id)
    WHERE agent_session_id IS NOT NULL;

CREATE TABLE conversation_reads (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    last_read_at_ms  BIGINT NOT NULL,
    updated_at_ms    BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
);

CREATE TABLE conversation_deletions (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    deleted_at_ms    BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
);

CREATE INDEX idx_conversation_deletions_account
    ON conversation_deletions(account_id, deleted_at_ms DESC);

CREATE TABLE message_mentions (
    message_id            TEXT NOT NULL REFERENCES conversation_messages(message_id) ON DELETE CASCADE,
    mentioned_account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, mentioned_account_id)
);

CREATE TYPE agent_session_status AS ENUM ('pending', 'running', 'stopping', 'stopped', 'ended', 'failed');

CREATE TABLE agent_sessions (
    session_id               TEXT PRIMARY KEY,
    conversation_id          TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    project_id               TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    host_installation_id     TEXT REFERENCES device_installations(installation_id) ON DELETE SET NULL,
    agent_id                 TEXT NOT NULL REFERENCES agents(agent_id),
    status                   agent_session_status NOT NULL,
    started_at_ms            BIGINT NOT NULL,
    ended_at_ms              BIGINT,
    idempotency_account_id   TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    idempotency_key          TEXT
);

CREATE INDEX idx_agent_sessions_conv_status
    ON agent_sessions(conversation_id, status);
CREATE INDEX idx_agent_sessions_project_started
    ON agent_sessions(project_id, started_at_ms DESC)
    WHERE project_id IS NOT NULL;
CREATE UNIQUE INDEX idx_agent_sessions_idempotency
    ON agent_sessions(idempotency_account_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE TYPE turn_role AS ENUM ('user', 'assistant', 'tool', 'system');
CREATE TYPE turn_status AS ENUM ('pending', 'streaming', 'completed', 'failed', 'canceled');

CREATE TABLE agent_turns (
    turn_id            TEXT PRIMARY KEY,
    agent_session_id   TEXT NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_seq           BIGINT NOT NULL,
    role               turn_role NOT NULL,
    status             turn_status NOT NULL,
    started_at_ms      BIGINT NOT NULL,
    finished_at_ms     BIGINT,
    summary_text       TEXT,
    usage_json         JSONB,
    UNIQUE (agent_session_id, turn_seq)
);

CREATE TABLE agent_turn_events (
    turn_id        TEXT NOT NULL REFERENCES agent_turns(turn_id) ON DELETE CASCADE,
    event_seq      BIGINT NOT NULL,
    kind           TEXT NOT NULL,
    payload_json   JSONB NOT NULL,
    created_at_ms  BIGINT NOT NULL,
    PRIMARY KEY (turn_id, event_seq)
);

CREATE INDEX idx_turn_events_turn_created
    ON agent_turn_events(turn_id, created_at_ms);

CREATE TYPE approval_state AS ENUM ('pending', 'decided', 'timeout', 'disconnected');

CREATE TABLE approval_requests (
    request_id        TEXT PRIMARY KEY,
    agent_session_id  TEXT NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_id           TEXT REFERENCES agent_turns(turn_id) ON DELETE SET NULL,
    method            TEXT NOT NULL,
    params_json       JSONB NOT NULL,
    state             approval_state NOT NULL,
    deadline_at_ms    BIGINT NOT NULL,
    created_at_ms     BIGINT NOT NULL,
    resolved_at_ms    BIGINT,
    resolution_json   JSONB
);

CREATE INDEX idx_approval_session_state
    ON approval_requests(agent_session_id, state);
CREATE INDEX idx_approval_deadline_state
    ON approval_requests(deadline_at_ms, state);

CREATE TYPE host_command_status AS ENUM ('pending', 'acked', 'succeeded', 'failed');

CREATE TABLE host_commands (
    command_id               TEXT PRIMARY KEY,
    host_installation_id     TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    agent_session_id         TEXT REFERENCES agent_sessions(session_id) ON DELETE SET NULL,
    method                   TEXT NOT NULL,
    params_json              JSONB NOT NULL,
    requested_by_account_id  TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    status                   host_command_status NOT NULL,
    response_json            JSONB,
    error_json               JSONB,
    deadline_at_ms           BIGINT NOT NULL,
    created_at_ms            BIGINT NOT NULL,
    ack_at_ms                BIGINT,
    finished_at_ms           BIGINT
);

CREATE INDEX idx_host_commands_host_status_deadline
    ON host_commands(host_installation_id, status, deadline_at_ms);

CREATE TABLE durable_event_log (
    event_id       TEXT NOT NULL,
    topic          TEXT NOT NULL,
    topic_kind     TEXT NOT NULL,
    topic_seq      BIGINT NOT NULL,
    partition_key  TEXT NOT NULL,
    payload_json   JSONB NOT NULL,
    created_at_ms  BIGINT NOT NULL,
    PRIMARY KEY (topic_kind, event_id),
    UNIQUE (topic_kind, topic, topic_seq)
) PARTITION BY LIST (topic_kind);

CREATE TABLE durable_event_log_account
    PARTITION OF durable_event_log FOR VALUES IN ('account');
CREATE TABLE durable_event_log_conversation
    PARTITION OF durable_event_log FOR VALUES IN ('conversation');
CREATE TABLE durable_event_log_project
    PARTITION OF durable_event_log FOR VALUES IN ('project');
CREATE TABLE durable_event_log_agent_session
    PARTITION OF durable_event_log FOR VALUES IN ('agent_session');
CREATE TABLE durable_event_log_host
    PARTITION OF durable_event_log FOR VALUES IN ('host');

CREATE INDEX idx_durable_topic_created
    ON durable_event_log(topic, created_at_ms);

CREATE TYPE outbox_status AS ENUM ('pending', 'claimed', 'acked', 'dead');

CREATE TABLE outbox_events (
    outbox_id        TEXT PRIMARY KEY,
    topic_kind       TEXT NOT NULL,
    event_id         TEXT NOT NULL,
    status           outbox_status NOT NULL,
    available_at_ms  BIGINT NOT NULL,
    attempts         INT NOT NULL DEFAULT 0,
    claimed_by       TEXT,
    claimed_at_ms    BIGINT,
    ack_at_ms        BIGINT,
    dead_at_ms       BIGINT,
    last_error_json  JSONB,
    FOREIGN KEY (topic_kind, event_id) REFERENCES durable_event_log(topic_kind, event_id)
);

CREATE INDEX idx_outbox_status_avail
    ON outbox_events(status, available_at_ms);
CREATE INDEX idx_outbox_event_id
    ON outbox_events(topic_kind, event_id);

CREATE TABLE audit_events (
    audit_id         TEXT PRIMARY KEY,
    actor_kind       TEXT NOT NULL,
    account_id       TEXT REFERENCES accounts(account_id) ON DELETE SET NULL,
    installation_id  TEXT REFERENCES device_installations(installation_id) ON DELETE SET NULL,
    event_type       TEXT NOT NULL,
    metadata         JSONB,
    at_ms            BIGINT NOT NULL
);

CREATE INDEX idx_audit_at_ms ON audit_events(at_ms DESC);
CREATE INDEX idx_audit_account_at
    ON audit_events(account_id, at_ms DESC)
    WHERE account_id IS NOT NULL;

CREATE TYPE push_kind AS ENUM ('apns', 'fcm');

CREATE TABLE push_tokens (
    token_hash       TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    installation_id  TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    kind             push_kind NOT NULL,
    locale           TEXT,
    created_at_ms    BIGINT NOT NULL,
    last_used_at_ms  BIGINT NOT NULL,
    revoked_at_ms    BIGINT
);

CREATE INDEX idx_push_tokens_account
    ON push_tokens(account_id)
    WHERE revoked_at_ms IS NULL;

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
