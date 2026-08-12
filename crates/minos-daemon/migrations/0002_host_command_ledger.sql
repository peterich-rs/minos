-- Durable host-command idempotency + host topic resume watermark.
-- Mirrors bot_delivery_ledger: transport is at-least-once; SQLite is Host authority.

CREATE TABLE host_command_ledger (
    command_id       TEXT PRIMARY KEY NOT NULL,
    status           TEXT NOT NULL CHECK (status IN (
        'inflight', 'completed'
    )),
    succeeded        INTEGER,
    result_json      TEXT,
    error_json       TEXT,
    finished_at_ms   INTEGER,
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    CHECK(length(command_id) > 0)
);

CREATE INDEX idx_host_command_ledger_status
    ON host_command_ledger(status, updated_at_ms);

CREATE TABLE host_topic_cursors (
    topic            TEXT PRIMARY KEY NOT NULL,
    topic_seq        INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    CHECK(length(topic) > 0),
    CHECK(topic_seq >= 0)
);
