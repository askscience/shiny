-- PDF plugin owns the `pdf_documents` table (self-contained schema).
-- Documents are stored as real .pdf file bytes.

CREATE TABLE IF NOT EXISTS pdf_documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES travelers(id),
    title TEXT NOT NULL DEFAULT 'Untitled',
    bytes BLOB NOT NULL,
    page_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_pdf_documents_user ON pdf_documents(user_id, updated_at DESC);
