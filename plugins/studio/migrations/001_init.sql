-- Studio plugin owns the `studio_tracks` table (self-contained schema).
-- A track is one pattern (config_json) plus its last render (wav BLOB).
-- config_json is the single shared contract between the AI tools, the REST
-- routes, and the Studio window — see skills/studio.md.

CREATE TABLE IF NOT EXISTS studio_tracks (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT 'Untitled',
    bpm REAL NOT NULL DEFAULT 120,
    steps INTEGER NOT NULL DEFAULT 16,
    tuning TEXT NOT NULL DEFAULT 'edo12',
    config_json TEXT NOT NULL DEFAULT '{}',
    duration_ms INTEGER NOT NULL DEFAULT 0,
    wav BLOB,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_studio_tracks_user ON studio_tracks(user_id, updated_at DESC);
