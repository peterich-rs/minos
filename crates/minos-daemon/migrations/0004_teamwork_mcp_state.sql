CREATE TABLE IF NOT EXISTS teamwork_delegations (
    delegation_seq INTEGER PRIMARY KEY,
    delegation_id  TEXT NOT NULL UNIQUE,
    room_id        TEXT NOT NULL REFERENCES chat_rooms(room_id),
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL,
    status         TEXT NOT NULL CHECK(status IN ('running', 'completed', 'cancelled', 'failed')),
    source_agent   TEXT,
    target_agent   TEXT NOT NULL,
    prompt         TEXT NOT NULL,
    thread_id      TEXT,
    error          TEXT
);

CREATE INDEX IF NOT EXISTS teamwork_delegations_by_room_status
    ON teamwork_delegations(room_id, status, delegation_seq DESC);

CREATE TABLE IF NOT EXISTS teamwork_user_feedback (
    feedback_seq         INTEGER PRIMARY KEY,
    feedback_id          TEXT NOT NULL UNIQUE,
    room_id              TEXT NOT NULL REFERENCES chat_rooms(room_id),
    created_at_ms        INTEGER NOT NULL,
    updated_at_ms        INTEGER NOT NULL,
    status               TEXT NOT NULL CHECK(status IN ('pending', 'answered', 'cancelled')),
    source_agent         TEXT,
    question             TEXT NOT NULL,
    question_message_seq INTEGER NOT NULL,
    answer_message_seq   INTEGER,
    answer_text          TEXT
);

CREATE INDEX IF NOT EXISTS teamwork_user_feedback_by_room_status
    ON teamwork_user_feedback(room_id, status, feedback_seq DESC);

CREATE TABLE IF NOT EXISTS teamwork_message_reactions (
    room_id       TEXT NOT NULL REFERENCES chat_rooms(room_id),
    message_id    TEXT NOT NULL,
    message_seq   INTEGER NOT NULL,
    emoji         TEXT NOT NULL,
    reactor       TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(room_id, message_id, emoji, reactor)
);

CREATE INDEX IF NOT EXISTS teamwork_message_reactions_by_message
    ON teamwork_message_reactions(room_id, message_id);
