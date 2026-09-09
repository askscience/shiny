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
