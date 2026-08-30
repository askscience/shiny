-- Calc plugin owns the `spreadsheets` table (self-contained schema).
-- Cells are stored as a JSON map "A1" -> "value" (formulas start with "="
-- and are evaluated client-side in the Calc window).

CREATE TABLE IF NOT EXISTS spreadsheets (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES travelers(id),
    title TEXT NOT NULL DEFAULT 'Untitled',
    cells TEXT NOT NULL DEFAULT '{}',
    rows INTEGER NOT NULL DEFAULT 100,
    cols INTEGER NOT NULL DEFAULT 26,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_spreadsheets_user ON spreadsheets(user_id, updated_at DESC);
