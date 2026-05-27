CREATE TABLE IF NOT EXISTS saved_artifacts (
    id TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL REFERENCES travelers(id),
    trip_id TEXT REFERENCES trips(id),
    artifact_type TEXT NOT NULL,
    title TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_saved_artifacts_traveler ON saved_artifacts(traveler_id, updated_at DESC);
