-- Per-user Browser settings: the shield toggle and incognito preference.
--
-- A new file rather than an edit to 002: plugin migrations are recorded by
-- filename in the core-owned `plugin_schema_versions` table, so editing an
-- applied file never re-runs on existing installs (PLUGINS.md §9).
--
-- Keyed on `traveler_id` (the account id) with no foreign key: settings are a
-- convenience, and a missing/renamed user row must not make an upsert fail.
-- The plugin reads and writes this table best-effort, exactly like
-- `peakd_history`.

CREATE TABLE IF NOT EXISTS browser_settings (
    traveler_id TEXT PRIMARY KEY,
    -- Ad blocking is on by default, matching Brave/Falkon.
    adblock     INTEGER NOT NULL DEFAULT 1,
    -- Incognito is a mode the window enters, not a persisted default; the
    -- column exists so the last choice can be echoed back to the window.
    incognito   INTEGER NOT NULL DEFAULT 0,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
