-- Per-user plugin activation set. Each user has their own workspace: a plugin
-- is "enabled" for a user unless this table has a row with enabled=0 for them.
-- New users default to enabled for every plugin discovered on the server.

CREATE TABLE IF NOT EXISTS user_plugin_states (
    user_id TEXT NOT NULL REFERENCES travelers(id),
    plugin_name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, plugin_name)
);

CREATE INDEX IF NOT EXISTS idx_user_plugin_states_user ON user_plugin_states(user_id);