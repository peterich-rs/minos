CREATE TABLE projects (
    project_id      TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    workspace_slug  TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL,
    UNIQUE(account_id, workspace_slug)
) STRICT;

CREATE INDEX idx_projects_account_updated
    ON projects(account_id, updated_at_ms DESC);

ALTER TABLE threads ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;

CREATE INDEX idx_threads_project_last
    ON threads(project_id, last_ts_ms DESC)
    WHERE project_id IS NOT NULL;

CREATE TABLE project_threads (
    project_id    TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    thread_id     TEXT NOT NULL,
    account_id    TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    linked_at_ms  INTEGER NOT NULL,
    PRIMARY KEY(project_id, thread_id)
) STRICT;

CREATE INDEX idx_project_threads_account_project
    ON project_threads(account_id, project_id, linked_at_ms DESC);
