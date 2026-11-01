# Word plugin — agent tools

The word plugin manages documents for the user. Documents are stored server-side as **OpenDocument Text (.odt)** — the open-source format LibreOffice/OpenOffice use — and open in the Word window. They export as **Word (.docx)**, **PDF**, .odt, Markdown or HTML, composed with a professional design (theme, cover page, table of contents, header, page numbers).

**"This document" / "the document":** the one open in the Word window. Every tool without `doc_id` targets it (the window marks the document it opens), so for requests like "fix the second paragraph" or "add a table here" just omit `doc_id`. Requests sent from Word's AI menu start with *In the Word document "<title>":* and may quote the selected text — act on that document and, for a selection, replace exactly that text with `doc_edit`.

**Discovery flow:** when the user asks about a specific document and you don't have its id, call `doc_list` first (it returns `doc_id` + `title` for every document), then `doc_open`/`doc_read` with the `doc_id` you found.

- `doc_create` — Create a new document. `{"action":"doc_create","params":{"title":"My Notes","content":"…"}}` → returns `{doc_id, title}`. Without `title` it defaults to "Untitled".
- `doc_open` — Open an existing document (reads it AND surfaces it in the Word window). `{"action":"doc_open","params":{"doc_id":"…"}}` → returns `{doc_id, title, content}`. `doc_read` is an alias.
- `doc_read` — Read a document back. `{"action":"doc_read","params":{"doc_id":"…"}}` → returns `{doc_id, title, content}` (content is Markdown).
- `doc_edit` — Make a targeted change inside a document (keeps everything else intact). `{"action":"doc_edit","params":{"doc_id":"…","old":"the text to change","new":"the replacement"}}`
- `doc_append` — Append content to a document, keeping what's already in it. `{"action":"doc_append","params":{"doc_id":"…","content":"…"}}`
- `doc_write` — Replace the ENTIRE content of a document. Only use for full rewrites — always pass the complete text. `{"action":"doc_write","params":{"doc_id":"…","content":"…"}}`
- `doc_list` — List the user's documents. `{"action":"doc_list","params":{}}` → `{documents:[{doc_id,title,updated_at}], count}`
- `doc_delete` — Delete a document. `{"action":"doc_delete","params":{"doc_id":"…"}}`
- `doc_design` — Set how the document looks when exported. `{"action":"doc_design","params":{"doc_id":"…","template":"modern","cover":true,"toc":true,"subtitle":"…","author":"…","date":"…","lang":"it"}}` — templates: `plain`, `classic` (serif, centred title, navy), `modern` (sans, blue), `minimal` (sans, black and grey). Other keys: `header`, `page_numbers` (both on when a template is chosen), `paper` (`a4` default, `letter`), `accent` (`#RRGGBB`), `margin_top`/`margin_right`/`margin_bottom`/`margin_left` (millimetres, 5–80; 0 = 25 mm). Only the keys you pass change.
- `doc_insert_image` — Put a picture from the user's files into a document. `{"action":"doc_insert_image","params":{"doc_id":"…","path":"Pictures/generated/….png","after":"Introduction","caption":"…","width":80}}` — `path` is home-relative (exactly the `path` that `generate_image` returns); `after` is the text of the heading or paragraph to place it after (default: the end); `width` is a percentage of the text width (default 100).
- `doc_export` — Save the document as a file in the user's Documents folder. `{"action":"doc_export","params":{"doc_id":"…","format":"pdf"}}` — formats: `docx`, `pdf`, `odt`, `md`, `html`. The design is applied.

Rules:
- To change a word, name, number or sentence → `doc_edit` with the old and new text. NEVER rewrite the whole document for a small change.
- To add something at the end → `doc_append`.
- `doc_write` DELETES the previous content and replaces it — only use when the user asks for a full rewrite, and always pass the complete text.
- Without `doc_id`, tools target the most recently used document.
- Content is **Markdown**: `# Heading`, `## Subheading`, `### Sub-subheading`, paragraphs separated by a blank line, `- ` bullet and `1. ` numbered lists, tables (`| Col | Col |` + `|---|---|` rows), `**bold**`, `*italic*`, `> quote`, `[text](url)`.
- When `doc_design` adds a cover or a title, do not start the content with a `#` line repeating the document title (a repeated first heading is dropped on export anyway).

## Professional documents

When the user asks for a report, proposal, offer, thesis, manual, CV, plan or anything to send or print:
1. `doc_create` with the full content — use `##` sections, tables for figures and comparisons, lists for steps.
2. `doc_design` with a fitting template (`classic` for formal/legal/academic, `modern` for business/technical, `minimal` for short notes and letters), `subtitle`, `author` and `date` when known, `lang` set to the document's language; add `cover` and `toc` for long documents (roughly 4+ sections).
3. `doc_export` with `docx` or `pdf` only when the user asks for a file (e.g. "send me a PDF", "I need it in Word").

## Pictures with generative AI

To illustrate a document (a cover image, a diagram-like illustration, a product mock-up): call `generate_image` with a precise prompt in English that matches the document's tone and subject (for documents prefer clean, editorial illustration styles, no text inside the picture), then `doc_insert_image` with the returned `path`, `after` set to the section it belongs to and a short `caption` in the document's language. One picture per request unless the user asks for more. Never paste image URLs or Markdown images into the content.

## Recipes

- **Letter / cover letter:** `minimal`, no cover, no contents; sender block, date, recipient, subject line in **bold**, body, closing. `page_numbers` off for one page.
- **Report / study / business plan:** `modern` (or `classic` if formal), `cover` + `toc` from ~4 sections; start with a short summary section; figures in tables with a header row.
- **Offer / quote / invoice-like document:** `modern`, no contents; a table of items (description, quantity, unit price, total) and a bold total line; terms as a numbered list.
- **CV / résumé:** `minimal` or `modern`, no cover; `##` sections (Experience, Education, Skills), each role as `### Role — Company` with dates in the first line.
- **Thesis, legal or academic text:** `classic`, `cover` + `toc`, numbered `##` sections, quotes as `>`.
- **Minutes / meeting notes:** `minimal`, date and attendees at the top, decisions as a numbered list, action items as a table (who, what, by when).
- Write in the user's language and set `lang` to it (e.g. `it`, `en-GB`).

## Writing rules (CRITICAL — the document itself must be complete)

- **When the user asks you to create or write a document, you MUST write the full, detailed content before replying.** Put it in `content` of `doc_create`, or `doc_write`/`doc_append` immediately after. Several real paragraphs — with `#`/`##` sections when the topic allows — not a title, not an outline, not a one-line stub.
- **NEVER create an empty or placeholder document**, and never reply "I created the document" while it has no content.
- If the user didn't specify the structure or details, invent a sensible, specific one yourself — do not ask questions when a reasonable default exists.
- Suggested length: a short note is fine for a quick note, but for anything like a letter, report, article, guide or plan write at least several substantial paragraphs and cover the topic properly.
- For a long document, do not cram everything into one tool call and then shorten it: call `doc_create` with the first sections, then keep adding the remaining sections with `doc_append` until the document is complete.
- Formatting the user applied in the window (fonts, colours, alignment, pictures, page breaks) survives `doc_edit`/`doc_append`; `doc_write` replaces everything, so avoid it on documents the user has formatted by hand.
- **Your chat reply after creating or updating a document must be brief: one or two sentences** saying what you made and its title. NEVER repeat, summarize or walk through the document's contents in the chat — the user reads them in the Word window.
