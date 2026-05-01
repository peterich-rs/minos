-- 0012_account_mac_pairings.sql
-- Pair model is now (mac_device_id, mobile_account_id). The mobile
-- device_id that performed the scan is recorded as audit metadata.
-- See docs/adr/0020-server-centric-auth-and-account-pairs.md.

CREATE TABLE account_mac_pairings (
    pair_id              TEXT NOT NULL PRIMARY KEY,    -- UUID
    mac_device_id        TEXT NOT NULL,
    mobile_account_id    TEXT NOT NULL,
    paired_via_device_id TEXT NOT NULL,                -- mobile device that scanned; audit only
    paired_at_ms         INTEGER NOT NULL,
    UNIQUE (mac_device_id, mobile_account_id),
    FOREIGN KEY (mac_device_id)        REFERENCES devices(device_id)   ON DELETE CASCADE,
    FOREIGN KEY (mobile_account_id)    REFERENCES accounts(account_id) ON DELETE CASCADE,
    FOREIGN KEY (paired_via_device_id) REFERENCES devices(device_id)   ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_amp_mobile_account ON account_mac_pairings(mobile_account_id);
CREATE INDEX idx_amp_mac_device     ON account_mac_pairings(mac_device_id);
