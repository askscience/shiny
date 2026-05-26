CREATE TABLE IF NOT EXISTS travelers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    auth_token TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS trips (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL REFERENCES travelers(id),
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
    traveler_id TEXT NOT NULL REFERENCES travelers(id),
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
    traveler_id TEXT NOT NULL REFERENCES travelers(id),
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
    traveler_id TEXT NOT NULL REFERENCES travelers(id),
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_trips_traveler ON trips(traveler_id);
CREATE INDEX IF NOT EXISTS idx_locations_trip ON locations(trip_id);
CREATE INDEX IF NOT EXISTS idx_locations_traveler ON locations(traveler_id);
CREATE INDEX IF NOT EXISTS idx_diary_traveler_date ON diary_entries(traveler_id, date);
CREATE INDEX IF NOT EXISTS idx_chat_traveler ON chat_messages(traveler_id);
