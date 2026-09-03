-- Calculator plugin owns the `calculator_history` table (self-contained schema).
-- Each row is one evaluated expression, kept so the AI and the Calculator
-- window share a single history log.

CREATE TABLE IF NOT EXISTS calculator_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    expression TEXT NOT NULL,
    result TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_calculator_history_user ON calculator_history(user_id, id DESC);
