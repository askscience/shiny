-- Traveler plugin schema. Owned by the `traveler` plugin; gated by the
-- `plugin_schema_versions` table maintained by the installer.

CREATE TABLE IF NOT EXISTS trips (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    start_time TEXT,
    end_time TEXT,
    status TEXT NOT NULL DEFAULT 'planned',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS locations (
    id TEXT PRIMARY KEY,
    trip_id TEXT REFERENCES trips(id),
    traveler_id TEXT NOT NULL,
    latitude REAL NOT NULL,
    longitude REAL NOT NULL,
    altitude REAL,
    speed REAL,
    heading REAL,
    accuracy REAL,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    source TEXT NOT NULL DEFAULT 'manual'
);

CREATE TABLE IF NOT EXISTS diary_entries (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    trip_id TEXT REFERENCES trips(id),
    date TEXT NOT NULL,
    title TEXT,
    content_markdown TEXT NOT NULL,
    summary TEXT,
    mood TEXT,
    tags TEXT,
    auto_generated INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS saved_artifacts (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    trip_id TEXT,
    artifact_type TEXT NOT NULL,
    title TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_trips_traveler ON trips(traveler_id);
CREATE INDEX IF NOT EXISTS idx_locations_trip ON locations(trip_id);
CREATE INDEX IF NOT EXISTS idx_locations_traveler ON locations(traveler_id);
CREATE INDEX IF NOT EXISTS idx_diary_traveler_date ON diary_entries(traveler_id, date);
CREATE INDEX IF NOT EXISTS idx_chat_traveler ON chat_messages(traveler_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_traveler ON saved_artifacts(traveler_id, updated_at DESC);