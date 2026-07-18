-- Track whether a suspended thread should auto-inject a continue turn after
-- host process death (running/starting/resuming at stop time). Idle sessions
-- rehydrate without a synthetic prompt.
ALTER TABLE threads ADD COLUMN needs_continue INTEGER NOT NULL DEFAULT 0;
