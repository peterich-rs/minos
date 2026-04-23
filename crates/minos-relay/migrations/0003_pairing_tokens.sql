CREATE TABLE pairing_tokens (
    token_hash        TEXT PRIMARY KEY,            -- argon2 hash of bearer
    issuer_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    created_at        INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL,
    consumed_at       INTEGER                     -- NULL until pair() succeeds
) STRICT;

CREATE INDEX idx_pairing_tokens_expires ON pairing_tokens(expires_at) WHERE consumed_at IS NULL;
