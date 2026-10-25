-- Per-conversation consent for the assistant's terminal commands ("Allow for
-- this chat"). Off by default: every terminal_exec asks first. A command the
-- built-in safety guard blocks stays blocked even when this is on.
ALTER TABLE chat_conversations
    ADD COLUMN terminal_auto_approve INTEGER NOT NULL DEFAULT 0;
