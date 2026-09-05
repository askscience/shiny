-- Studio plugin: user presets (named parameter snapshots per instrument kind).
-- params_json holds the voice fields to apply: { wave?, synth?, fx?, level?, pan? }.

CREATE TABLE IF NOT EXISTS studio_presets (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    params_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_studio_presets_user ON studio_presets(user_id, kind, name);
