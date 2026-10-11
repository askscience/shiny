-- Live dial settings, not baked edits.
--
-- `images.adjust` is a JSON object of the dial values ({"brightness":30,…}).
-- The rendered picture is `look(settings(bytes))`: dials stay where the user
-- put them, fine-tuning replaces the value instead of stacking, and any
-- destructive edit (one-clicks, geometry, AI operations) bakes the live state
-- into `bytes` and clears both this column and `look`.

ALTER TABLE images ADD COLUMN adjust TEXT;
