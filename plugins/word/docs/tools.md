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
- **`doc_id` is optional on most tools**: when omitted, the target is the user's
  most recently updated document (`last_doc_id`, ordered by `updated_at DESC`).
  If none exists the call fails with `No document yet — call doc_create first`.
- Content conventions: plain text, one paragraph per line, `#`/`##`/`###`
  headings, `**bold**`, `*italic*`.
- Every tool is scoped to `req.traveler_id`, so there is no cross-user access.

## `doc_create`

**Create a new document.**

| | |
|---|---|
| Aliases | `create_document`, `new_document` |
| `step_label` | `Creating document…` |
| Code | `DocCreate::invoke`, `src/tools/mod.rs` |
| Params | `title?: string`, `content?: string` |
| Returns | `{ doc_id, title }` |
| Humanised | `Created document "«title»"` |

Behaviour: `title` defaults to `Untitled` when missing/blank. `content` is
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
2. If `html.contains(old)`, replaces the **first** occurrence
   (`replacen(old, new, 1)`).
3. Otherwise tries a case-insensitive match (`find_ci`, byte-safe via
   `char_indices`) and splices `new` over the matched range.
4. If neither matches, returns `ActionOutcome::error` whose message includes the
   current plain text, so the model can produce a correct full rewrite. Nothing
   is written in that case.
5. On success re-encodes and updates `odt` + `updated_at`.

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

**Read a document's plain text.**

| | |
|---|---|
| Aliases | `read_document`, `open_document`, `doc_open` |
| `step_label` | `Reading document…` |
| Code | `DocRead::invoke`, `src/tools/mod.rs` |
| Params | `doc_id?: string` |
| Returns | `{ doc_id, title, content, hint }` |
| Humanised | `Read "«title»"` |

Behaviour: `content` is `odt::odt_to_plain_text` — one line per paragraph,
`- ` prefixes for list items. A `hint` string reminds the model to prefer
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

## Tool-to-route correspondence

The tools and the REST routes share the same storage but are separate surfaces.
Routes use the HTML view; tools use the plain-text/HTML view.

| Tool | Closest route | Difference |
|---|---|---|
| `doc_create` | `POST /api/documents` | Tool takes plain-text `content`; route takes `html` |
| `doc_write` | `PUT /api/documents/:id` | Tool always replaces content; route merges title if omitted |
| `doc_edit` / `doc_append` | — | Tool-only (HTML-level edits) |
| `doc_read` | `GET /api/documents/:id` | Tool returns plain text; route returns HTML |
| `doc_list` | `GET /api/documents` | Same fields, different key (`doc_id` vs `id`) |
| `doc_delete` | `DELETE /api/documents/:id` | Both delete |
| — | `POST /api/documents/import` | Route-only (`.odt` upload) |
| — | `GET /api/documents/:id/export` | Route-only (`.odt` download) |

## More

- [routes.md](routes.md) · [architecture.md](architecture.md) ·
  [window.md](window.md) · [README.md](README.md)
