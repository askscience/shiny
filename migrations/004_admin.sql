-- Plugin system admin gate. Adds an `is_admin` flag to travelers (the table
-- that backs the current auth identity) and auto-promotes the first created
-- traveler to admin so the first user can manage plugins without setting
-- ADMIN_TOKEN env. Subsequent users default to 0 (non-admin).

ALTER TABLE travelers ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;

-- Promote the earliest-created traveler to admin. This is idempotent on
-- re-runs because is_admin just gets set to 1 again.
UPDATE travelers
SET is_admin = 1
WHERE id = (
    SELECT id FROM travelers ORDER BY created_at ASC LIMIT 1
);