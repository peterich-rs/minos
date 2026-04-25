CREATE TABLE threads (
    thread_id         TEXT PRIMARY KEY,
    agent             TEXT NOT NULL CHECK (agent IN ('codex','claude','gemini')),
    owner_device_id   TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    title             TEXT,
    first_ts_ms       INTEGER NOT NULL,
    last_ts_ms        INTEGER NOT NULL,
    ended_at_ms       INTEGER,
    end_reason        TEXT,
    message_count     INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX idx_threads_last_ts  ON threads(last_ts_ms DESC);
CREATE INDEX idx_threads_owner    ON threads(owner_device_id, last_ts_ms DESC);
