CREATE TABLE pairing_tokens (
    token_hash        TEXT PRIMARY KEY,            -- SHA-256 hex digest of the plaintext token bearer (32B random → 64 hex chars). Deterministic for PK lookup; safe because tokens are one-shot and TTL ≤ 5 min.
    issuer_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    created_at        INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL,
    consumed_at       INTEGER                     -- NULL until pair() succeeds
) STRICT;

CREATE INDEX idx_pairing_tokens_expires ON pairing_tokens(expires_at) WHERE consumed_at IS NULL;
