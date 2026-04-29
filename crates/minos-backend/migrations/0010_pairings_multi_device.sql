DROP TRIGGER IF EXISTS pairings_single_pair_per_device_insert;
DROP TRIGGER IF EXISTS pairings_single_pair_per_device_update;
DROP TRIGGER IF EXISTS pairings_single_non_host_pair_insert;
DROP TRIGGER IF EXISTS pairings_single_non_host_pair_update;

-- Pairing is now multi-device. A mobile client may add multiple runtime
-- devices, and a runtime may be visible to multiple mobile clients. The
-- existing UNIQUE(device_a, device_b) constraint still dedupes the exact same
-- pair; no trigger should reject a device appearing in another pair row.
