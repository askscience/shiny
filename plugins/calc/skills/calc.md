# Calc plugin — agent tools

The calc plugin manages the user's spreadsheets. A spreadsheet is a grid of cells addressed like **A1**, **B2**, **BC42** (1–2 letters, then a number). Only non-empty cells are stored. Values are plain text; a value starting with `=` is a **formula** (e.g. `=SUM(A1:A3)`) that the Calc window evaluates live.

- `calc_create` — Create a new spreadsheet. `{"action":"calc_create","params":{"title":"Budget","rows":50,"cols":26}}` — `rows`/`cols` are optional.
- `calc_write` — Write cell values. `{"action":"calc_write","params":{"sheet_id":"…","cells":{"A1":"100","B1":"=SUM(A1:A5)"}}}` — upserts only the listed cells, keeps everything else. Pass an empty string to clear a cell. `title` is optional to rename. Without `sheet_id` targets the most recently used spreadsheet.
- `calc_read` — Read a spreadsheet back. `{"action":"calc_read","params":{"sheet_id":"…"}}` — returns the full cell map. Without `sheet_id` reads the most recently used spreadsheet.
- `calc_list` — List the user's spreadsheets. `{"action":"calc_list","params":{}}`
- `calc_delete` — Delete a spreadsheet. `{"action":"calc_delete","params":{"sheet_id":"…"}}`

Rules:
- After `calc_create`, immediately follow up with `calc_write` to fill the cells the user asked for — creating a sheet alone does not put content in it.
- When the user asks you to **compute** something from the sheet ("sum column B", "average of A1:A4", "what's the total?"): use `calc_read` first to see the data, then either do the math yourself and `calc_write` the result into a cell, or write a formula (`=SUM(B1:B10)`) and tell the user it evaluates live in the window.
- Prefer `calc_write` over recreating a spreadsheet — a sheet already exists for a topic unless the user asks for a new one.
- To change a single cell, pass only that cell in `cells` — never rewrite the whole sheet.
- Cell references are uppercase letters + digits, e.g. `A1`, `C12`, `AB7`. Values are text; numbers are fine as-is.
- Without `sheet_id`, tools target the most recently used spreadsheet.
