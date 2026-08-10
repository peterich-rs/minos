-- Appearance-order index for polymorphic mentions.
--
-- Fresh 0001 already includes `ordinal`. Legacy volumes that rebuilt via
-- ensure_polymorphic_message_mentions_sqlite also get the column there.
-- This migration only ensures the hydrate ORDER BY index exists (idempotent).

CREATE INDEX IF NOT EXISTS idx_chat_message_mentions_message_ordinal
    ON chat_message_mentions(message_id, target_kind, ordinal);
