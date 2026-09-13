-- Browsing history for the in-app Browser window.
--
-- Idempotent (PLUGINS.md §17): a plugin archive may be re-installed, and the
-- migration runner must never fail on a second run.
--
-- The foreign key is intentionally loose. History is a convenience: the
-- insert in `src/history.rs` is guarded by an `EXISTS` check so a user without
-- a matching `travelers` row records nothing rather than raising a constraint
-- error on every page load.

CREATE TABLE IF NOT EXISTS peakd_history (
    id          TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    url         TEXT NOT NULL,
    mode        TEXT NOT NULL DEFAULT 'page',
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- The window lists a user's history newest-first.
CREATE INDEX IF NOT EXISTS idx_peakd_history_traveler_created
    ON peakd_history (traveler_id, created_at DESC);
