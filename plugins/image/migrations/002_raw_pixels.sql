-- Store images as raw RGBA instead of encoded PNG so the edit hot path does
-- no codec work. `format` marks old (png) vs new (rgba) rows; legacy rows are
-- decoded and upgraded lazily on first touch. `orig_*` keeps the reset target
-- dimensions (an image can be cropped/rotated after upload).

ALTER TABLE images ADD COLUMN format TEXT NOT NULL DEFAULT 'png';
ALTER TABLE images ADD COLUMN orig_width INTEGER NOT NULL DEFAULT 0;
ALTER TABLE images ADD COLUMN orig_height INTEGER NOT NULL DEFAULT 0;
