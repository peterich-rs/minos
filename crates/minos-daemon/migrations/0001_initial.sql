CREATE TABLE schema_version (
    version    INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);

CREATE TABLE workspaces (
    root          TEXT PRIMARY KEY,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL
);

CREATE TABLE threads (
    thread_id          TEXT PRIMARY KEY,
    workspace_root     TEXT NOT NULL REFERENCES workspaces(root),
    agent              TEXT NOT NULL,
    codex_session_id   TEXT,
    status             TEXT NOT NULL,
    last_pause_reason  TEXT,
    last_close_reason  TEXT,
    last_seq           INTEGER NOT NULL DEFAULT 0,
    started_at         INTEGER NOT NULL,
    last_activity_at   INTEGER NOT NULL,
    ended_at           INTEGER
);

CREATE INDEX threads_by_workspace ON threads(workspace_root, last_activity_at DESC);
CREATE INDEX threads_by_status    ON threads(status, last_activity_at DESC);

CREATE TABLE events (
    thread_id TEXT NOT NULL,
    seq       INTEGER NOT NULL,
    payload   BLOB NOT NULL,
    ts_ms     INTEGER NOT NULL,
    source    TEXT NOT NULL DEFAULT 'live',
    PRIMARY KEY (thread_id, seq),
    FOREIGN KEY (thread_id) REFERENCES threads(thread_id)
) WITHOUT ROWID;

CREATE INDEX events_by_ts ON events(thread_id, ts_ms);
