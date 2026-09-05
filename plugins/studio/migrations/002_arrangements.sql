-- Studio plugin: arrangements (multi-clip timeline layouts).
-- config_json holds the full arrangement: { bpm, length_beats, master, tracks[], clips[] }.

CREATE TABLE IF NOT EXISTS studio_arrangements (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT 'Untitled',
    bpm REAL NOT NULL DEFAULT 120,
    length_beats REAL NOT NULL DEFAULT 32,
    master REAL NOT NULL DEFAULT 0.9,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_studio_arrangements_user ON studio_arrangements(user_id, updated_at DESC);
