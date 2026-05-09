ALTER TABLE chat_messages
ADD COLUMN reply_to_message_id TEXT REFERENCES chat_messages(message_id) ON DELETE SET NULL;

ALTER TABLE chat_messages
ADD COLUMN recalled_at_ms INTEGER;

CREATE INDEX idx_chat_messages_reply_to
ON chat_messages(reply_to_message_id)
WHERE reply_to_message_id IS NOT NULL;