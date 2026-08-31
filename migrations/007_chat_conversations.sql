-- Chat conversations: group chat_messages into resumable threads so the AI
-- keeps the context of each conversation (instead of one flat history).
CREATE TABLE IF NOT EXISTS chat_conversations (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT 'New chat',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_traveler ON chat_conversations(traveler_id);
