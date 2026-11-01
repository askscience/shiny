-- When the Word window last opened a document. Tools without a `doc_id`
-- target the document the user has open (latest of opened/updated), so
-- "this document" in the chat means the one on screen.

ALTER TABLE documents ADD COLUMN opened_at TEXT;
