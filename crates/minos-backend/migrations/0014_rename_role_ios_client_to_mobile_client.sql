-- 0014_rename_role_ios_client_to_mobile_client.sql
-- Phase B of plan 12-agent-session-manager-and-minos-home renames the
-- DeviceRole wire string `ios-client` → `mobile-client`. The `devices.role`
-- CHECK constraint enumerates the legacy spelling, so we must rebuild the
-- table (SQLite cannot ALTER an existing CHECK).
--
-- Rebuild procedure follows the SQLite "Making Other Kinds Of Table Schema
-- Changes" recipe (https://sqlite.org/lang_altertable.html#otheralter):
--
--   1. UPDATE existing rows to the new wire string while the old CHECK
--      still allows them.
--   2. Defer FK enforcement for the duration of the rebuild so child
--      tables that reference `devices(device_id)` survive the rename.
--   3. CREATE devices_new with the updated CHECK list.
--   4. INSERT * SELECT * to copy rows.
--   5. DROP devices, RENAME devices_new -> devices.
--   6. Recreate the index added in 0009 (the column DDL is preserved
--      verbatim from 0001 + 0009; only the CHECK list changes).
--
-- `PRAGMA defer_foreign_keys = ON` only applies inside the current
-- transaction; sqlx wraps each migration in one. This is the documented
-- escape hatch for FK-referenced rebuilds.

UPDATE devices
   SET role = 'mobile-client'
 WHERE role = 'ios-client';

PRAGMA defer_foreign_keys = ON;

CREATE TABLE devices_new (
    device_id      TEXT PRIMARY KEY,          -- UUIDv4 string
    display_name   TEXT NOT NULL,
    role           TEXT NOT NULL CHECK (role IN ('agent-host','mobile-client','browser-admin')),
    secret_hash    TEXT,                      -- argon2id; NULL while unpaired
    created_at     INTEGER NOT NULL,          -- unix epoch ms
    last_seen_at   INTEGER NOT NULL,
    account_id     TEXT REFERENCES accounts(account_id)
) STRICT;

INSERT INTO devices_new
    (device_id, display_name, role, secret_hash, created_at, last_seen_at, account_id)
SELECT
    device_id, display_name, role, secret_hash, created_at, last_seen_at, account_id
FROM devices;

DROP TABLE devices;
ALTER TABLE devices_new RENAME TO devices;

CREATE INDEX idx_devices_account ON devices(account_id) WHERE account_id IS NOT NULL;
