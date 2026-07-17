-- Grok support: extend historical CHECK lists where present is handled for
-- fresh installs via 0001_initial.sql. Existing DBs with CHECK constraints
-- on agent/runtime_agent need a table rebuild to accept 'grok'; SQLite
-- cannot ALTER CHECK, so this migration is intentionally a no-op marker.
SELECT 1;
