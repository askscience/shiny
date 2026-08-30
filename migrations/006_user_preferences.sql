-- Per-user preferences: appearance, assistant name/model, plugin window
-- layout, and the desktop/tiling state (workspaces, focus, fullscreen, master
-- ratio). Every user gets their own fully isolated row space keyed by user_id,
-- so accounts can never see each other's desktop or files.
CREATE TABLE IF NOT EXISTS user_preferences (
    user_id TEXT NOT NULL REFERENCES travelers(id),
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, key)
);

CREATE INDEX IF NOT EXISTS idx_user_preferences_user ON user_preferences(user_id);
