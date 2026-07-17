-- Conversation message metadata for delegation reply / mentions.
ALTER TABLE chat_messages ADD COLUMN reply_to_message_id TEXT;
ALTER TABLE chat_messages ADD COLUMN delegation_id TEXT;
ALTER TABLE chat_messages ADD COLUMN mentions_json TEXT NOT NULL DEFAULT '[]';

CREATE INDEX IF NOT EXISTS chat_messages_by_delegation
    ON chat_messages(conversation_id, delegation_id)
    WHERE delegation_id IS NOT NULL;
