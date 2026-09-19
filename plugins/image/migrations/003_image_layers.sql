-- Compositor-style layer model.
--
-- A row in `images` is now a *document*; its pixels are the flattened
-- composite cache. The real source of truth is `image_layers`, an ordered
-- (bottom-to-top) stack of pixel layers and folders that the compositor
-- blends together.
--
-- A pixel layer stores its own local rectangle (`width` × `height`, raw RGBA
-- in `bytes`) placed on the document canvas at (`x`, `y`). `original` keeps
-- the layer's untouched pixels so an edit can be reset. `opacity` (0..1) and
-- `blend_mode` mirror Compositor's non-destructive layer appearance, and
-- `mask` (raw 8-bit grayscale coverage, optional) multiplies the layer alpha.
--
-- Groups carry `is_group = 1` and no pixels; their children point at them via
-- `group_id`. `position` orders siblings within the same parent (NULL = root).

CREATE TABLE IF NOT EXISTS image_layers (
    id            TEXT PRIMARY KEY,
    image_id      TEXT NOT NULL,
    user_id       TEXT NOT NULL,
    name          TEXT NOT NULL DEFAULT 'Layer',
    position      INTEGER NOT NULL DEFAULT 0,
    visible       INTEGER NOT NULL DEFAULT 1,
    opacity       REAL NOT NULL DEFAULT 1.0,
    blend_mode    TEXT NOT NULL DEFAULT 'normal',
    x             INTEGER NOT NULL DEFAULT 0,
    y             INTEGER NOT NULL DEFAULT 0,
    width         INTEGER NOT NULL DEFAULT 0,
    height        INTEGER NOT NULL DEFAULT 0,
    bytes         BLOB NOT NULL,
    original      BLOB NOT NULL,
    mask          BLOB,
    mask_width    INTEGER NOT NULL DEFAULT 0,
    mask_height   INTEGER NOT NULL DEFAULT 0,
    mask_enabled  INTEGER NOT NULL DEFAULT 1,
    is_group      INTEGER NOT NULL DEFAULT 0,
    group_id      TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_image_layers_doc ON image_layers(image_id, position);
CREATE INDEX IF NOT EXISTS idx_image_layers_parent ON image_layers(image_id, group_id, position);

-- Every existing single-image document becomes a one-layer stack.
INSERT INTO image_layers
    (id, image_id, user_id, name, position, visible, opacity, blend_mode,
     x, y, width, height, bytes, original, mask_enabled, is_group, group_id,
     created_at, updated_at)
SELECT
    lower(hex(randomblob(16))), i.id, i.user_id, 'Background', 0, 1, 1.0, 'normal',
    0, 0, i.width, i.height, i.bytes, i.original, 1, 0, NULL,
    i.created_at, i.updated_at
FROM images i
WHERE NOT EXISTS (SELECT 1 FROM image_layers l WHERE l.image_id = i.id);
