-- Global bot digital body fields (identity SSOT on Hub).
-- See docs/superpowers/specs/global-bot-identity-design.md

ALTER TABLE agents ADD COLUMN IF NOT EXISTS display_name TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN IF NOT EXISTS avatar_url TEXT;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE agents ADD COLUMN IF NOT EXISTS default_reasoning_effort TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN IF NOT EXISTS system_prompt TEXT NOT NULL DEFAULT '';

UPDATE agents SET display_name = name WHERE display_name = '';

-- Case-insensitive: mention resolution matches eq_ignore_ascii_case on name.
CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_owner_name_active
    ON agents (owner_account_id, lower(name))
    WHERE status = 'active';
