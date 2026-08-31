-- Mail plugin: one mail account per user (credentials live here, v1 plaintext).
CREATE TABLE IF NOT EXISTS mail_accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    email TEXT NOT NULL,
    provider TEXT NOT NULL DEFAULT 'custom',
    imap_host TEXT NOT NULL,
    imap_port INTEGER NOT NULL DEFAULT 993,
    imap_security TEXT NOT NULL DEFAULT 'ssl',
    smtp_host TEXT NOT NULL,
    smtp_port INTEGER NOT NULL DEFAULT 465,
    smtp_security TEXT NOT NULL DEFAULT 'ssl',
    username TEXT NOT NULL,
    password TEXT NOT NULL,
    verified INTEGER NOT NULL DEFAULT 0,
    verified_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_mail_accounts_user ON mail_accounts(user_id);
