CREATE TRIGGER pairings_single_pair_per_device_insert
BEFORE INSERT ON pairings
FOR EACH ROW
WHEN EXISTS (
    SELECT 1
    FROM pairings
    WHERE (device_a = NEW.device_a OR device_b = NEW.device_a OR device_a = NEW.device_b OR device_b = NEW.device_b)
      AND NOT (device_a = NEW.device_a AND device_b = NEW.device_b)
)
BEGIN
    SELECT RAISE(ABORT, 'pairings_device_already_paired');
END;

CREATE TRIGGER pairings_single_pair_per_device_update
BEFORE UPDATE OF device_a, device_b ON pairings
FOR EACH ROW
WHEN EXISTS (
    SELECT 1
    FROM pairings
    WHERE rowid != OLD.rowid
      AND (device_a = NEW.device_a OR device_b = NEW.device_a OR device_a = NEW.device_b OR device_b = NEW.device_b)
)
BEGIN
    SELECT RAISE(ABORT, 'pairings_device_already_paired');
END;