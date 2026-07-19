-- Conversation product metadata: priority, progress workflow, git snapshot at create.
ALTER TABLE conversations ADD COLUMN priority TEXT;
ALTER TABLE conversations ADD COLUMN progress TEXT NOT NULL DEFAULT 'todo';
ALTER TABLE conversations ADD COLUMN branch TEXT;
ALTER TABLE conversations ADD COLUMN worktree_path TEXT;
