-- Documents written in the window's editor keep their HTML as `source`
-- (`source_format = 'html'`); their PDF is printed by the shell and uploaded.
-- `pdf_stale` says the source changed since that PDF was made.
ALTER TABLE pdf_documents ADD COLUMN pdf_stale INTEGER NOT NULL DEFAULT 0;
