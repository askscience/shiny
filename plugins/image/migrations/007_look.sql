-- The selected look is a live selection over the current pixels, not a baked
-- effect: `images.look` records it, and the rendered picture is `look(bytes)`.
-- Picking another look simply replaces it. Any other edit (adjustments,
-- one-clicks, geometry, AI) bakes the selected look into `bytes` first and
-- clears this column, so real edits accumulate on what the user sees.

ALTER TABLE images ADD COLUMN look TEXT;
