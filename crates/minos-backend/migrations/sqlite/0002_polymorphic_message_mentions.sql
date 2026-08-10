-- Polymorphic chat_message_mentions marker migration (version 2).
--
-- Fresh installs already have (message_id, target_kind, target_id) from 0001.
-- Existing volumes that applied pre-bot 0001 still have mentioned_account_id.
--
-- SQLite cannot conditionally SELECT/INDEX a missing column in pure SQL
-- (prepare-time name resolution), so the legacy → polymorphic rebuild runs in
-- Rust after the migrator: `store::ensure_polymorphic_message_mentions`.
--
-- Keep this file as a version marker aligned with postgres/0002. No schema
-- change here — the following statement is intentionally a no-op.

SELECT 1;
