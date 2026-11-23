-- Column widths a person dragged in the Calc window, as a JSON map
-- column index (0-based, as text) -> width in CSS pixels. Columns not listed
-- have the default width.

ALTER TABLE spreadsheets ADD COLUMN col_widths TEXT NOT NULL DEFAULT '{}';
