-- Updates plugin: audit trail of every check/refresh/apply/ollama action.
-- `output` holds the (truncated) command log so a failed upgrade is diagnosable
-- from the window later.
CREATE TABLE IF NOT EXISTS updates_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    action TEXT NOT NULL,          -- refresh | apply | ollama | check
    manager TEXT,                  -- apt | dnf | pacman | ... | ollama
    detail TEXT,                   -- human summary (packages / "all")
    status TEXT NOT NULL,          -- ok | error
    output TEXT,                   -- truncated command log
    at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_updates_history_user
    ON updates_history(user_id, id DESC);
