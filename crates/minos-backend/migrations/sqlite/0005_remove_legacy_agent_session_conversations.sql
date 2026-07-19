DELETE FROM conversations
WHERE title = 'Legacy agent session'
  AND EXISTS (
      SELECT 1
        FROM agent_sessions s
       WHERE s.conversation_id = conversations.conversation_id
  )
  AND NOT EXISTS (
      SELECT 1
        FROM chat_messages m
       WHERE m.conversation_id = conversations.conversation_id
  );
