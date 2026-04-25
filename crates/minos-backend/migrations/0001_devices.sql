CREATE TABLE devices (
    device_id      TEXT PRIMARY KEY,          -- UUIDv4 string
    display_name   TEXT NOT NULL,
    role           TEXT NOT NULL CHECK (role IN ('agent-host','ios-client','browser-admin')),
    secret_hash    TEXT,                      -- argon2id; NULL while unpaired
    created_at     INTEGER NOT NULL,          -- unix epoch ms
    last_seen_at   INTEGER NOT NULL
) STRICT;
