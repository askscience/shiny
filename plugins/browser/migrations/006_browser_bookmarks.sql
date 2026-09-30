-- Per-user bookmarks for the Browser window.
--
-- A new file rather than an edit to an applied one: plugin migrations are
-- recorded by filename in `plugin_schema_versions` and never re-run
-- (PLUGINS.md §9).
--
-- Keyed on a generated `id`; `traveler_id` (the account id) scopes the list
-- with no foreign key, exactly like `peakd_history` and `browser_settings`.
-- Bookmarking is a convenience, so a broken database must never make it fail:
-- every statement in `src/bookmarks.rs` is best-effort.
--
-- Duplicate URLs are collapsed in code rather than by a UNIQUE constraint, so
-- re-bookmarking a page updates its title instead of raising and losing the
-- edit. Idempotent (`CREATE TABLE IF NOT EXISTS`) so a re-install is safe.

CREATE TABLE IF NOT EXISTS browser_bookmarks (
    id          TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    url         TEXT NOT NULL,
    title       TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- The window lists a user's bookmarks newest-first.
CREATE INDEX IF NOT EXISTS idx_browser_bookmarks_traveler_created
    ON browser_bookmarks (traveler_id, created_at DESC);
