-- Optional system-prompt / instructions supplement for personalized agents.
ALTER TABLE agent_profiles ADD COLUMN instructions TEXT NOT NULL DEFAULT '';
