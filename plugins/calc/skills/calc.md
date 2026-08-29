# Calc plugin — agent tools

Spreadsheets are just JSON to you. A spreadsheet is a grid of cells addressed like **A1**, **B2**, **BC42**; only non-empty cells are stored, as a JSON map `"A1" -> "value"`. Values are text; a value starting with `=` is a formula (e.g. `=SUM(A1:A3)`) the Calc window evaluates live.

**The JSON contract:**
- `calc_read` — read a spreadsheet: `{"action":"calc_read","params":{"sheet_id":"…"}}` → returns `{ sheet_id, title, cells: { "A1": "100", "B1": "=SUM(A1:A3)", … } }`. The `cells` map IS the JSON you write back with `calc_write`.
- `calc_write` — merge cell values: `{"action":"calc_write","params":{"sheet_id":"…","cells":{"A1":"100","B1":"=SUM(A1:A3)"}}}` → upserts only the listed cells and keeps everything else. An empty-string value (`"A1":""`) clears that cell. `cells` may also be an array of `[["A1","100"],["B1","200"]]` pairs, a single `{"ref":"A1","value":"100"}`, or bare `{"A1":"100"}` next to `sheet_id`. `title` optionally renames the sheet.
- `calc_clear` — clear ALL cell values (keeps the sheet): `{"action":"calc_clear","params":{"sheet_id":"…"}}`
- `calc_create` — new spreadsheet: `{"action":"calc_create","params":{"title":"Budget","cells":{"A1":"Item","B1":"Cost"}}}` — `cells` is optional but **strongly recommended**: it creates AND fills the sheet in one call, so you never leave an empty document. Returns the new `sheet_id`.
- `calc_list` — list spreadsheets: `{"action":"calc_list","params":{}}`
- `calc_delete` — permanently delete a spreadsheet (needs `{"confirm":true}`) — only when the user explicitly asks to delete the whole document.

Rules (CRITICAL — never leave an empty sheet, never wipe the user's data):
- **When the user asks you to create a spreadsheet, you MUST put initial content in it before replying.** Either pass `cells` inside `calc_create`, or call `calc_write` immediately after. If the user didn't specify the columns, invent a sensible starter layout for the topic (e.g. for a room list: A1="Room", B1="Guests", C1="Notes", plus a couple of example rows). NEVER reply "I created the sheet" with an empty document, and never ask the user for details when a reasonable default exists.
- **ALWAYS pass the `sheet_id`** you got from `calc_read`/`calc_list`/`calc_create` — never write without knowing which sheet.
- **NEVER call `calc_delete`** unless the user explicitly asks to delete the whole spreadsheet/document.
- **NEVER clear the whole sheet with `calc_write`** (empty strings for most cells) — the tool refuses; use `calc_clear` instead.
- To change one cell, pass only that cell — do not echo back cells you aren't changing.
- To **compute** ("sum column B", "what's the total?"): `calc_read` first, then do the math yourself and `calc_write` the result, or write a formula like `=SUM(B1:B10)` and tell the user it evaluates live.
