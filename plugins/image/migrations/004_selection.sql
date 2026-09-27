-- Active selection mask for a document.
--
-- A selection is stored as a canvas-sized, single-channel 8-bit coverage mask
-- (0 = unselected, 255 = fully selected). The editor computes it in the browser
-- (marquee, lasso, magic wand, select-all) and uploads it once per change; the
-- operations engine crops it to the target layer's rectangle and confines every
-- non-geometric operation to it. Geometry operations clear it.

ALTER TABLE images ADD COLUMN selection BLOB;
ALTER TABLE images ADD COLUMN selection_width INTEGER NOT NULL DEFAULT 0;
ALTER TABLE images ADD COLUMN selection_height INTEGER NOT NULL DEFAULT 0;
