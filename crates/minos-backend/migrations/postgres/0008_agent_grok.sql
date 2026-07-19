-- Add Grok as a first-class runtime agent.

INSERT INTO agents (agent_id, runtime_kind, display_name, created_at_ms)
VALUES (
    'agent_grok',
    'grok',
    'Grok',
    (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT
)
ON CONFLICT (agent_id) DO NOTHING;
