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
    CHECK(length(title) > 0),
    CHECK(message_count >= 0),
    CHECK(agent_session_count >= 0)
);

CREATE INDEX conversations_by_project_updated
    ON conversations(project_id, updated_at_ms DESC, conversation_id);

CREATE TABLE threads (
    thread_id            TEXT PRIMARY KEY,
    conversation_id      TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    workspace_root       TEXT NOT NULL REFERENCES workspaces(root),
    agent                TEXT NOT NULL,
    provider_session_id  TEXT,
    status              TEXT NOT NULL CHECK(status IN ('starting', 'idle', 'running', 'resuming', 'suspended', 'closed')),
    last_pause_reason    TEXT,
    last_close_reason    TEXT,
    last_seq             INTEGER NOT NULL DEFAULT 0,
    started_at           INTEGER NOT NULL,
    last_activity_at     INTEGER NOT NULL,
    ended_at             INTEGER,
    CHECK(length(agent) > 0),
    CHECK(last_seq >= 0)
);

CREATE INDEX threads_by_conversation_last
    ON threads(conversation_id, last_activity_at DESC, thread_id);

CREATE INDEX threads_by_conversation_agent_last
    ON threads(conversation_id, agent, last_activity_at DESC, thread_id);

CREATE INDEX threads_by_workspace
    ON threads(workspace_root, last_activity_at DESC);

CREATE INDEX threads_by_status
    ON threads(status, last_activity_at DESC);

CREATE TABLE chat_messages (
    message_seq      INTEGER PRIMARY KEY,
    message_id       TEXT NOT NULL UNIQUE,
    conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    thread_id        TEXT REFERENCES threads(thread_id) ON DELETE SET NULL,
    created_at_ms    INTEGER NOT NULL,
    sender_role      TEXT NOT NULL CHECK(sender_role IN ('user', 'agent')),
    agent            TEXT,
    body             TEXT NOT NULL,
    CHECK(length(message_id) > 0),
    CHECK(
        (sender_role = 'user' AND thread_id IS NULL AND agent IS NULL)
        OR
        (sender_role = 'agent' AND thread_id IS NOT NULL AND agent IS NOT NULL)
    )
);

CREATE INDEX chat_messages_by_conversation_seq
    ON chat_messages(conversation_id, message_seq DESC);

CREATE INDEX chat_messages_by_thread_seq
    ON chat_messages(thread_id, message_seq DESC)
    WHERE thread_id IS NOT NULL;

CREATE TABLE events (
    thread_id             TEXT NOT NULL,
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
    PRIMARY KEY (thread_id, seq),
    FOREIGN KEY (thread_id) REFERENCES threads(thread_id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX events_by_ts
    ON events(thread_id, ts_ms);

CREATE TABLE artifacts (
    thread_id    TEXT NOT NULL,
    artifact_id  TEXT NOT NULL,
    size_bytes   INTEGER NOT NULL,
    sha256       TEXT NOT NULL,
    media_type   TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (thread_id, artifact_id),
    FOREIGN KEY (thread_id) REFERENCES threads(thread_id) ON DELETE CASCADE
) WITHOUT ROWID;
