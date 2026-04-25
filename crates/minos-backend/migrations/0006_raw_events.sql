CREATE TABLE raw_events (
    thread_id    TEXT NOT NULL REFERENCES threads(thread_id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    agent        TEXT NOT NULL CHECK (agent IN ('codex','claude','gemini')),
    payload_json TEXT NOT NULL,
    ts_ms        INTEGER NOT NULL,
    PRIMARY KEY (thread_id, seq)
) STRICT;

CREATE INDEX idx_raw_events_thread_seq ON raw_events(thread_id, seq);
