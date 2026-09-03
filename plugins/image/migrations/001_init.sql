-- Image plugin owns the `images` table (self-contained schema).
-- Images are stored as PNG BLOBs: `bytes` is the current edited image,
-- `original` is the untouched upload (used by the "reset" operation).

CREATE TABLE IF NOT EXISTS images (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT 'Untitled',
    width INTEGER NOT NULL DEFAULT 0,
    height INTEGER NOT NULL DEFAULT 0,
    bytes BLOB NOT NULL,
    original BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_images_user ON images(user_id, updated_at DESC);
