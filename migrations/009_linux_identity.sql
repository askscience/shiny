-- Linux-user integration (Phase 1): bind a Shiny account to a real Linux
-- account. `unix_user` is the login name, `unix_uid`/`unix_home` are cached
-- from NSS at login so the Files plugin and the auth layer don't have to
-- re-resolve on every request. `auth_source` records how the account is
-- authenticated: 'local' (Argon2 hash) or 'pam' (real Linux password).
--
-- All columns are nullable/defaulted so existing rows and the current
-- single-user behaviour are unchanged when SHINY_LINUX_USERS is off.

ALTER TABLE travelers ADD COLUMN unix_user TEXT;
ALTER TABLE travelers ADD COLUMN unix_uid INTEGER;
ALTER TABLE travelers ADD COLUMN unix_home TEXT;
ALTER TABLE travelers ADD COLUMN auth_source TEXT NOT NULL DEFAULT 'local';

CREATE UNIQUE INDEX IF NOT EXISTS idx_travelers_unix_user
    ON travelers(unix_user) WHERE unix_user IS NOT NULL;
