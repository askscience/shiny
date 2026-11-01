-- How a document is composed on export (theme, paper, cover, contents,
-- header, page numbers, subtitle/author/date): JSON, see `engine::Design`.
-- The `odt` column keeps holding the content only.

ALTER TABLE documents ADD COLUMN design TEXT NOT NULL DEFAULT '{}';
