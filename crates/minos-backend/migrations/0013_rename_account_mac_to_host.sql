-- 0013_rename_account_mac_to_host.sql
-- Phase B of plan 12-agent-session-manager-and-minos-home renames the
-- protocol-facing `Mac` vocabulary to `Host`. The pair model in storage is
-- otherwise unchanged: `(host_device_id, mobile_account_id)`.
--
-- SQLite 3.25+ supports both `ALTER TABLE ... RENAME TO` and
-- `ALTER TABLE ... RENAME COLUMN` losslessly. Indexes that referenced the
-- old column name are updated automatically; only their *names* need to be
-- refreshed for grep-ability.

ALTER TABLE account_mac_pairings RENAME TO account_host_pairings;
ALTER TABLE account_host_pairings RENAME COLUMN mac_device_id TO host_device_id;

DROP INDEX IF EXISTS idx_amp_mobile_account;
DROP INDEX IF EXISTS idx_amp_mac_device;
CREATE INDEX idx_account_host_pairings_account
    ON account_host_pairings(mobile_account_id);
CREATE INDEX idx_account_host_pairings_host
    ON account_host_pairings(host_device_id);
