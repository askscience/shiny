-- The browsing history under the shell's new name, LiteAura.
--
-- Until the rename the table was `peakd_history` (002). A migration cannot
-- move its rows idempotently: the copy has to name the `query` column, which
-- some installs only get from `history::ensure_schema` after the migrations
-- run. So this file only creates the new table, and `ensure_schema` copies
-- whatever is left in `peakd_history` on load and then drops it. 002 stays
-- unchanged: it is recorded by filename, and a fresh install still runs it
-- (its empty `peakd_history` is dropped on the first load).
--
-- Idempotent (PLUGINS.md §17).

CREATE TABLE IF NOT EXISTS liteaura_history (
    id          TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    url         TEXT NOT NULL,
    mode        TEXT NOT NULL DEFAULT 'page',
    -- The raw input the user typed, when this navigation was a search. NULL for
    -- a plain URL visit.
    query       TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_liteaura_history_traveler_created
    ON liteaura_history (traveler_id, created_at DESC);
