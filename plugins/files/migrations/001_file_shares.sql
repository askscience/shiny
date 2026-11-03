-- Public share links created through the managed Gokapi server (src/share).
--
-- Gokapi stores the file and serves the link; this table only records which
-- Shiny account owns each Gokapi file id, so "my shared links" and revoke are
-- per-user even though every upload goes through one host-wide API key.
--
-- Keyed on the Gokapi file id with no foreign key on `owner`: a deleted
-- account must not make the insert fail (same convention as browser_settings).

CREATE TABLE IF NOT EXISTS file_shares (
    id            TEXT PRIMARY KEY,
    owner         TEXT NOT NULL,
    path          TEXT NOT NULL,
    name          TEXT NOT NULL,
    url           TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    -- Unix seconds; 0 means the link never expires by time.
    expires_at    INTEGER NOT NULL DEFAULT 0,
    -- 0 means unlimited downloads.
    max_downloads INTEGER NOT NULL DEFAULT 0,
    has_password  INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS file_shares_owner ON file_shares(owner);
