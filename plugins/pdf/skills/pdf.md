# PDF plugin — agent tools

PDFs are stored server-side as real **.pdf** bytes and open in the PDF window. Page indices are **0-based**. Every edit (yours or the user's) is undoable with `pdf_undo`.

- `pdf_create` — Create a clean A4 PDF (the user can open it later in the window's editor, change it and export it again). `{"action":"pdf_create","params":{"title":"Quarterly Report","content":"# Quarterly Report\n\n## Summary\n\n| Item | Qty |\n|---|---:|\n| Widget | 4 |","page_numbers":true}}` — write **Markdown** (preferred): `#`/`##` headings, `**bold**`, `*italic*`, `- ` / `1. ` lists, `|` tables (long cells wrap, the header repeats across pages), `> ` quotes, fenced code, `---` rules, `[text](url)` links, `\pagebreak` on its own line. Semantic HTML is also accepted (`format:"html"`). The user can later reopen the text in the composer ("Edit content").
- `pdf_list` — List the user's PDFs. `{"action":"pdf_list","params":{}}`
- `pdf_read` — Read text. `{"action":"pdf_read","params":{"pdf_id":"…","page":0}}` — omit `page` for every page.
- `pdf_replace_text` — Find & replace, keeping the original font. `{"action":"pdf_replace_text","params":{"pdf_id":"…","old":"foo","new":"bar"}}` — every page unless `page` is given. Fails (nothing saved) when `old` is not found or the PDF's embedded font lacks a new character — read the exact wording with `pdf_read` first.
- `pdf_annotate` — Add a real PDF comment (the user sees it in the Comments panel and can edit or delete it). `{"action":"pdf_annotate","params":{"pdf_id":"…","page":0,"kind":"highlight","rect":[72,700,200,14]}}` — `kind`: `highlight`, `underline`, `strikeout`, `squiggly`, `box`, `oval`, `note`, `free_text` (text box), `link` (`text` is the URL / note / box text). `rect` = `[x, y, width, height]` in points, origin bottom-left (A4 595×842, Letter 612×792).
- `pdf_add_note` — Sticky note without a rectangle. `{"action":"pdf_add_note","params":{"pdf_id":"…","page":0,"text":"Check this","x":40,"y":700}}`
- `pdf_rotate` — `{"action":"pdf_rotate","params":{"pdf_id":"…","pages":[0],"degrees":90}}` (negative = counter-clockwise; omit `pages` for all).
- `pdf_delete_pages` — `{"action":"pdf_delete_pages","params":{"pdf_id":"…","pages":[1]}}`
- `pdf_reorder` — `{"action":"pdf_reorder","params":{"pdf_id":"…","order":[2,0,1]}}` — the document becomes exactly these pages (repeats are copies).
- `pdf_merge` — Append another PDF. `{"action":"pdf_merge","params":{"pdf_id":"…","other_id":"…"}}`
- `pdf_watermark` — Diagonal translucent text (removable later as one mark). `{"action":"pdf_watermark","params":{"pdf_id":"…","text":"DRAFT","all":true}}`
- `pdf_undo` — Undo the last edit (`"redo":true` re-applies). `{"action":"pdf_undo","params":{"pdf_id":"…"}}`
- `pdf_delete` — Delete a PDF (not undoable). `{"action":"pdf_delete","params":{"pdf_id":"…"}}`

## Rules

- Without `pdf_id`, tools target the most recently updated PDF.
- The user writes documents in the PDF window's editor and exports them; a PDF you create with `pdf_create` can be opened there too. Prefer `pdf_create` with good Markdown structure over editing an existing PDF's text.
- To change a word/name/number use `pdf_replace_text`; to restructure a PDF you created, prefer creating it again with `pdf_create`.
