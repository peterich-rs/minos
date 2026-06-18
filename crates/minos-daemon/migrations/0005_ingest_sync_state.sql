CREATE TABLE ingest_sync_state (
    thread_id          TEXT PRIMARY KEY,
    backend_acked_seq  INTEGER NOT NULL DEFAULT 0,
    dirty_from_seq     INTEGER,
    dirty_to_seq       INTEGER,
    dirty_bytes        INTEGER NOT NULL DEFAULT 0,
    dirty_events       INTEGER NOT NULL DEFAULT 0,
    updated_at         INTEGER NOT NULL,
    FOREIGN KEY (thread_id) REFERENCES threads(thread_id) ON DELETE CASCADE
) WITHOUT ROWID;
