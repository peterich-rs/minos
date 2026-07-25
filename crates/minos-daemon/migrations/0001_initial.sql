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
    updated_at_ms         INTEGER NOT NULL,
    -- Product metadata (priority / workflow / git snapshot at create)
    priority              TEXT,
    progress              TEXT NOT NULL DEFAULT 'todo',
    branch                TEXT,
    worktree_path         TEXT,
    CHECK(length(title) > 0),
    CHECK(message_count >= 0),
    CHECK(agent_session_count >= 0)
);

CREATE INDEX conversations_by_project_updated
    ON conversations(project_id, updated_at_ms DESC, conversation_id);

-- Host-local conversation roster (runtime agent names). Mentions / starts are
-- gated on membership; sessions alone do not imply membership.
CREATE TABLE conversation_agent_members (
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    agent            TEXT NOT NULL,
    joined_at_ms     INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, agent),
    CHECK(length(agent) > 0)
);

CREATE INDEX conversation_agent_members_by_agent
    ON conversation_agent_members(agent, conversation_id);

CREATE TABLE sessions (
    session_id            TEXT PRIMARY KEY,
    conversation_id       TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    workspace_root        TEXT NOT NULL REFERENCES workspaces(root),
    parent_session_id     TEXT REFERENCES sessions(session_id) ON DELETE CASCADE,
    agent                 TEXT NOT NULL,
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
    sender_role          TEXT NOT NULL CHECK(sender_role IN ('user', 'agent')),
    agent                TEXT,
    body                 TEXT NOT NULL,
    -- Delegation / mention metadata
    reply_to_message_id  TEXT,
    delegation_id        TEXT,
    mentions_json        TEXT NOT NULL DEFAULT '[]',
    CHECK(length(message_id) > 0),
    CHECK(
        (sender_role = 'user' AND session_id IS NULL AND agent IS NULL)
        OR
        (sender_role = 'agent' AND session_id IS NOT NULL AND agent IS NOT NULL)
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

-- Host-local personalized agent profiles (fixed runtime + model + effort).
CREATE TABLE agent_profiles (
    id                TEXT PRIMARY KEY NOT NULL,
    name              TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    runtime_agent     TEXT NOT NULL,
    model             TEXT NOT NULL,
    reasoning_effort  TEXT NOT NULL DEFAULT '',
    env_json          TEXT NOT NULL DEFAULT '[]',
    instructions      TEXT NOT NULL DEFAULT '',
    created_at_ms     INTEGER NOT NULL,
    updated_at_ms     INTEGER NOT NULL
);

CREATE INDEX idx_agent_profiles_updated
    ON agent_profiles (updated_at_ms DESC);
