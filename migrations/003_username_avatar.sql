ALTER TABLE travelers ADD COLUMN username TEXT;
ALTER TABLE travelers ADD COLUMN avatar TEXT;

UPDATE travelers SET username = lower(substr(email, 1, instr(email, '@') - 1))
WHERE username IS NULL AND instr(email, '@') > 0;

UPDATE travelers SET username = lower(replace(email, ' ', '_'))
WHERE username IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_travelers_username ON travelers(username);
