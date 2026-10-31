-- Undo / redo history for PDF edits.
--
-- Every committed edit snapshots the bytes it replaced as an 'undo' row; undo
-- swaps the newest one back in and keeps the current bytes as a 'redo' row. A
-- fresh edit clears the redo rows. Only the last few revisions per document
-- are kept (see `store::KEEP`).

CREATE TABLE IF NOT EXISTS pdf_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pdf_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'undo',
    label TEXT NOT NULL DEFAULT '',
    bytes BLOB NOT NULL,
    page_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_pdf_revisions_doc ON pdf_revisions(pdf_id, user_id, kind, id DESC);
