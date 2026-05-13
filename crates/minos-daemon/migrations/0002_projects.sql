-- Projects: a project groups threads under a named workspace.
-- Each project maps to a folder under .minos/workspaces/<slug>.
CREATE TABLE projects (
    project_id       TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    workspace_slug   TEXT NOT NULL UNIQUE,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX projects_by_updated ON projects(updated_at DESC);

-- Link threads to projects. Nullable for backward compat with existing threads.
ALTER TABLE threads ADD COLUMN project_id TEXT REFERENCES projects(project_id);

CREATE INDEX threads_by_project ON threads(project_id, last_activity_at DESC);
