-- PDFs created in the app keep the text they were made from, so the window
-- can reopen it in the composer and regenerate the document (an undoable
-- edit). Imported PDFs have no source.
ALTER TABLE pdf_documents ADD COLUMN source TEXT;
ALTER TABLE pdf_documents ADD COLUMN source_format TEXT;
ALTER TABLE pdf_documents ADD COLUMN page_numbers INTEGER NOT NULL DEFAULT 0;
