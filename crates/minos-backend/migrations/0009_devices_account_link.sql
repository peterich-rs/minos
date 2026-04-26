ALTER TABLE devices ADD COLUMN account_id TEXT REFERENCES accounts(account_id);
CREATE INDEX idx_devices_account ON devices(account_id) WHERE account_id IS NOT NULL;
