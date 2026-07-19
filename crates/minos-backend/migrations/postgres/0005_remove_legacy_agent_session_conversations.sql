DELETE FROM conversations c
WHERE c.title = 'Legacy agent session'
  AND EXISTS (
      SELECT 1
        FROM agent_sessions s
       WHERE s.conversation_id = c.conversation_id
  )
  AND NOT EXISTS (
      SELECT 1
        FROM conversation_messages m
       WHERE m.conversation_id = c.conversation_id
  );
