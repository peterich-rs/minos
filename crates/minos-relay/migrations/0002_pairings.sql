-- Enforce undirected uniqueness by storing (a, b) with a < b.
CREATE TABLE pairings (
    device_a       TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    device_b       TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (device_a, device_b),
    CHECK (device_a < device_b)
) STRICT;

CREATE INDEX idx_pairings_a ON pairings(device_a);
CREATE INDEX idx_pairings_b ON pairings(device_b);
