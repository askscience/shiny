# Word plugin — agent tools

The word plugin manages documents for the user. Documents are stored server-side as **OpenDocument Text (.odt)** — the open-source format LibreOffice/OpenOffice use — and open in the Word window.

**Discovery flow:** when the user asks about a specific document and you don't have its id, call `doc_list` first (it returns `doc_id` + `title` for every document), then `doc_open`/`doc_read` with the `doc_id` you found.

- `doc_create` — Create a new document. `{"action":"doc_create","params":{"title":"My Notes","content":"…"}}` → returns `{doc_id, title}`. Without `title` it defaults to "Untitled".
- `doc_open` — Open an existing document (reads it AND surfaces it in the Word window). `{"action":"doc_open","params":{"doc_id":"…"}}` → returns `{doc_id, title, content}`. `doc_read` is an alias.
- `doc_read` — Read a document back. `{"action":"doc_read","params":{"doc_id":"…"}}` → returns `{doc_id, title, content}` (content is the plain text).
- `doc_edit` — Make a targeted change inside a document (keeps everything else intact). `{"action":"doc_edit","params":{"doc_id":"…","old":"the text to change","new":"the replacement"}}`
- `doc_append` — Append content to a document, keeping what's already in it. `{"action":"doc_append","params":{"doc_id":"…","content":"…"}}`
- `doc_write` — Replace the ENTIRE content of a document. Only use for full rewrites — always pass the complete text. `{"action":"doc_write","params":{"doc_id":"…","content":"…"}}`
- `doc_list` — List the user's documents. `{"action":"doc_list","params":{}}` → `{documents:[{doc_id,title,updated_at}], count}`
- `doc_delete` — Delete a document. `{"action":"doc_delete","params":{"doc_id":"…"}}`

Rules:
- To change a word, name, number or sentence → `doc_edit` with the old and new text. NEVER rewrite the whole document for a small change.
- To add something at the end → `doc_append`.
- `doc_write` DELETES the previous content and replaces it — only use when the user asks for a full rewrite, and always pass the complete text.
- Without `doc_id`, tools target the most recently used document.
- Content conventions: plain text, `# Heading`, `## Subheading`, `**bold**`, `*italic*`, one paragraph per line.

## Writing rules (CRITICAL — the document itself must be complete)

- **When the user asks you to create or write a document, you MUST write the full, detailed content before replying.** Put it in `content` of `doc_create`, or `doc_write`/`doc_append` immediately after. Several real paragraphs — with `#`/`##` sections when the topic allows — not a title, not an outline, not a one-line stub.
- **NEVER create an empty or placeholder document**, and never reply "I created the document" while it has no content.
- If the user didn't specify the structure or details, invent a sensible, specific one yourself — do not ask questions when a reasonable default exists.
- Suggested length: a short note is fine for a quick note, but for anything like a letter, report, article, guide or plan write at least several substantial paragraphs and cover the topic properly.
- For a long document, do not cram everything into one tool call and then shorten it: call `doc_create` with the first sections, then keep adding the remaining sections with `doc_append` until the document is complete.
- **Your chat reply after creating or updating a document must be brief: one or two sentences** saying what you made and its title. NEVER repeat, summarize or walk through the document's contents in the chat — the user reads them in the Word window.
