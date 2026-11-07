# Word — agent tools

Every tool is registered in `src/plugin.rs` (`register()`) wrapped in
`shiny_plugin_sdk::tools::bridged`, so the agent may call it either through the
structured action channel or as a bridged tool. Implementations live in
`src/tools/mod.rs`.

Shared conventions:

- Tool results are `ActionOutcome::ok(tool, data)` or `ActionOutcome::error(tool,
  message)`. Hard failures return `Err(AppError)`.
- Parameters are read with `ParamHelpers` (`param_str`, `param_u32`,
  `param_bool`, `require_str`).
- **`doc_id` is optional on most tools**: when omitted, the target is the
  document the user has open — the latest of `updated_at` and `opened_at`
  (`last_doc_id`; the window sets `opened_at` whenever it opens a document).
  If none exists the call fails with `No document yet — call doc_create first`.
- Content is **Markdown** (CommonMark + GFM tables), read by WordCraft's
  Markdown reader (`engine::markdown_to_doc`): `#`–`###` headings, paragraphs,
  `- `/`1. ` lists, `| a | b |` tables, `**bold**`, `*italic*`, `> quotes`,
  links. Two consecutive text lines become two paragraphs
  (`separate_lines`), as models often write one paragraph per line; tables,
  list items and fenced code keep their lines together.
- `doc_read` returns Markdown too, so read → write round trips keep headings,
  lists and tables.
- Every tool is scoped to `req.traveler_id`, so there is no cross-user access.

## `doc_create`

**Create a new document.**

| | |
|---|---|
| Aliases | `create_document`, `new_document` |
| `step_label` | `Creating document…` |
| Code | `DocCreate::invoke`, `src/tools/mod.rs` |
| Params | `title?: string` (alias `name`), `content?: string` |
| Returns | `{ doc_id, title }` |
| Humanised | `Created document "«title»"` |

Behaviour: when `title` is missing/blank the document is named after the
content's first Markdown heading (else its first non-empty line, cleaned and
cut to 80 characters), and only then `Untitled` (`title_from_content`). `content` is
passed through `content_to_html` and encoded with `odt::html_to_odt`. A UUID id
is generated and the row inserted. Creating with no content is allowed and
produces an empty paragraph; the embedded skill still strongly instructs the
model to fill documents in the same call.

Errors:

- `AppError::Internal` when the codec cannot build the ZIP.

## `doc_write`

**Replace the entire content of a document.**

| | |
|---|---|
| Aliases | `write_document`, `update_document` |
| `step_label` | `Writing document…` |
| Code | `DocWrite::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?: string`, `content: string` (required, non-empty) |
| Returns | `{ doc_id, title }` |
| Humanised | `Wrote to "«title»"` |

Behaviour: rejects empty/whitespace `content`; resolves `doc_id` (or the most
recent document); fetches the existing title; converts content to ODT and
updates `odt` + `updated_at`. Intended only for full rewrites — the skill tells
the model never to use it for a small change.

Errors:

- `BadRequest("content required (non-empty)")`
- `BadRequest("No document yet — call doc_create first")`
- `NotFound("Document not found")`

## `doc_edit`

**Make a targeted `old` → `new` replacement inside a document.**

| | |
|---|---|
| Aliases | `edit_document`, `modify_document`, `change_document`, `replace_text` |
| `step_label` | `Editing document…` |
| Code | `DocEdit::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?: string`, `old: string` (required, non-empty), `new?: string` |
| Returns | `{ doc_id, title }` |
| Humanised | `Edited "«title»"` |

Behaviour:

1. Reads the document and converts it to HTML.
2. HTML-escapes `old` and `new` (`wordcraft_formats::html::esc`), so text with
   `<`, `&` or `"` matches the escaped HTML and can't inject markup.
3. If the HTML contains the escaped `old`, replaces the **first** occurrence.
4. Otherwise tries a case-insensitive match (`find_ci`, byte-safe via
   `char_indices`) and splices the escaped `new` over the matched range.
5. If neither matches, returns `ActionOutcome::error` whose message includes the
   current content as Markdown, so the model can produce a correct full
   rewrite. Nothing is written in that case.
6. On success re-encodes and updates `odt` + `updated_at`.

Errors:

- `BadRequest("old required — the text to change")`
- `BadRequest("No document yet — call doc_create first")`
- `NotFound("Document not found")`
- **Outcome error** (not `Err`) when the text is not present: `Text "«old»" not
  found in the document. Current content: «preview»`

## `doc_append`

**Append content, keeping what is already there.**

| | |
|---|---|
| Aliases | `append_text`, `add_to_document` |
| `step_label` | `Appending to document…` |
| Code | `DocAppend::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?: string`, `content: string` (required, non-empty) |
| Returns | `{ doc_id, title }` |
| Humanised | `Appended to "«title»"` |

Behaviour: reads the existing HTML, appends `"\n" + content_to_html(content)`,
re-encodes the whole document. Appending at the HTML level means existing
formatting/fonts are preserved.

Errors: same shape as `doc_write` (`content required`, no document, not found).

## `doc_read`

**Read a document as Markdown.**

| | |
|---|---|
| Aliases | `read_document`, `open_document`, `doc_open` |
| `step_label` | `Reading document…` |
| Code | `DocRead::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?: string` |
| Returns | `{ doc_id, title, content, format: "markdown", hint }` |
| Humanised | `Read "«title»"` |

Behaviour: `content` is `engine::odt_to_markdown` — headings, lists, tables
and emphasis as Markdown. A `hint` string reminds the model to prefer
`doc_edit`/`doc_append` and use `doc_write` only for full rewrites. `doc_open`
is the alias the embedded skill writes about; it behaves exactly like
`doc_read` and does not itself raise the window (the window reacts to the
`agent:actions` event — see [window.md](window.md)).

Errors: `BadRequest("No document yet — call doc_create first")`,
`NotFound("Document not found")`, or `Internal` on a corrupt ODT.

## `doc_list`

**List the user's documents.**

| | |
|---|---|
| Aliases | `list_documents`, `documents` |
| `step_label` | `Listing documents…` |
| Code | `DocList::invoke`, `src/tools/mod.rs` |
| Params | none (`{}`) |
| Returns | `{ documents: [{ doc_id, title, updated_at }], count }` |
| Humanised | `Found «n» documents` |

Behaviour: `SELECT id,title,updated_at … ORDER BY updated_at DESC LIMIT 100`.
Used for the discovery flow: list first to get a `doc_id`, then open/read/edit.

Errors: only DB errors surface as `Err(AppError::from(sqlx::Error))`.

## `doc_delete`

**Delete a document.**

| | |
|---|---|
| Aliases | `delete_document`, `remove_document` |
| `step_label` | `Deleting document…` |
| Code | `DocDelete::invoke`, `src/tools/mod.rs` |
| Params | `doc_id: string` (**required** — no most-recent fallback) |
| Returns | `{ doc_id }` |
| Humanised | `Document deleted` |

Behaviour: `require_str("doc_id")` then a scoped `DELETE`. If zero rows are
affected it returns an **outcome error** `Document not found` rather than
`Err`.

Errors:

- Missing required param → parameter error from `require_str`.
- Outcome error `Document not found`.

## `doc_design`

**Set how a document is composed on export.**

| | |
|---|---|
| Aliases | `doc_layout`, `format_document`, `style_document` |
| `step_label` | `Designing document…` |
| Code | `DocDesign::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?`, `template?: plain\|classic\|modern\|minimal`, `cover?`, `toc?`, `header?`, `page_numbers?` (bools), `paper?: a4\|letter`, `subtitle?`, `author?`, `date?`, `lang?`, `accent?: #RRGGBB` |
| Returns | `{ doc_id, title, design }` |
| Humanised | `Designed "«title»"` |

Behaviour: reads the stored `design` JSON and changes only the keys passed.
Choosing a non-plain `template` also turns on `header` and `page_numbers`
(`Design::with_template_defaults`) unless the call passes them. Invalid values
(unknown template, bad colour, text over 200 characters) return an outcome
error naming the allowed values. The content is not touched. See
[architecture.md](architecture.md#composition) for what each option draws.

## `doc_export`

**Save a document as a file.**

| | |
|---|---|
| Aliases | `export_document`, `save_document_as`, `doc_save_as` |
| `step_label` | `Exporting document…` |
| Code | `DocExport::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?`, `format?: docx\|pdf\|odt\|md\|html` (default `docx`) |
| Returns | `{ doc_id, title, format, filename, url }` |
| Humanised | `Exported "«filename»"` |

Behaviour: validates the format and the document and returns the export
route's `url` (`/api/documents/:id/export?format=…`). The Word window reacts to
the `doc_export` action by fetching that URL and saving the file to
`~/Documents` (`saveOrDownload`), so the file lands where the user is and the
plugin needs no filesystem access of its own.

## `doc_insert_image`

**Put a picture from the user's files into a document.**

| | |
|---|---|
| Aliases | `insert_image`, `doc_add_image`, `add_picture` |
| `step_label` | `Adding picture…` |
| Code | `DocInsertImage::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?`, `path: string` (alias `file`), `after?: string`, `caption?: string`, `width?: number` (10–100, % of the text width) |
| Returns | `{ doc_id, title, note? }` |
| Humanised | `Added a picture to "«title»"` |

Behaviour: resolves `path` against the user's files home (the OS home in
Linux-user mode, else `~/.shiny/home/<id>`; `~/` and absolute paths inside
the home are accepted) and refuses anything that resolves outside it, a
folder, or a file over 25 MB (`home_file`). `engine::picture_html` keeps a
small PNG/JPEG as is and scales/re-encodes anything else (longest side
1600 px; JPEG, or PNG with transparency); the picture is centred and sized to
`width` % of 600 px, never wider than itself. With `caption`, an italic
centred caption follows. `engine::insert_after_block` places it after the
heading or paragraph containing `after` (case-insensitive; past the whole
list or table when the text is inside one), else at the end with a `note`.
This is how the agent illustrates documents: `generate_image` returns a
home-relative `path`, which goes straight into `doc_insert_image`.

Errors: `BadRequest("path required …")`; outcome errors for a missing or
outside file, a file that is not a picture, or one too large.

## Tool-to-route correspondence

The tools and the REST routes share the same storage but are separate surfaces.
Routes use the HTML view; tools use the plain-text/HTML view.

| Tool | Closest route | Difference |
|---|---|---|
| `doc_create` | `POST /api/documents` | Tool takes Markdown `content`; route takes `html` |
| `doc_write` | `PUT /api/documents/:id` | Tool always replaces content; route merges title if omitted |
| `doc_edit` / `doc_append` | — | Tool-only (HTML-level edits) |
| `doc_read` | `GET /api/documents/:id` | Tool returns Markdown; route returns HTML + `design` |
| `doc_list` | `GET /api/documents` | Same fields, different key (`doc_id` vs `id`) |
| `doc_delete` | `DELETE /api/documents/:id` | Both delete |
| `doc_design` | `PUT /api/documents/:id/design` | Tool merges keys; route replaces the design |
| `doc_export` | `GET /api/documents/:id/export?format=` | Tool returns the URL; the window downloads it |
| — | `POST /api/documents/import` | Route-only (file upload) |

## More

- [routes.md](routes.md) · [architecture.md](architecture.md) ·
  [window.md](window.md) · [README.md](README.md)
