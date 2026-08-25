-- Core-owned document storage for the word plugin (same pattern as
-- saved_artifacts: core owns the table, plugins own the domain logic).
-- Documents are stored as real .odt (OpenDocument Text) file bytes.

CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES travelers(id),
    title TEXT NOT NULL DEFAULT 'Untitled',
    odt BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_documents_user ON documents(user_id, updated_at DESC);