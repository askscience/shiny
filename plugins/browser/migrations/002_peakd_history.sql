-- Browsing history for the in-app Browser window, and the raw material for the
-- home surface's related-news cards.
--
-- Why a second migration rather than an edit to 001: migrations are recorded by
-- filename in the core-owned `plugin_schema_versions` table. Editing 001 would
-- *not* re-run on any install that already applied it, so the new `query`
-- column would silently never exist on existing installs. A new file runs
-- exactly once, everywhere.
--
-- The history table is `peakd_history`. An earlier revision of `src/history.rs`
-- read and wrote `browser_history` instead — the table of the plugin's former
-- `browser` name — so on a clean install every record and read failed silently
-- (the module swallows DB errors by design, and reading history was not wired
-- to any route yet). `browser_history` is dropped below; it was a stale
-- artifact of the rename and nothing has read it since.
--
-- Idempotent (PLUGINS.md §17): a plugin archive may be re-installed, and the
-- migration runner must never fail on a second run. Keep every statement
-- self-contained: the runner feeds the whole file through `sqlx::raw_sql`, so a
-- statement that references a table which may not exist would take the rest of
-- the file down with it.
--
-- The foreign key is intentionally loose. History is a convenience: the insert
-- in `src/history.rs` is guarded by an `EXISTS` check so a user without a
-- matching `travelers` row records nothing rather than raising a constraint
-- error on every page load.

CREATE TABLE IF NOT EXISTS peakd_history (
    id          TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    url         TEXT NOT NULL,
    mode        TEXT NOT NULL DEFAULT 'page',
    -- The raw input the user typed, when this navigation was a search. NULL for
    -- a plain URL visit. This is the strongest interest signal the
    -- recommendation profile has, so it is stored verbatim rather than parsed.
    query       TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- The window lists a user's history newest-first, and the recommender reads
-- the same rows when building the interest profile.
CREATE INDEX IF NOT EXISTS idx_peakd_history_traveler_created
    ON peakd_history (traveler_id, created_at DESC);

-- Retire the stale table left by the `browser` → `peakd` rename. It has not
-- been written to since the rename, so it is a dead artifact rather than data
-- the plugin still owns.
DROP TABLE IF EXISTS browser_history;
