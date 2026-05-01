-- 0011_drop_legacy_pairings.sql
-- Pre-deployment: drop old device-keyed pairings outright.
-- The replacement is account_mac_pairings (migration 0012).
-- See docs/adr/0020-server-centric-auth-and-account-pairs.md.

DROP INDEX IF EXISTS idx_pairings_a;
DROP INDEX IF EXISTS idx_pairings_b;
DROP INDEX IF EXISTS idx_pairings_device_a;
DROP INDEX IF EXISTS idx_pairings_device_b;
DROP TABLE IF EXISTS pairings;
