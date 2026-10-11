-- Minimal layer model.
--
-- The editor no longer supports folders, masks, blend modes, opacity or free
-- layer placement: a layer is a full-canvas-ish RGBA patch stacked bottom-to-top
-- and composited with plain source-over. This migration rebuilds `image_layers`
-- and drops the document selection mask, keeping only the pixel layers.

-- Folders carry no pixels — drop them (their children become root layers).
CREATE TABLE image_layers_new (
    id            TEXT PRIMARY KEY,
    image_id      TEXT NOT NULL,
    user_id       TEXT NOT NULL,
    name          TEXT NOT NULL DEFAULT 'Layer',
    position      INTEGER NOT NULL DEFAULT 0,
    visible       INTEGER NOT NULL DEFAULT 1,
    width         INTEGER NOT NULL DEFAULT 0,
    height        INTEGER NOT NULL DEFAULT 0,
    bytes         BLOB NOT NULL,
    original      BLOB NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO image_layers_new
    (id, image_id, user_id, name, position, visible, width, height, bytes, original, created_at, updated_at)
SELECT
    id, image_id, user_id, name, position, visible, width, height, bytes, original, created_at, updated_at
FROM image_layers
WHERE is_group = 0 AND width > 0 AND height > 0
ORDER BY image_id, position;

DROP TABLE image_layers;
ALTER TABLE image_layers_new RENAME TO image_layers;
CREATE INDEX IF NOT EXISTS idx_image_layers_doc ON image_layers(image_id, position);

-- Drop the per-document selection mask (the selection tools are gone).
CREATE TABLE images_new (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    title       TEXT NOT NULL DEFAULT 'Untitled',
    width       INTEGER NOT NULL DEFAULT 0,
    height      INTEGER NOT NULL DEFAULT 0,
    bytes       BLOB NOT NULL,
    original    BLOB NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    format      TEXT NOT NULL DEFAULT 'png',
    orig_width  INTEGER NOT NULL DEFAULT 0,
    orig_height INTEGER NOT NULL DEFAULT 0
);

INSERT INTO images_new
    (id, user_id, title, width, height, bytes, original, created_at, updated_at, format, orig_width, orig_height)
SELECT
    id, user_id, title, width, height, bytes, original, created_at, updated_at, format, orig_width, orig_height
FROM images;

DROP TABLE images;
ALTER TABLE images_new RENAME TO images;
CREATE INDEX IF NOT EXISTS idx_images_user ON images(user_id, updated_at DESC);
