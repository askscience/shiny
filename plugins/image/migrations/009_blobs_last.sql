-- Move the pixel buffers to the end of the row.
--
-- SQLite stores a row's columns in order, and a large BLOB spills into a
-- chain of overflow pages that has to be walked to reach any column after
-- it. `bytes` and `original` sat in the middle, so reading `updated_at`,
-- `format`, `look` or `adjust` (the gallery list, every render) walked both
-- buffers of every row: the list of 17 pictures read 116 MB. With the
-- buffers last, the small columns are read from the row's first page, and
-- `bytes` (the working pixels) is reached without touching `original`.
CREATE TABLE images_new (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    title       TEXT NOT NULL DEFAULT 'Untitled',
    width       INTEGER NOT NULL DEFAULT 0,
    height      INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    format      TEXT NOT NULL DEFAULT 'png',
    orig_width  INTEGER NOT NULL DEFAULT 0,
    orig_height INTEGER NOT NULL DEFAULT 0,
    look        TEXT,
    adjust      TEXT,
    bytes       BLOB NOT NULL,
    original    BLOB NOT NULL
);

INSERT INTO images_new
    (id, user_id, title, width, height, created_at, updated_at, format, orig_width, orig_height, look, adjust, bytes, original)
SELECT
    id, user_id, title, width, height, created_at, updated_at, format, orig_width, orig_height, look, adjust, bytes, original
FROM images;

DROP TABLE images;
ALTER TABLE images_new RENAME TO images;
CREATE INDEX IF NOT EXISTS idx_images_user ON images(user_id, updated_at DESC);
