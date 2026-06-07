CREATE TABLE IF NOT EXISTS chat_rooms (
    room_id        TEXT PRIMARY KEY,
    title          TEXT NOT NULL,
    workspace_root TEXT NOT NULL,
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chat_messages (
    message_seq    INTEGER PRIMARY KEY,
    message_id     TEXT NOT NULL UNIQUE,
    room_id        TEXT NOT NULL REFERENCES chat_rooms(room_id),
    created_at_ms  INTEGER NOT NULL,
    sender_role    TEXT NOT NULL CHECK(sender_role IN ('user', 'agent')),
    event_type     TEXT NOT NULL CHECK(event_type IN ('user_message', 'agent_result')),
    body           TEXT NOT NULL,
    agent          TEXT,
    thread_id      TEXT,
    thread_short_id TEXT,
    workspace_root TEXT
);

CREATE INDEX IF NOT EXISTS chat_messages_by_room_seq
    ON chat_messages(room_id, message_seq DESC);

CREATE INDEX IF NOT EXISTS chat_messages_by_thread
    ON chat_messages(thread_id, message_seq DESC);

CREATE TABLE IF NOT EXISTS chat_agent_sessions (
    room_id           TEXT NOT NULL,
    thread_id         TEXT NOT NULL,
    agent             TEXT NOT NULL,
    thread_short_id   TEXT NOT NULL,
    workspace_root    TEXT NOT NULL,
    first_seen_at_ms  INTEGER NOT NULL,
    last_seen_at_ms   INTEGER NOT NULL,
    first_message_seq INTEGER NOT NULL,
    last_message_seq  INTEGER NOT NULL,
    PRIMARY KEY(room_id, thread_id),
    FOREIGN KEY(room_id) REFERENCES chat_rooms(room_id)
);

CREATE INDEX IF NOT EXISTS chat_agent_sessions_by_room_last
    ON chat_agent_sessions(room_id, last_message_seq DESC);

CREATE TABLE IF NOT EXISTS chat_mcp_commands (
    command_seq     INTEGER PRIMARY KEY,
    room_id         TEXT NOT NULL REFERENCES chat_rooms(room_id),
    created_at_ms   INTEGER NOT NULL,
    claimed_at_ms   INTEGER,
    completed_at_ms INTEGER,
    status          TEXT NOT NULL CHECK(status IN ('pending', 'claimed', 'completed', 'failed')),
    command_type    TEXT NOT NULL CHECK(command_type IN ('mention_agent', 'mention_user')),
    source_agent    TEXT,
    target_agent    TEXT,
    body            TEXT NOT NULL,
    error           TEXT
);

CREATE INDEX IF NOT EXISTS chat_mcp_commands_by_room_status_seq
    ON chat_mcp_commands(room_id, status, command_seq ASC);
