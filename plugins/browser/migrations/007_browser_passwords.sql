-- The Browser's password vault (src/vault.rs).
--
-- A new file rather than an edit to an applied one: plugin migrations are
-- recorded by filename in `plugin_schema_versions` and never re-run
-- (PLUGINS.md §9). Idempotent so a re-install is safe.
--
-- `browser_vault` holds one row per account: the Argon2id salt and cost the
-- login password is stretched with, and the random data key sealed under the
-- result (AES-256-GCM, base64 `nonce || ciphertext`). Changing the login
-- password only re-seals this key.
--
-- `browser_passwords` holds the entries. Everything a user typed — site,
-- username, password, note — is one sealed JSON document under the data key;
-- only the id, the owner and timestamps are plaintext, so the table does not
-- even reveal which sites the user has accounts on.

CREATE TABLE IF NOT EXISTS browser_vault (
    traveler_id TEXT PRIMARY KEY,
    salt        TEXT NOT NULL,
    m_cost      INTEGER NOT NULL,
    t_cost      INTEGER NOT NULL,
    p_cost      INTEGER NOT NULL,
    wrapped_key TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS browser_passwords (
    id          TEXT PRIMARY KEY,
    traveler_id TEXT NOT NULL,
    sealed      TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_browser_passwords_traveler
    ON browser_passwords (traveler_id);
