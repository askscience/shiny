-- hello plugin: no DB schemas — empty migrations dir keeps the layout canonical.
CREATE TABLE IF NOT EXISTS hello_pings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    who TEXT NOT NULL,
    at TEXT NOT NULL DEFAULT (datetime('now'))
);