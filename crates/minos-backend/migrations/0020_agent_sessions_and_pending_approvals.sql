ALTER TABLE chat_messages
ADD COLUMN sender_agent_id TEXT REFERENCES agents(agent_id) ON DELETE CASCADE;

ALTER TABLE chat_messages
ADD COLUMN agent_session_id TEXT;

CREATE INDEX idx_chat_messages_agent_session
ON chat_messages(agent_session_id)
WHERE agent_session_id IS NOT NULL;

CREATE TABLE pending_approvals (
    request_id       TEXT PRIMARY KEY,
    thread_id        TEXT NOT NULL,
    turn_id          TEXT NOT NULL,
    host_device_id   TEXT NOT NULL,
    method           TEXT NOT NULL,
    params_json      TEXT NOT NULL,
    created_at_ms    INTEGER NOT NULL,
    timeout_at_ms    INTEGER NOT NULL,
    resolved_at_ms   INTEGER,
    resolution       TEXT CHECK (
        resolution IS NULL OR resolution IN ('user_decision', 'timeout', 'disconnected')
    )
) STRICT;

CREATE INDEX idx_pending_approvals_timeout
ON pending_approvals(timeout_at_ms)
WHERE resolved_at_ms IS NULL;

CREATE INDEX idx_pending_approvals_thread
ON pending_approvals(thread_id)
WHERE resolved_at_ms IS NULL;