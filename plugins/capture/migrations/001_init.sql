-- Capture: per-user settings and live destinations. Recordings and projects
-- are files in the user's Videos folder, not rows.
CREATE TABLE IF NOT EXISTS capture_settings (
    traveler_id TEXT PRIMARY KEY,
    settings TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS capture_destinations (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    name TEXT NOT NULL,
    server TEXT NOT NULL DEFAULT '',
    -- The stream key, AES-256-GCM sealed (shiny-vault) under a machine key.
    key_sealed TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    kbps INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS capture_destinations_owner ON capture_destinations (traveler_id);
