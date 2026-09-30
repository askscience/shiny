-- The user's chosen search engine for the Browser's address bar.
--
-- A new file rather than an edit to 003: plugin migrations are recorded by
-- filename in the core-owned `plugin_schema_versions` table, so editing an
-- applied file never re-runs on existing installs (PLUGINS.md §9). A new file
-- runs exactly once, everywhere.
--
-- SQLite cannot express conditional DDL, so a bare `ALTER TABLE … ADD COLUMN`
-- would abort a replay with "duplicate column name" on a database that already
-- has it. That is safe here because the runner records the filename and this
-- file therefore runs once. The plugin *also* repairs the column itself in
-- `src/settings.rs::ensure_schema` (guarded by a `pragma_table_info` check),
-- which is what covers an install that applied this file before the
-- search-engine setting shipped — the same split used for `peakd_history.query`
-- in 002.
--
-- The default is `duckduckgo`. A self-hosted `SEARXNG_URL` still overrides any
-- named engine (see `src/routes.rs::Target::into_url`).

ALTER TABLE browser_settings ADD COLUMN search_engine TEXT NOT NULL DEFAULT 'duckduckgo';
