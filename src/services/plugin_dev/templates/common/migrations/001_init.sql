-- @@TITLE@@: the plugin's own table. Rows belong to one user (`user_id`).
-- A migration that has run is never edited: add 002_….sql for a change.
CREATE TABLE IF NOT EXISTS @@IDENT@@_items (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_@@IDENT@@_items_user ON @@IDENT@@_items(user_id, created_at);
