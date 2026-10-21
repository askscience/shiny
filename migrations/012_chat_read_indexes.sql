-- Indexes for the bounded history reads and the conversation list.
CREATE INDEX IF NOT EXISTS idx_chat_messages_traveler_time
    ON chat_messages (traveler_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_chat_conversations_traveler_updated
    ON chat_conversations (traveler_id, updated_at DESC);
