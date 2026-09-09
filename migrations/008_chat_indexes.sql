-- Index for conversation history reads (chat_memory::recent_history and the
-- conversation message list endpoint both filter+order on these columns).
CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation
    ON chat_messages (conversation_id, timestamp);
