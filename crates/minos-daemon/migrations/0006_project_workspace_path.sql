-- Persist the real workspace_path (user's cwd) for project matching on restart.
-- Previously only workspace_slug was stored, and list_projects reconstructed a
-- synthetic .minos/workspaces/<slug> path that didn't match the user's real cwd.
ALTER TABLE projects ADD COLUMN workspace_path TEXT;
