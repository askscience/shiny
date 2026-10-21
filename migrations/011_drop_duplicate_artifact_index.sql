-- The traveler plugin's 001 migration created an index identical to
-- idx_saved_artifacts_traveler from core 002. Drop the duplicate for existing
-- databases; traveler migration 002 handles fresh installs.
DROP INDEX IF EXISTS idx_artifacts_traveler;
