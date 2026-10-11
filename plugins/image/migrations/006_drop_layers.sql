-- Single-image documents.
--
-- The layer stack is gone: a document is one picture. `images.bytes` already
-- holds the flattened composite (every layer mutation called
-- `refresh_composite`), so the stack table can simply be dropped and the
-- adjusted image survives as-is.

DROP TABLE IF EXISTS image_layers;
