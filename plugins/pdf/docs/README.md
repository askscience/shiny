# PDF plugin

A **self-contained** PDF viewer and editor for Shiny. Documents are stored
server-side as real `.pdf` bytes in the plugin-owned `pdf_documents` table and
open in the PDF window. The window paints the **real document in the browser**
with vendored **pdf.js** (canvas + selectable text layer), while the server keeps
the single operations engine that powers both the agent tools and the window:
page `rotate` / `reorder` / `delete-pages` / `merge`, content `replace_text`,
annotations (`highlight` / `underline` / `strikeout` / `squiggly` / `note` /
`free_text` / `link`) and `watermark`, plus page-to-PNG rendering.

Category: **Office**. See the [plugin docs index](../../../docs/plugins/README.md).

---

## Manifest

`plugins/pdf/plugin.toml`:

```toml
name = "pdf"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "PDF viewer & editor — render pages, extract text, edit text, add highlights/notes/links/watermarks, rotate/delete/reorder/merge pages, create PDFs"
summary = "PDF: view pages, extract text, edit & annotate (highlight, note, link, watermark), rotate/reorder/merge, create PDFs"
author = "shiny"
skills_dir = "skills"
web_dir = "web"
category = "Office"
```

| Field | Value | Notes |
|---|---|---|
| `name` | `pdf` | Install dir + tool/route prefix. |
| `version` | `0.1.0` | Surfaced by `GET /api/plugins`. |
| `api_level` | `1` | Must be ≤ `CORE_API_LEVEL`. |
| `entry_symbol` | `shiny_plugin_entry` | The exported `#[no_mangle] extern "C"` symbol. |
| `migrations_dir` | *(absent → `migrations`)* | Applied at load. |
| `skills_dir` | `skills` | `skills/pdf.md` is injected into the agent system prompt. |
| `web_dir` | `web` | `web/plugin.js` + `web/icon.svg` served to the frontend. |
| `category` | `Office` | Tray grouping hint. |

`Manifest` is also built in `src/plugin.rs` (`PdfPlugin::manifest`).

The persona fragment (`PERSONA`, `src/plugin.rs`):

> `a PDF editor; view, read and edit the user's PDF documents`

The context line the agent sees:

> `PDF: enabled — the PDF window renders and edits .pdf files stored server-side.`

---

## What it adds

- **12 agent tools** — `pdf_create`, `pdf_list`, `pdf_read`, `pdf_rotate`,
  `pdf_delete_pages`, `pdf_reorder`, `pdf_merge`, `pdf_replace_text`,
  `pdf_annotate`, `pdf_add_note`, `pdf_watermark`, `pdf_delete`.
- **19 REST routes** under `/api/pdfs` (list/create/import, metadata, bytes,
  text, runs, edit-text, page ops, annotate, watermark). See [routes.md](routes.md).
- **One PDF window** — `web/plugin.js`, a hairline top bar, a thumbnail rail and
  a scrollable canvas area that paints real pages with pdf.js and lays a
  selectable text layer over them. No plugin CSS (`pdf-*` lives in core).
- **1 table** — `pdf_documents`.
- **Two engines, on purpose** — `pdf_oxide` renders pages and backs the
  structural operations; `lopdf` handles in-place text editing, which needs raw
  content-stream access. See [architecture.md](architecture.md).

---

## Dependencies

`plugins/pdf/Cargo.toml`:

| Crate | Why |
|---|---|
| `shiny-plugin-sdk` | `Plugin`/`Tool` traits, routes, `PluginCtx`, bridged runtime, `Db`/`Value`. |
| `pdf_oxide` (`0.3`, `rendering`) | Parse, render to PNG, and the structural editor (`rotate`, `select_pages`, `merge`, annotations, `DocumentBuilder` creation). |
| `lopdf` (`0.45`) | Raw content-stream text replacement (`stream_edit.rs`). |
| `async-trait` | `#[async_trait]` on `Plugin`/`Tool` impls. |
| `serde`, `serde_json` | Tool/route request + response JSON, `TextRun`/`TextEdit`. |
| `semver` | Manifest version. |
| `uuid` (`v4`) | Document ids. |
| `axum` (`json`, `query`, `multipart`) | Route extractors and responses. |
| `tokio` (`full`) | Async runtime for the tool path. |

The **browser** dependency is vendored, not an npm package:
`web/vendor/pdfjs/` contains `pdf.min.mjs`, `pdf.worker.min.mjs`, `cmaps/` and
`standard_fonts/`. Do not drop `cmaps/` or `standard_fonts/` — CID-keyed
documents and PDFs without embedded fonts need them.

---

## Database tables & migrations

`plugins/pdf/migrations/001_init.sql` creates the only table:

```sql
CREATE TABLE IF NOT EXISTS pdf_documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES travelers(id),
    title TEXT NOT NULL DEFAULT 'Untitled',
    bytes BLOB NOT NULL,
    page_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pdf_documents_user ON pdf_documents(user_id, updated_at DESC);
```

| Column | Meaning |
|---|---|
| `id` | UUID primary key. |
| `user_id` | Owner (FK to `travelers`); every query is scoped to it. |
| `title` | Display name (trimmed/truncated to 120 chars on write). |
| `bytes` | The real `.pdf` file bytes — the single source of truth. |
| `page_count` | Cached count, refreshed on every `commit`. |
| `created_at`, `updated_at` | Timestamps; `updated_at` drives “most recent PDF” fallback and list order. |

> **Most-recent fallback.** Tools without a `pdf_id` target
> `SELECT id … ORDER BY updated_at DESC LIMIT 1` (`last_pdf_id`,
> `src/tools/mod.rs:40`). The window’s `openNewest()` does the same via the list.

---

## Source layout

```
plugins/pdf/
├── plugin.toml
├── Cargo.toml
├── migrations/001_init.sql
├── skills/pdf.md            agent-facing tool reference
├── src/
│   ├── lib.rs               module wiring
│   ├── plugin.rs            PdfPlugin, PERSONA, route_specs, register()
│   ├── routes.rs            19 REST handlers
│   ├── tools/mod.rs         12 agent tools
│   ├── ops.rs               shared pdf_oxide engine: render, create, page ops, annotate, watermark
│   └── stream_edit.rs       lopdf in-place text editing (+ withdrawn style_text)
└── web/
    ├── plugin.js            window: pdf.js viewer + inline editor + agent wiring
    └── icon.svg
```

---

## Build / install + dev notes

```bash
# Build the whole workspace (binary + every plugin cdylib).
cargo build --release --workspace

# Package just this plugin.
mkdir -p /tmp/pkg/pdf && cp plugins/pdf/plugin.toml /tmp/pkg/pdf/
cp -r plugins/pdf/migrations plugins/pdf/skills /tmp/pkg/pdf/
cp target/release/libshiny_pdf_plugin.so /tmp/pkg/pdf/
cp -r plugins/pdf/web /tmp/pkg/pdf/
(cd /tmp/pkg && zip -r pdf.zip pdf)

# Install / hot-reload on a running server.
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -F "plugin=@/tmp/pkg/pdf.zip"
```

Dev notes:

- The plugin is **cdylib + rlib**, so `cargo test -p shiny-pdf-plugin` runs
  against the library.
- `ops.rs` deliberately **avoids pdf_oxide’s HTML/CSS engine**: its
  subset-embedded fonts render overlapping in the tiny-skia rasteriser, so
  creation lays text out manually with `DocumentBuilder` + Base-14 fonts
  (Helvetica/Helvetica-Bold/Helvetica-Oblique/Courier/Times-*). For the same
  reason, markup annotations are drawn as page **content** (see
  [architecture.md](architecture.md)).
- Uploads are capped at **64 MB** (`MAX_UPLOAD`, `src/routes.rs:24`); the page
  count is validated against `MAX_PAGES = 2000` (`ops::validate`).
- `stream_edit::style_text` exists but is **withdrawn** — no route is registered
  and the toolbar has no formatting controls. Read its doc comment before
  touching it.

---

## Tools summary

| Tool | Aliases | Step label | Purpose |
|---|---|---|---|
| `pdf_create` | `create_pdf`, `new_pdf` | Creating PDF… | Render HTML/Markdown/plain into a styled PDF. |
| `pdf_list` | `list_pdfs`, `pdfs` | Listing PDFs… | List the user’s documents. |
| `pdf_read` | `read_pdf`, `open_pdf` | Reading PDF… | Extract text (one page or all). |
| `pdf_rotate` | `rotate_pdf`, `rotate_pages` | Rotating pages… | Rotate selected/all pages by a multiple of 90°. |
| `pdf_delete_pages` | `delete_pages`, `remove_pages` | Deleting pages… | Remove 0-based page indices. |
| `pdf_reorder` | `reorder_pages`, `move_pages` | Reordering pages… | The document becomes exactly `order`. |
| `pdf_merge` | `merge_pdfs`, `combine_pdfs`, `append_pdf` | Merging PDFs… | Append another PDF’s pages. |
| `pdf_replace_text` | `replace_text`, `edit_pdf_text`, `find_and_replace` | Editing PDF text… | Find & replace on a page via lopdf. |
| `pdf_annotate` | `add_annotation`, `highlight_pdf`, `add_note`, `add_pdf_note`, `annotate_pdf` | Annotating PDF… | Add highlight/underline/strikeout/squiggly/note/free_text/link. |
| `pdf_add_note` | `add_note`, `note_pdf`, `add_sticky_note`, `comment_pdf` | Adding note… | Sticky note without a rectangle. |
| `pdf_watermark` | `watermark_pdf`, `add_watermark` | Adding watermark… | Diagonal text watermark. |
| `pdf_delete` | `delete_pdf`, `remove_pdf` | Deleting PDF… | Delete a document. |

## Routes summary

| Method | Path | Handler tag | Purpose |
|---|---|---|---|
| GET | `/api/pdfs` | `pdf_list` | List documents (JSON array in `data`). |
| POST | `/api/pdfs` | `pdf_create` | Create from `text`/`content` + `format`. |
| POST | `/api/pdfs/import` | `pdf_import` | Multipart `.pdf` upload. |
| GET | `/api/pdfs/:id` | `pdf_get` | Metadata. |
| PUT | `/api/pdfs/:id` | `pdf_rename` | Rename. |
| DELETE | `/api/pdfs/:id` | `pdf_delete` | Delete. |
| GET | `/api/pdfs/:id/export` | `pdf_export` | Download (`Content-Disposition: attachment`). |
| GET | `/api/pdfs/:id/file` | `pdf_file` | Inline bytes for the viewer (`no-store`). |
| GET | `/api/pdfs/:id/pages/:page` | `pdf_render` | Server PNG at `?dpi=`. |
| GET | `/api/pdfs/:id/text/:page` | `pdf_text` | Extracted text for a page. |
| GET | `/api/pdfs/:id/runs/:page` | `pdf_text_runs` | Editable text runs with geometry/style. |
| POST | `/api/pdfs/:id/edit-text` | `pdf_edit_text` | Apply in-place edits in one transaction. |
| POST | `/api/pdfs/:id/rotate` | `pdf_rotate` | Rotate pages. |
| POST | `/api/pdfs/:id/reorder` | `pdf_reorder` | Reorder/keep pages. |
| POST | `/api/pdfs/:id/delete-pages` | `pdf_delete_pages` | Remove pages. |
| POST | `/api/pdfs/:id/merge` | `pdf_merge` | Append another PDF. |
| POST | `/api/pdfs/:id/replace-text` | `pdf_replace_text` | Find & replace. |
| POST | `/api/pdfs/:id/annotate` | `pdf_annotate` | Add an annotation. |
| POST | `/api/pdfs/:id/watermark` | `pdf_watermark` | Add a watermark. |

Full detail: [tools.md](tools.md) and [routes.md](routes.md).
