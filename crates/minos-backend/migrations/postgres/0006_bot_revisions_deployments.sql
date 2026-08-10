-- Bot revision snapshots (immutable body at schedule time) + host deployments.
-- See bot-mailbox-ws-im-bus-design Phase 5 / global-bot-identity-design.

CREATE TABLE bot_revisions (
    revision_id              TEXT PRIMARY KEY,
    agent_id                 TEXT NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    runtime_agent            TEXT NOT NULL,
    model                    TEXT NOT NULL DEFAULT '',
    default_reasoning_effort TEXT NOT NULL DEFAULT '',
    system_prompt            TEXT NOT NULL DEFAULT '',
    display_name             TEXT NOT NULL DEFAULT '',
    workspace_path           TEXT,
    created_at_ms            BIGINT NOT NULL
);

CREATE INDEX idx_bot_revisions_agent_created
    ON bot_revisions(agent_id, created_at_ms DESC);

CREATE TABLE bot_deployments (
    agent_id              TEXT NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    host_installation_id  TEXT NOT NULL REFERENCES device_installations(installation_id) ON DELETE CASCADE,
    status                TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled')),
    updated_at_ms         BIGINT NOT NULL,
    PRIMARY KEY (agent_id, host_installation_id)
);

CREATE INDEX idx_bot_deployments_host
    ON bot_deployments(host_installation_id, status);
