-- What the assistant is working on in one conversation, kept between turns.
--
-- dev_language: the language the user chose for building plugins in this chat
--   ('python' or 'rust'). Chosen once per conversation; the plugin builder's
--   commands for the other language are refused there.
-- dev_plugin: the plugin workspace this chat is working on.
-- plan / plan_state: the plan the assistant proposed (JSON: title, text,
--   steps) and where it stands: none, proposed, approved, changes, cancelled.
-- pending: a question left without an answer when its turn ended (asked
--   aloud, or its card was never answered): 'plan' or 'language'. The user's
--   next message answers it.
CREATE TABLE IF NOT EXISTS chat_agent_state (
    conversation_id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    dev_language TEXT,
    dev_plugin TEXT,
    plan TEXT,
    plan_state TEXT NOT NULL DEFAULT 'none',
    plan_remark TEXT,
    pending TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
