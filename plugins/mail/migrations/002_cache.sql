-- Mail plugin: local message cache.
-- Envelopes + parsed bodies are persisted here so the AI and the Mail window
-- read mail from SQLite instead of opening a fresh IMAP connection to the
-- provider (Gmail/etc.) on every list/read/search.

CREATE TABLE IF NOT EXISTS mail_messages (
    id TEXT PRIMARY KEY,              -- account_id + ':' + folder + ':' + uid
    user_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    folder TEXT NOT NULL,
    uid TEXT NOT NULL,
    message_id TEXT,
    subject TEXT NOT NULL DEFAULT '',
    from_addr TEXT NOT NULL DEFAULT '',
    to_addr TEXT NOT NULL DEFAULT '',
    cc_addr TEXT NOT NULL DEFAULT '',
    from_json TEXT NOT NULL DEFAULT '[]',
    to_json TEXT NOT NULL DEFAULT '[]',
    cc_json TEXT NOT NULL DEFAULT '[]',
    sent_at TEXT,
    body_text TEXT NOT NULL DEFAULT '',
    body_html TEXT NOT NULL DEFAULT '',
    attachments_json TEXT NOT NULL DEFAULT '[]',
    seen INTEGER NOT NULL DEFAULT 0,
    has_attachment INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    fetched_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_mail_messages_user ON mail_messages(user_id, account_id, folder);
CREATE INDEX IF NOT EXISTS idx_mail_messages_uid ON mail_messages(account_id, folder, uid);
CREATE INDEX IF NOT EXISTS idx_mail_messages_subject ON mail_messages(user_id, subject);
CREATE INDEX IF NOT EXISTS idx_mail_messages_from ON mail_messages(user_id, from_addr);

CREATE TABLE IF NOT EXISTS mail_sync_state (
    account_id TEXT NOT NULL,
    folder TEXT NOT NULL,
    user_id TEXT NOT NULL,
    last_synced_at TEXT,
    total INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, folder)
);
