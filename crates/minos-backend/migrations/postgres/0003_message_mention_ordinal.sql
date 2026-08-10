-- Preserve body appearance order for polymorphic mentions on hydrate/history.
-- Inserts write ordinal 0..n-1 within each (message_id, target_kind) stream;
-- list_message_mentions_full orders by ordinal instead of target_id lex.

ALTER TABLE chat_message_mentions
    ADD COLUMN IF NOT EXISTS ordinal INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_chat_message_mentions_message_ordinal
    ON chat_message_mentions(message_id, target_kind, ordinal);
