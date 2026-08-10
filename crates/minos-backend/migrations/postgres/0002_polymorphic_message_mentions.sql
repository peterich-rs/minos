-- Polymorphic chat_message_mentions: target_kind ∈ {account, agent}.
--
-- Fresh installs already have the polymorphic shape from 0001_initial.sql.
-- Existing volumes that applied pre-bot 0001 still have mentioned_account_id.
-- Idempotent: rebuild only when the legacy column is present; no-op when
-- target_kind already exists.

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM information_schema.columns
         WHERE table_schema = 'public'
           AND table_name = 'chat_message_mentions'
           AND column_name = 'mentioned_account_id'
    ) AND NOT EXISTS (
        SELECT 1
          FROM information_schema.columns
         WHERE table_schema = 'public'
           AND table_name = 'chat_message_mentions'
           AND column_name = 'target_kind'
    ) THEN
        CREATE TABLE chat_message_mentions__polymorphic (
            message_id   TEXT NOT NULL REFERENCES chat_messages(message_id) ON DELETE CASCADE,
            target_kind  TEXT NOT NULL CHECK (target_kind IN ('account', 'agent')),
            target_id    TEXT NOT NULL,
            PRIMARY KEY (message_id, target_kind, target_id)
        );

        INSERT INTO chat_message_mentions__polymorphic (message_id, target_kind, target_id)
        SELECT message_id, 'account', mentioned_account_id
          FROM chat_message_mentions;

        DROP TABLE chat_message_mentions;
        ALTER TABLE chat_message_mentions__polymorphic RENAME TO chat_message_mentions;

        CREATE INDEX IF NOT EXISTS idx_chat_message_mentions_target
            ON chat_message_mentions(target_kind, target_id, message_id);

        DROP INDEX IF EXISTS idx_chat_message_mentions_account;
    END IF;
END $$;
