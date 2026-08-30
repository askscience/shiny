-- Impress plugin owns the `presentations` table (self-contained schema).
-- Slides are stored as a JSON array of the SDK `Slide` model; real .odp
-- bytes are produced/consumed by the SDK `odp` codec on export/import.

CREATE TABLE IF NOT EXISTS presentations (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES travelers(id),
    title TEXT NOT NULL DEFAULT 'Untitled',
    slides TEXT NOT NULL DEFAULT '[]',
    theme TEXT NOT NULL DEFAULT 'aurora',
    aspect TEXT NOT NULL DEFAULT '16x9',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_presentations_user ON presentations(user_id, updated_at DESC);
