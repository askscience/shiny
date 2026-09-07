# PDF plugin — agent tools

The pdf plugin manages PDF documents for the user. PDFs are stored server-side as real **.pdf** file bytes (rendered and edited with `pdf_oxide`) and open in the PDF window.

- `pdf_create` — Create a professionally-styled PDF. `{"action":"pdf_create","params":{"title":"Quarterly Report","content":"<h1>Quarterly Report</h1><h2>Summary</h2><table><tr><th>Item</th><th>Qty</th></tr><tr><td>Widget</td><td>4</td></tr></table>"}}` — `format` is `"html"` (default), `"markdown"`, or `"plain"`. Write the content as semantic HTML (`h1`-`h4`, `p`, `ul`/`ol`/`li`, `table`, `strong`/`b`, `em`/`i`, `blockquote`, `pre`/`code`, `hr`, `br`) **or** as Markdown — both are accepted. The renderer applies a clean professional style automatically. Use `"format":"plain"` only when the user explicitly asks for plain/raw text.
- `pdf_list` — List the user's PDFs. `{"action":"pdf_list","params":{}}` returns `pdf_id`, `title`, `page_count`, `updated_at` per document.
- `pdf_read` — Read the text of a PDF (one page or the whole document). `{"action":"pdf_read","params":{"pdf_id":"…","page":0}}` — omit `page` to read every page. Returns `content` and `page_count`.
- `pdf_rotate` — Rotate page(s). `{"action":"pdf_rotate","params":{"pdf_id":"…","pages":[0,2],"degrees":90}}` — `degrees` must be a multiple of 90 (clockwise). Omit `pages` (or set `"all":true`) to rotate every page.
- `pdf_delete_pages` — Remove pages. `{"action":"pdf_delete_pages","params":{"pdf_id":"…","pages":[1]}}` (0-based indices).
- `pdf_reorder` — Reorder/keep pages. `{"action":"pdf_reorder","params":{"pdf_id":"…","order":[2,0,1]}}` — the document becomes exactly these pages, in this order.
- `pdf_merge` — Append another PDF to this one. `{"action":"pdf_merge","params":{"pdf_id":"…","other_id":"…"}}` — `other_id`'s pages are added to the end of `pdf_id`. Without `pdf_id` the most recently used PDF is the target.
- `pdf_replace_text` — Find & replace text on a page. `{"action":"pdf_replace_text","params":{"pdf_id":"…","page":0,"old":"foo","new":"bar"}}` — replaces every occurrence of `old` with `new` on `page` (default 0).
- `pdf_annotate` — Add an annotation (highlight, underline, strikeout, note, free-text box, or link). `{"action":"pdf_annotate","params":{"pdf_id":"…","page":0,"kind":"highlight","rect":[72,700,200,24],"text":"","color":[1.0,0.85,0.2]}}` — `rect` is `[x, y, width, height]` in PDF points (origin bottom-left; a Letter page is 612×792pt, A4 is 595×842pt). For `link` the `text` is the URL; for `note`/`free_text` it is the note/box text.
- `pdf_add_note` — Add a sticky note the easy way (no rectangle needed). `{"action":"pdf_add_note","params":{"pdf_id":"…","page":0,"text":"Double-check this","x":40,"y":700}}` — `x`/`y` are PDF points from the bottom-left (default: near the top-left of the page). Prefer this over `pdf_annotate` for simple notes.
- `pdf_watermark` — Add a diagonal text watermark. `{"action":"pdf_watermark","params":{"pdf_id":"…","text":"DRAFT","all":true}}` — omitting `pages` (or `all:true`) watermarks every page.
- `pdf_delete` — Delete a PDF. `{"action":"pdf_delete","params":{"pdf_id":"…"}}`

## Creating a professional PDF

`pdf_create` renders semantic HTML (or Markdown) through a built-in Base-14 renderer, so **structure is what drives the professional look**:

- Start with a single `<h1>` title, then break content into `<h2>`/`<h3>` sections — never one giant paragraph.
- Put tabular data in `<table>` (`<tr><th>…` header cells); use `<ul>`/`<ol><li>` for lists; `<strong>`/`<b>` and `<em>`/`<i>` for emphasis; `<blockquote>` for callouts; `<hr>` between sections; `<pre>`/`<code>` for code.
- Headings render bold and sized, the title gets a rule underneath, tables get a header rule, lists get bullets/numbers, quotes get an indent + left rule.
- CSS and inline `style` attributes are **not applied** (pdf_oxide's CSS engine mis-renders embedded fonts, so the plugin uses a reliable built-in renderer). Do not rely on colors or custom CSS — express importance with headings, bold, lists and tables instead.

## Rules

- Page indices are **0-based**.
- `pages`, `order`, `degrees`, `kind`, `rect`, `old`, `text` are required where shown; `title`, `page`, `all`, `color`, `css`, `x`, `y`, `format` are optional.
- `pdf_create` defaults to **HTML + CSS** (write HTML in `content`, optional CSS in `css`). Use `"format":"markdown"` for quick Markdown; `"format":"plain"` only when the user explicitly asks for plain text.
- Annotation `rect` is in PDF points (bottom-left origin). A full Letter page is `[0, 0, 612, 792]`; a common text line is ~`[72, 700, 300, 24]`. For a simple note use `pdf_add_note` with just `text` (and optional `x`/`y`).
- Without `pdf_id`, tools target the most recently used PDF (except `pdf_create` and `pdf_merge` which create/merge).
- To change a word/name/number use `pdf_replace_text`; to mark up a page use `pdf_annotate` (or `pdf_add_note` for a note); to change a page's orientation use `pdf_rotate`; to remove/reorder pages use `pdf_delete_pages` / `pdf_reorder`; to combine documents use `pdf_merge`.
