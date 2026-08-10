-- Minos daemon local SQLite schema (latest-only single migration).
-- Wipe the local DB and reopen to apply; no incremental ALTER chain.

CREATE TABLE schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  INTEGER NOT NULL
);

CREATE TABLE workspaces (
    root           TEXT PRIMARY KEY,
    first_seen_at  INTEGER NOT NULL,
    last_seen_at   INTEGER NOT NULL
);

CREATE TABLE projects (
    project_id      TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    workspace_slug  TEXT NOT NULL UNIQUE,
    workspace_path  TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    CHECK(length(name) > 0),
    CHECK(length(workspace_slug) > 0)
);

CREATE INDEX projects_by_updated
    ON projects(updated_at DESC, project_id);

CREATE INDEX projects_by_workspace_path
    ON projects(workspace_path)
    WHERE workspace_path IS NOT NULL;

CREATE TABLE conversations (
    conversation_id       TEXT PRIMARY KEY,
    project_id            TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    title                 TEXT NOT NULL,
    last_message_preview  TEXT,
    message_count         INTEGER NOT NULL DEFAULT 0,
    agent_session_count   INTEGER NOT NULL DEFAULT 0,
    created_at_ms         INTEGER NOT NULL,
    -- Last top-level chat message activity (message upsert only; not title/git).
    updated_at_ms         INTEGER NOT NULL,
    -- Product metadata (priority / workflow / git work-unit binding)
    priority              TEXT,
    progress              TEXT NOT NULL DEFAULT 'todo',
    branch                TEXT,
    worktree_path         TEXT,
    -- inherit | worktree (null = legacy inherit snapshot)
    git_mode              TEXT,
    -- Cached live git flags (refreshed by git_get_status / agent start)
    git_dirty             INTEGER,
    git_head              TEXT,
    CHECK(length(title) > 0),
    CHECK(message_count >= 0),
    CHECK(agent_session_count >= 0),
    CHECK(git_mode IS NULL OR git_mode IN ('inherit', 'worktree')),
    CHECK(git_dirty IS NULL OR git_dirty IN (0, 1))
);

CREATE INDEX conversations_by_project_updated
    ON conversations(project_id, updated_at_ms DESC, conversation_id);

-- Host-local conversation roster keyed by bot identity (not runtime string).
-- Mentions / starts are gated on membership; sessions alone do not imply membership.
-- Runtime is resolved from bot_identities at query time.
-- `brief` is a short peer-facing role description for multi-agent coordination.
CREATE TABLE conversation_agent_members (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    bot_id           TEXT NOT NULL,
    joined_at_ms     INTEGER NOT NULL,
    brief            TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (conversation_id, bot_id),
    CHECK(length(bot_id) > 0),
    CHECK(length(brief) <= 500)
);

CREATE INDEX conversation_agent_members_by_bot
    ON conversation_agent_members(bot_id, conversation_id);

CREATE TABLE sessions (
    session_id            TEXT PRIMARY KEY,
    conversation_id       TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    workspace_root        TEXT NOT NULL REFERENCES workspaces(root),
    parent_session_id     TEXT REFERENCES sessions(session_id) ON DELETE CASCADE,
    -- Runtime agent label used for CLI launch (execution detail, not identity).
    agent                 TEXT NOT NULL,
    -- Bot identity this session belongs to (nullable for legacy/direct paths).
    bot_id                TEXT,
    provider_session_id   TEXT,
    status                TEXT NOT NULL CHECK(status IN (
        'starting', 'idle', 'running', 'resuming', 'suspended', 'closed'
    )),
    last_pause_reason     TEXT,
    last_close_reason     TEXT,
    last_seq              INTEGER NOT NULL DEFAULT 0,
    -- Auto-inject continue after host process death when session was mid-flight
    needs_continue        INTEGER NOT NULL DEFAULT 0,
    started_at            INTEGER NOT NULL,
    last_activity_at      INTEGER NOT NULL,
    ended_at              INTEGER,
    CHECK(length(agent) > 0),
    CHECK(last_seq >= 0)
);

CREATE INDEX sessions_by_conversation_bot_last
    ON sessions(conversation_id, bot_id, last_activity_at DESC, session_id)
    WHERE bot_id IS NOT NULL;

CREATE INDEX sessions_by_conversation_last
    ON sessions(conversation_id, last_activity_at DESC, session_id);

CREATE INDEX sessions_by_conversation_agent_last
    ON sessions(conversation_id, agent, last_activity_at DESC, session_id);

CREATE INDEX sessions_by_parent
    ON sessions(parent_session_id, last_activity_at DESC, session_id)
    WHERE parent_session_id IS NOT NULL;

CREATE INDEX sessions_by_workspace
    ON sessions(workspace_root, last_activity_at DESC);

CREATE INDEX sessions_by_status
    ON sessions(status, last_activity_at DESC);

CREATE TABLE chat_messages (
    message_seq          INTEGER PRIMARY KEY,
    message_id           TEXT NOT NULL UNIQUE,
    conversation_id      TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    session_id           TEXT REFERENCES sessions(session_id) ON DELETE SET NULL,
    created_at_ms        INTEGER NOT NULL,
    sender_role          TEXT NOT NULL CHECK(sender_role IN ('user', 'agent', 'system')),
    -- Bot identity for agent-authored rows (not runtime string).
    bot_id               TEXT,
    body                 TEXT NOT NULL,
    -- Delegation / mention metadata
    reply_to_message_id  TEXT,
    delegation_id        TEXT,
    mentions_json        TEXT NOT NULL DEFAULT '[]',
    CHECK(length(message_id) > 0),
    CHECK(
        (sender_role IN ('user', 'system') AND session_id IS NULL AND bot_id IS NULL)
        OR
        (sender_role = 'agent' AND session_id IS NOT NULL AND bot_id IS NOT NULL)
    )
);

CREATE INDEX chat_messages_by_conversation_seq
    ON chat_messages(conversation_id, message_seq DESC);

CREATE INDEX chat_messages_by_session_seq
    ON chat_messages(session_id, message_seq DESC)
    WHERE session_id IS NOT NULL;

CREATE INDEX chat_messages_by_delegation
    ON chat_messages(conversation_id, delegation_id)
    WHERE delegation_id IS NOT NULL;

-- Domain-neutral local reactions on conversation timeline messages (not Nostr kind:7).
-- Host is a single local user: actor_id = 'local', actor_kind = 'user'.
CREATE TABLE chat_message_reactions (
    reaction_id     TEXT PRIMARY KEY,
    message_id      TEXT NOT NULL REFERENCES chat_messages(message_id) ON DELETE CASCADE,
    emoji           TEXT NOT NULL,
    actor_id        TEXT NOT NULL,
    actor_kind      TEXT NOT NULL CHECK(actor_kind IN ('user', 'agent')),
    display_name    TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    UNIQUE(message_id, emoji, actor_id),
    CHECK(length(emoji) > 0 AND length(emoji) <= 32),
    CHECK(length(reaction_id) > 0),
    CHECK(length(actor_id) > 0),
    CHECK(length(display_name) > 0)
);

CREATE INDEX chat_message_reactions_by_message
    ON chat_message_reactions(message_id);

CREATE TABLE events (
    session_id            TEXT NOT NULL,
    seq                   INTEGER NOT NULL,
    body_kind             TEXT NOT NULL,
    body_inline           BLOB,
    artifact_id           TEXT,
    artifact_size_bytes   INTEGER,
    artifact_sha256       TEXT,
    artifact_media_type   TEXT,
    projection_json       BLOB NOT NULL,
    ts_ms                 INTEGER NOT NULL,
    source                TEXT NOT NULL DEFAULT 'live',
    PRIMARY KEY (session_id, seq),
    FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX events_by_ts
    ON events(session_id, ts_ms);

CREATE TABLE artifacts (
    session_id    TEXT NOT NULL,
    artifact_id   TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL,
    sha256        TEXT NOT NULL,
    media_type    TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (session_id, artifact_id),
    FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE ingest_sync_state (
    session_id          TEXT PRIMARY KEY,
    backend_acked_seq   INTEGER NOT NULL DEFAULT 0,
    dirty_from_seq      INTEGER,
    dirty_to_seq        INTEGER,
    dirty_bytes         INTEGER NOT NULL DEFAULT 0,
    dirty_events        INTEGER NOT NULL DEFAULT 0,
    updated_at          INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
) WITHOUT ROWID;

-- Host-local bot identity cache (cloud agents mirror + offline runtime seeds).
-- Replaces agent_profiles: keyed by bot_id; system_prompt was instructions.
CREATE TABLE bot_identities (
    bot_id             TEXT PRIMARY KEY NOT NULL,
    display_name       TEXT NOT NULL,
    description        TEXT NOT NULL DEFAULT '',
    runtime_agent      TEXT NOT NULL,
    model              TEXT NOT NULL,
    reasoning_effort   TEXT NOT NULL DEFAULT '',
    system_prompt      TEXT NOT NULL DEFAULT '',
    env_json           TEXT NOT NULL DEFAULT '[]',
    -- user_configured | host_runtime_seed
    source             TEXT NOT NULL DEFAULT 'user_configured',
    owner_account_id   TEXT,
    synced_at_ms       INTEGER,
    created_at_ms      INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL,
    CHECK(length(bot_id) > 0),
    CHECK(length(display_name) > 0),
    CHECK(length(runtime_agent) > 0),
    CHECK(source IN ('user_configured', 'host_runtime_seed'))
);

CREATE INDEX idx_bot_identities_updated
    ON bot_identities (updated_at_ms DESC);

CREATE INDEX idx_bot_identities_runtime
    ON bot_identities (runtime_agent, updated_at_ms DESC);

-- Durable BotInboxDelivery ledger (restart-safe exactly-once inject).
CREATE TABLE bot_delivery_ledger (
    delivery_id      TEXT PRIMARY KEY NOT NULL,
    conversation_id  TEXT NOT NULL,
    bot_id           TEXT NOT NULL,
    session_id       TEXT,
    status           TEXT NOT NULL CHECK (status IN (
        'received', 'injected', 'completed', 'rejected'
    )),
    accepted         INTEGER,
    last_error       TEXT,
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    CHECK(length(delivery_id) > 0),
    CHECK(length(conversation_id) > 0),
    CHECK(length(bot_id) > 0)
);

CREATE INDEX idx_bot_delivery_ledger_status
    ON bot_delivery_ledger(status, updated_at_ms);
