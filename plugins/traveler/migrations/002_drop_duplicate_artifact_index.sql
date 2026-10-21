-- Core migration 011 drops this duplicate for existing databases. This file
-- keeps a fresh install from recreating it after 001_init.sql.
DROP INDEX IF EXISTS idx_artifacts_traveler;
